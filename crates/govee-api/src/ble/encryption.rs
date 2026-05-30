//! BLE session encryption, a reimplementation of the Govee app's
//! `com.govee.encryp.ble` layer (app v7.4.40). Devices speak one of two session
//! schemes negotiated by a BgcInfo read:
//!
//! - V1 ([`EncryptionManager`]): a 16-byte session key the device hands us
//!   (AES-ECB-wrapped under a static app key), then steady-state AES-ECB with an
//!   RC4-style keystream over the partial trailing block.
//! - V2 ([`EncryptionManagerV2`]): the app sends an 8-byte random IV base, the
//!   device replies with its own 8-byte IV base plus 11 bytes of device info; the
//!   data key is derived from that info, then steady-state AES-GCM with a 12-byte
//!   IV of `ivBase(8) || counter(4 BE)`.
//!
//! The three static app keys are not secrets we hold; they are obfuscated string
//! resources shipped in the app's `strings.xml`, recovered the same way the app
//! does at runtime (mirrors `com.govee.encryp.LibTools` + `AESUtils.decode`). This
//! carries the app's own public resources, not extracted key material. Byte-level
//! protocol notes: research/api-map/06-ble-encryption.md.
//!
//! Self-consistency (round-trip) and key-derivation tests cover the crypto here;
//! wire-correctness against real hardware is validated through the transport layer.

use aes::Aes256;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes_gcm::AesGcm;
use aes_gcm::aead::consts::{U12, U16};
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::aes::Aes128;
use once_cell::sync::Lazy;

/// GCM start byte for every frame.
const FRAME_HEAD: u8 = 0xE7;
const V1_OP_REQUEST: u8 = 0x01;
const V1_OP_CONFIRM: u8 = 0x02;
const V2_OP_REQUEST_SINGLE: u8 = 0x11;

/// Tag length the app prefers (96-bit). The frame carries this length byte so the
/// device knows how to verify; on decrypt we try this first then 128-bit, matching
/// AesGcmUtils.a()'s retry.
const TAG_LEN: u8 = 12;

type Gcm96 = AesGcm<Aes128, U12, U12>;
type Gcm128 = AesGcm<Aes128, U12, U16>;

// --- static app keys (LibTools) ---

/// One of the app's three obfuscated key resources: a hex-encoded ciphertext blob
/// plus the passphrase it's AES-ECB-encrypted under. Both are verbatim
/// `strings.xml` values. `derive` reproduces LibTools: ECB-PKCS5-decrypt the blob
/// under the passphrase to get a hex string, then hex-decode that to the 16-byte
/// AES key.
struct KeyResource {
    blob_hex: &'static str,
    passphrase: &'static str,
}

// strings.xml resource pairings (the x/y suffixes are crossed on purpose, see
// LibTools.a()/b()).
const KEY_COMMUNICATION: KeyResource = KeyResource {
    // app_communication / app_session
    blob_hex: "B8D8F6B2C294122FF9EA53918D398FE976C059D9C923D59E2489172089C9E158DDE974848DFA7115CAA351C6486B917D",
    passphrase: "chiygnveeihhmme_govee_sessioniyz",
};
const KEY_COMMUNICATION_X: KeyResource = KeyResource {
    // app_y_com / app_x_name
    blob_hex: "45EDFDC7FDCF3DB195884A04AD64E8E23667C865CD2586700E99AF824D1A763DBDA87480CB44EDB214EFF0EC7509DE1B",
    passphrase: "xhiygnvetihhxme_govee_nessioniyz",
};
const KEY_COMMUNICATION_Y: KeyResource = KeyResource {
    // app_x_com / app_y_name
    blob_hex: "F1DF5006B5DF5737CB9E666FCF0E046058BA5BEABCB015C2DE9D56ABFC6F90972D9C9F6E481908EE64AC76F5A215B51A",
    passphrase: "yhiygnveeihhyme_govee_aessioniyz",
};

impl KeyResource {
    fn derive(&self) -> [u8; 16] {
        let ct = hex_decode(self.blob_hex).expect("app key blob is valid hex");
        let key: [u8; 32] = self
            .passphrase
            .as_bytes()
            .try_into()
            .expect("passphrase is 32 bytes (AES-256)");
        let cipher = Aes256::new(GenericArray::from_slice(&key));
        let mut buf = ct;
        for chunk in buf.chunks_exact_mut(16) {
            cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
        }
        let hex_str = pkcs5_unpad(&buf);
        let derived = hex_decode(std::str::from_utf8(hex_str).expect("derived key is ascii hex"))
            .expect("derived key string is valid hex");
        derived.try_into().expect("derived AES key is 16 bytes")
    }
}

struct AppKeys {
    communication: [u8; 16],
    x: [u8; 16],
    y: [u8; 16],
}

static APP_KEYS: Lazy<AppKeys> = Lazy::new(|| AppKeys {
    communication: KEY_COMMUNICATION.derive(),
    x: KEY_COMMUNICATION_X.derive(),
    y: KEY_COMMUNICATION_Y.derive(),
});

// --- small helpers ---

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Strip PKCS5/PKCS7 padding. The decrypted blob's last byte is the pad length.
fn pkcs5_unpad(b: &[u8]) -> &[u8] {
    let n = *b.last().unwrap_or(&0) as usize;
    if (1..=16).contains(&n) && n <= b.len() && b[b.len() - n..].iter().all(|&x| x as usize == n) {
        &b[..b.len() - n]
    } else {
        b
    }
}

fn random_bytes(buf: &mut [u8]) {
    getrandom::getrandom(buf).expect("system RNG");
}

/// A fresh 8-byte outbound IV base for a V2 handshake (the app uses the first 8
/// bytes of a random AesGcmUtils.f()). Pass the same value to [`v2_build_request`]
/// and [`v2_session_from_reply`].
pub fn random_iv_send() -> [u8; 8] {
    let mut iv = [0u8; 8];
    random_bytes(&mut iv);
    iv
}

/// XOR of the first `len` bytes, the trailing checksum byte on V1 frames
/// (Controller4Aes.c).
fn xor_checksum(data: &[u8], len: usize) -> u8 {
    data[..len].iter().fold(0u8, |a, b| a ^ b)
}

// --- Safe: AES-ECB blocks + RC4-style tail (V1 cipher, V2 devKey derivation) ---

fn ecb_encrypt_block(block: &mut [u8; 16], key: &[u8; 16]) {
    Aes128::new(GenericArray::from_slice(key)).encrypt_block(GenericArray::from_mut_slice(block));
}

fn ecb_decrypt_block(block: &mut [u8; 16], key: &[u8; 16]) {
    Aes128::new(GenericArray::from_slice(key)).decrypt_block(GenericArray::from_mut_slice(block));
}

/// RC4 keystream XOR (Safe.f/g). Symmetric: encrypt == decrypt. Used only for the
/// trailing sub-16-byte block of a Safe buffer.
fn rc4(data: &[u8], key: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + s[i] as usize + key[i % key.len()] as usize) & 0xff;
        s.swap(i, j);
    }
    let mut i = 0usize;
    let mut j = 0usize;
    let mut out = Vec::with_capacity(data.len());
    for &b in data {
        i = (i + 1) & 0xff;
        j = (j + s[i] as usize) & 0xff;
        s.swap(i, j);
        let k = s[(s[i] as usize + s[j] as usize) & 0xff];
        out.push(k ^ b);
    }
    out
}

/// Safe.d: ECB-encrypt each full 16-byte block, RC4 the trailing partial block.
fn safe_encrypt(data: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut chunks = data.chunks_exact(16);
    for chunk in &mut chunks {
        let mut block: [u8; 16] = chunk.try_into().unwrap();
        ecb_encrypt_block(&mut block, key);
        out.extend_from_slice(&block);
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        out.extend_from_slice(&rc4(tail, key));
    }
    out
}

/// Safe.b: ECB-decrypt each full block, RC4 the trailing partial block.
fn safe_decrypt(data: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut chunks = data.chunks_exact(16);
    for chunk in &mut chunks {
        let mut block: [u8; 16] = chunk.try_into().unwrap();
        ecb_decrypt_block(&mut block, key);
        out.extend_from_slice(&block);
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        out.extend_from_slice(&rc4(tail, key));
    }
    out
}

// --- AES-GCM (V2) ---

/// Encrypt with a 12-byte tag (the app's preferred 96-bit). Returns ct||tag.
fn gcm_encrypt(plaintext: &[u8], iv: &[u8; 12], key: &[u8; 16], aad: &[u8]) -> Vec<u8> {
    let cipher = Gcm96::new(GenericArray::from_slice(key));
    cipher
        .encrypt(
            GenericArray::from_slice(iv),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("gcm encrypt")
}

/// Decrypt ct||tag, trying a 12-byte tag first then 16, matching AesGcmUtils.a().
fn gcm_decrypt(ct_tag: &[u8], iv: &[u8; 12], key: &[u8; 16], aad: &[u8]) -> Option<Vec<u8>> {
    let nonce = GenericArray::from_slice(iv);
    if let Ok(pt) =
        Gcm96::new(GenericArray::from_slice(key)).decrypt(nonce, Payload { msg: ct_tag, aad })
    {
        return Some(pt);
    }
    Gcm128::new(GenericArray::from_slice(key))
        .decrypt(nonce, Payload { msg: ct_tag, aad })
        .ok()
}

// --- version negotiation (BgcInfo) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
}

/// Parse the BgcInfo characteristic read (BgcInfoReader.d): byte[0] is the encrypt
/// version, 1 -> V1, 2 -> V2.
pub fn negotiate_version(bgc_info: &[u8]) -> Option<Version> {
    match bgc_info.first()? {
        1 => Some(Version::V1),
        2 => Some(Version::V2),
        _ => None,
    }
}

// --- V1 handshake (Controller4Aes) ---

/// Build a V1 20-byte plaintext frame: head, opcode, optional payload, random
/// padding to byte 18, XOR checksum at byte 19 (Controller4Aes.a).
fn v1_plain_frame(op: u8, payload: &[u8]) -> [u8; 20] {
    let mut f = [0u8; 20];
    f[0] = FRAME_HEAD;
    f[1] = op;
    let end = 2 + payload.len();
    f[2..end].copy_from_slice(payload);
    let mut pad = vec![0u8; 19 - end];
    random_bytes(&mut pad);
    f[end..19].copy_from_slice(&pad);
    f[19] = xor_checksum(&f, 19);
    f
}

/// V1 requestSessionKey frame (E7 01), ECB-wrapped under KEY_COMMUNICATION.
pub fn v1_build_request() -> Vec<u8> {
    safe_encrypt(&v1_plain_frame(V1_OP_REQUEST, &[]), &APP_KEYS.communication)
}

/// Parse a V1 key reply: decrypt under KEY_COMMUNICATION, require E7 01, return the
/// 16-byte session key from bytes [2..18] (Controller4Aes.g).
pub fn v1_parse_key_reply(cipher: &[u8]) -> Option<[u8; 16]> {
    let plain = safe_decrypt(cipher, &APP_KEYS.communication);
    if plain.len() >= 18 && plain[0] == FRAME_HEAD && plain[1] == V1_OP_REQUEST {
        Some(plain[2..18].try_into().unwrap())
    } else {
        None
    }
}

/// V1 confirmSessionKey frame (E7 02), ECB-wrapped under KEY_COMMUNICATION.
pub fn v1_build_confirm() -> Vec<u8> {
    safe_encrypt(&v1_plain_frame(V1_OP_CONFIRM, &[]), &APP_KEYS.communication)
}

/// Validate a V1 confirm ACK: decrypt, require E7 02 (Controller4Aes.h).
pub fn v1_is_confirm_ack(cipher: &[u8]) -> bool {
    let plain = safe_decrypt(cipher, &APP_KEYS.communication);
    plain.len() >= 2 && plain[0] == FRAME_HEAD && plain[1] == V1_OP_CONFIRM
}

// --- V2 handshake (Controller4AesGcm / EncryptionManagerV2) ---

/// Build the V2 single-frame (E7 11) requestSessionKey, Controller4AesGcm.d.
/// `iv_send` is the app's freshly generated 8-byte IV base (becomes the outbound
/// steady-state IV base). The 8 bytes are GCM-encrypted under KEY_COMMUNICATION_X
/// with the header+IV+taglen as AAD. Frame layout: [E7,11,0] || IV(12) || taglen ||
/// ct||tag.
///
/// Only the single-frame path is built; the device uses it whenever the link MTU
/// fits the frame (~46 bytes), which it does on the BlueZ links this runs over.
/// The low-MTU split path (E7 19 request, E7 1A data, multi-packet reassembly) is
/// documented in research/api-map/06-ble-encryption.md and not implemented.
pub fn v2_build_request(iv_send: &[u8; 8]) -> Vec<u8> {
    let mut gcm_iv = [0u8; 12];
    random_bytes(&mut gcm_iv);
    let header = [FRAME_HEAD, V2_OP_REQUEST_SINGLE, 0u8];
    let mut aad = [0u8; 16];
    aad[..3].copy_from_slice(&header);
    aad[3..15].copy_from_slice(&gcm_iv);
    aad[15] = TAG_LEN;
    let ct_tag = gcm_encrypt(iv_send, &gcm_iv, &APP_KEYS.x, &aad);
    let mut frame = Vec::with_capacity(16 + ct_tag.len());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(&gcm_iv);
    frame.push(TAG_LEN);
    frame.extend_from_slice(&ct_tag);
    frame
}

/// Parse a single-frame V2 key reply (Controller4AesGcm.h): status byte [2] must be
/// 0. The reply is [E7,11,status] || IV(12) || ct||tag; AesGcmUtils.a strips the
/// leading IV, so the IV is bytes [3..15], the ct||tag is [15..], and the AAD is
/// the head+IV bytes [0..15]. Returns the 19-byte payload.
pub fn v2_parse_single_reply(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.len() < 15 || frame[0] != FRAME_HEAD || frame[2] != 0 {
        return None;
    }
    let iv = bytes12(&frame[3..15])?;
    gcm_decrypt(&frame[15..], &iv, &APP_KEYS.x, &frame[..15])
}

/// From a decrypted 19-byte V2 key payload, build the [`Session`] (EncryptionManagerV2
/// v()): reply[0..8] is the device's inbound IV base, reply[8..19] (11 bytes) padded
/// to 16 then ECB-encrypted under KEY_Y is the data key.
pub fn v2_session_from_reply(iv_send: [u8; 8], payload: &[u8]) -> Option<Session> {
    if payload.len() != 19 {
        return None;
    }
    let iv_recv: [u8; 8] = payload[0..8].try_into().unwrap();
    let mut key16 = [0u8; 16];
    key16[..11].copy_from_slice(&payload[8..19]);
    let mut dev_key = key16;
    ecb_encrypt_block(&mut dev_key, &APP_KEYS.y);
    Some(Session::V2 {
        iv_send,
        iv_recv,
        dev_key,
        send_counter: 1,
    })
}

fn bytes12(s: &[u8]) -> Option<[u8; 12]> {
    s.get(..12)?.try_into().ok()
}

/// 4-byte big-endian frame counter (BytesUtils.d).
fn counter_be(n: u32) -> [u8; 4] {
    n.to_be_bytes()
}

// --- established session ---

/// An established BLE session. `encrypt_command` turns a 20-byte device command
/// frame into the bytes to write; `decrypt_notification` reverses an inbound frame.
pub enum Session {
    /// No session encryption: command frames are written to the data
    /// characteristic verbatim. Fallback for older unencrypted devices, reached
    /// only when a device exposes no BgcInfo characteristic and the V1 handshake
    /// also fails (service/ble.rs connect_and_handshake). Encrypted devices like
    /// the H6093 negotiate V1 and do not use this path.
    Plaintext,
    V1 {
        session_key: [u8; 16],
    },
    V2 {
        /// outbound (app->device) IV base
        iv_send: [u8; 8],
        /// inbound (device->app) IV base
        iv_recv: [u8; 8],
        dev_key: [u8; 16],
        send_counter: u32,
    },
}

impl Session {
    pub fn v1(session_key: [u8; 16]) -> Self {
        Session::V1 { session_key }
    }

    /// Encrypt an outbound command frame into the single large-MTU wire form.
    /// V1: Safe-encrypt under the session key. V2: GCM with IV = iv_send||counter,
    /// output counter(4) || ct||tag (the random GCM IV is reconstructable from
    /// base+counter so it is not transmitted).
    pub fn encrypt_command(&mut self, frame: &[u8]) -> Vec<u8> {
        match self {
            Session::Plaintext => frame.to_vec(),
            Session::V1 { session_key } => safe_encrypt(frame, session_key),
            Session::V2 {
                iv_send,
                dev_key,
                send_counter,
                ..
            } => {
                let counter = counter_be(*send_counter);
                let mut iv = [0u8; 12];
                iv[..8].copy_from_slice(iv_send);
                iv[8..].copy_from_slice(&counter);
                let ct_tag = gcm_encrypt(frame, &iv, dev_key, &counter);
                *send_counter += 1;
                let mut out = Vec::with_capacity(4 + ct_tag.len());
                out.extend_from_slice(&counter);
                out.extend_from_slice(&ct_tag);
                out
            }
        }
    }

    /// Decrypt an inbound notification (single large-MTU form). V2 reads the 4-byte
    /// counter prefix, rebuilds IV = iv_recv||counter, AAD = counter.
    pub fn decrypt_notification(&self, data: &[u8]) -> Option<Vec<u8>> {
        match self {
            Session::Plaintext => Some(data.to_vec()),
            Session::V1 { session_key } => Some(safe_decrypt(data, session_key)),
            Session::V2 {
                iv_recv, dev_key, ..
            } => {
                if data.len() < 4 {
                    return None;
                }
                let counter = &data[..4];
                let mut iv = [0u8; 12];
                iv[..8].copy_from_slice(iv_recv);
                iv[8..].copy_from_slice(counter);
                gcm_decrypt(&data[4..], &iv, dev_key, counter)
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn key_derivation_matches_app() {
        // verified end-to-end against the app's strings.xml + LibTools derivation.
        let k = &*APP_KEYS;
        assert_eq!(
            k.communication,
            hex_decode("4d616b696e674c696665536d61727465")
                .unwrap()
                .as_slice(),
            "V1 KEY_COMMUNICATION (ascii MakingLifeSmarte)"
        );
        assert_eq!(
            k.x,
            hex_decode("fc03783c7c42cb83e202a1643648aff6")
                .unwrap()
                .as_slice(),
            "V2 KEY_COMMUNICATION_X"
        );
        assert_eq!(
            k.y,
            hex_decode("ae028b630bae6ecc4bff1b249e22f955")
                .unwrap()
                .as_slice(),
            "V2 KEY_COMMUNICATION_Y"
        );
    }

    #[test]
    fn pkcs5_strips_valid_padding() {
        assert_eq!(pkcs5_unpad(&[1, 2, 3, 1]), &[1, 2, 3]);
        assert_eq!(pkcs5_unpad(&[9, 9, 3, 3, 3]), &[9, 9]);
        let mut padded = vec![0xab; 16];
        padded.extend(std::iter::repeat_n(16u8, 16));
        assert_eq!(pkcs5_unpad(&padded).len(), 16);
        assert_eq!(pkcs5_unpad(&[1, 2, 13]), &[1, 2, 13]);
        assert_eq!(pkcs5_unpad(&[1, 2, 3, 3]).len(), 4);
    }

    #[test]
    fn rc4_is_symmetric() {
        let key = [7u8; 16];
        let msg = b"four";
        let ct = rc4(msg, &key);
        assert_ne!(&ct[..], &msg[..]);
        assert_eq!(rc4(&ct, &key), msg);
    }

    #[test]
    fn safe_round_trips_with_tail() {
        let key = [0x42u8; 16];
        // 20 bytes = one ECB block + a 4-byte RC4 tail, the V1 frame size.
        let msg: Vec<u8> = (0..20).collect();
        let ct = safe_encrypt(&msg, &key);
        assert_eq!(ct.len(), 20);
        assert_eq!(safe_decrypt(&ct, &key), msg);
    }

    #[test]
    fn gcm_round_trips_96_bit_tag() {
        let key = [3u8; 16];
        let iv = [9u8; 12];
        let aad = [1u8, 2, 3, 4];
        let msg = b"hello govee";
        let ct = gcm_encrypt(msg, &iv, &key, &aad);
        // ct = ciphertext(len) + 12-byte tag
        assert_eq!(ct.len(), msg.len() + 12);
        assert_eq!(gcm_decrypt(&ct, &iv, &key, &aad).unwrap(), msg);
        // wrong aad must fail
        assert!(gcm_decrypt(&ct, &iv, &key, &[0, 0, 0, 0]).is_none());
    }

    #[test]
    fn v1_request_frame_structure() {
        let cipher = v1_build_request();
        assert_eq!(cipher.len(), 20);
        // decrypting our own request must recover a valid E7 01 plaintext frame
        let plain = safe_decrypt(&cipher, &APP_KEYS.communication);
        assert_eq!(plain[0], FRAME_HEAD);
        assert_eq!(plain[1], V1_OP_REQUEST);
        assert_eq!(plain[19], xor_checksum(&plain, 19));
    }

    #[test]
    fn v1_key_reply_parses() {
        // simulate a device reply: a 20-byte E7 01 frame carrying a known key.
        let key = [0xABu8; 16];
        let mut plain = [0u8; 20];
        plain[0] = FRAME_HEAD;
        plain[1] = V1_OP_REQUEST;
        plain[2..18].copy_from_slice(&key);
        plain[19] = xor_checksum(&plain, 19);
        let cipher = safe_encrypt(&plain, &APP_KEYS.communication);
        assert_eq!(v1_parse_key_reply(&cipher), Some(key));
    }

    #[test]
    fn v2_session_round_trips_a_command() {
        // build a session from a synthetic 19-byte reply and round-trip a frame.
        let iv_send = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let payload: Vec<u8> = (0..19).collect();
        let mut session = v2_session_from_reply(iv_send, &payload).unwrap();
        let (iv_recv, dev_key) = match &session {
            Session::V2 {
                iv_recv, dev_key, ..
            } => (*iv_recv, *dev_key),
            _ => unreachable!(),
        };
        assert_eq!(&iv_recv, &payload[0..8]);

        let frame: Vec<u8> = (0..20).collect();
        let wire = session.encrypt_command(&frame);
        // counter(4) + ct(20) + tag(12)
        assert_eq!(wire.len(), 4 + 20 + 12);
        assert_eq!(&wire[..4], &1u32.to_be_bytes());

        // a peer using iv_send as its recv base decrypts what we sent
        let peer = Session::V2 {
            iv_send: [0; 8],
            iv_recv: iv_send,
            dev_key,
            send_counter: 1,
        };
        assert_eq!(peer.decrypt_notification(&wire).unwrap(), frame);

        // counter advances
        let wire2 = session.encrypt_command(&frame);
        assert_eq!(&wire2[..4], &2u32.to_be_bytes());
    }

    #[test]
    fn v2_single_reply_parses() {
        // build a device-style reply: [E7,11,00] || IV(12) || ct||tag, the 19-byte
        // payload GCM-encrypted under KEY_X with head+IV as AAD.
        let payload: Vec<u8> = (10..29).collect();
        assert_eq!(payload.len(), 19);
        let iv = [0x5au8; 12];
        let head = [FRAME_HEAD, V2_OP_REQUEST_SINGLE, 0u8];
        let mut aad = Vec::new();
        aad.extend_from_slice(&head);
        aad.extend_from_slice(&iv);
        let ct_tag = gcm_encrypt(&payload, &iv, &APP_KEYS.x, &aad);
        let mut frame = Vec::new();
        frame.extend_from_slice(&head);
        frame.extend_from_slice(&iv);
        frame.extend_from_slice(&ct_tag);
        assert_eq!(
            v2_parse_single_reply(&frame).as_deref(),
            Some(payload.as_slice())
        );
        // non-zero status byte is rejected
        frame[2] = 1;
        assert!(v2_parse_single_reply(&frame).is_none());
    }

    #[test]
    fn plaintext_session_is_passthrough() {
        let mut s = Session::Plaintext;
        let frame: Vec<u8> = (0..20).collect();
        let wire = s.encrypt_command(&frame);
        assert_eq!(wire, frame);
        assert_eq!(
            s.decrypt_notification(&wire).as_deref(),
            Some(frame.as_slice())
        );
    }

    #[test]
    fn version_negotiation() {
        assert_eq!(negotiate_version(&[1, 0, 0]), Some(Version::V1));
        assert_eq!(negotiate_version(&[2, 0, 0]), Some(Version::V2));
        assert_eq!(negotiate_version(&[3]), None);
        assert_eq!(negotiate_version(&[]), None);
    }
}
