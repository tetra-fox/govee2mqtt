//! BLE command structs for generic lights: scene-code frames and device power.

use super::codec::{GoveeBlePacket, PacketCodec, finish};
use crate::packet;

/// Register every generic-light codec into the PacketManager's table.
pub(super) fn register(codecs: &mut Vec<PacketCodec>) {
    codecs.push(PacketCodec::new(
        &["Generic:Light"],
        SetSceneCode::encode,
        SetSceneCode::decode,
    ));
    codecs.push(packet!(
        &["Generic:Light"],
        SetDevicePower,
        SetDevicePower,
        0x33,
        0x01,
        on,
    ));
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct SetSceneCode {
    code: u16,
    scence_param: String,
}

impl SetSceneCode {
    pub fn new(code: u16, scence_param: String) -> Self {
        Self { code, scence_param }
    }

    /// For reference, see:
    /// <https://github.com/egold555/Govee-Reverse-Engineering/issues/11#issuecomment-2565692233>
    /// <https://github.com/AlgoClaw/Govee/blob/main/decoded/explanation>
    fn encode(&self) -> anyhow::Result<Vec<u8>> {
        let bytes = data_encoding::BASE64.decode(self.scence_param.as_bytes())?;

        let mut data = vec![0xa3, 0x00, 0x01, 0x00 /* line count */, 0x02];
        let mut num_lines = 0u8;
        let mut last_line_marker = 1;

        for b in bytes {
            if data.len().is_multiple_of(19) {
                num_lines += 1;

                data.push(0xa3);
                last_line_marker = data.len();

                data.push(num_lines);
            }

            data.push(b);
        }
        // The last line uses 0xff as the indicator, rather than its line number
        data[last_line_marker] = 0xff;
        // back-patch the number of lines into the packet
        data[3] = num_lines + 1;

        // Now apply padding and checksums
        let mut padded = vec![];
        for chunk in data.chunks(19) {
            let mut padded_chunk = chunk.to_vec();
            padded_chunk = finish(padded_chunk);
            padded.append(&mut padded_chunk);
        }

        // and finally encode the scene code as the final packet "line"
        let hi = (self.code >> 8) as u8;
        let lo = (self.code & 0xff) as u8;
        padded.append(&mut finish(vec![0x33, 0x05, 0x04, lo, hi]));
        Ok(padded)
    }

    fn decode(_data: &[u8]) -> anyhow::Result<GoveeBlePacket> {
        Err(super::codec::CodecUnsupported("SetSceneCode::decode is not implemented").into())
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct SetDevicePower {
    pub on: bool,
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn scene_command() {
        const FOREST_SCENCE_PARAM: &str = "AyYAAQAKAgH/GQG0CgoCyBQF//8AAP//////AP//lP8AFAGWAAAAACMAAg8FAgH/FAH7AAAB+goEBP8AtP8AR///4/8AAAAAAAAAABoAAAABAgH/BQHIFBQC7hQBAP8AAAAAAAAAAA==";
        const FOREST_SCENE_CODE: u16 = 212;

        let command = SetSceneCode::new(FOREST_SCENE_CODE, FOREST_SCENCE_PARAM.to_string());

        let padded = command.encode().unwrap();

        println!("data is:");
        let mut hex = String::new();
        for (idx, b) in padded.iter().enumerate() {
            if idx % 20 == 0 && !hex.is_empty() {
                hex.push('\n');
            } else if !hex.is_empty() {
                hex.push(' ');
            }
            hex.push_str(&format!("{b:02x}"));
        }
        println!("{hex}");

        assert_eq!(
            hex,
            "\
a3 00 01 07 02 03 26 00 01 00 0a 02 01 ff 19 01 b4 0a 0a d9
a3 01 02 c8 14 05 ff ff 00 00 ff ff ff ff ff 00 ff ff 94 12
a3 02 ff 00 14 01 96 00 00 00 00 23 00 02 0f 05 02 01 ff 0a
a3 03 14 01 fb 00 00 01 fa 0a 04 04 ff 00 b4 ff 00 47 ff b3
a3 04 ff e3 ff 00 00 00 00 00 00 00 00 1a 00 00 00 01 02 5d
a3 05 01 ff 05 01 c8 14 14 02 ee 14 01 00 ff 00 00 00 00 92
a3 ff 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 5c
33 05 04 d4 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 e6"
        );
    }
}
