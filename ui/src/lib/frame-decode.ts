// Frame inspection helpers for the wire-frames page. Two transports:
//
//   ble  - the 20-byte Govee command frame. byte[0] is the family
//          (0x33 write / 0xAA read-notify / 0xA3 live blob / 0xE7 handshake),
//          byte[1] is the subcommand, bytes[2..18] are data, byte[19] is the
//          XOR of bytes[0..18]. The daemon ships these as a space-separated
//          lower-hex string (see hex_pretty in govee-api).
//
//   iot  - JSON; the daemon sends the bare `msg` object as published, the
//          subscriber side delivers the full `{msg: {...}}` envelope. We
//          accept either by checking both.
//
// Opcode names mirror research/api-map/07-frame-reference.md. Update both
// sides when adding an opcode; the doc is the ground truth and this map is
// the UI-side index into it.

import type { FrameTransport } from "./types";

export type ByteRole = "family" | "subcommand" | "param" | "padding" | "checksum" | "unknown";

export type ByteAnnotation = {
  offset: number;
  value: number;
  role: ByteRole;
  /// Short label shown next to the byte in the inspector. Optional; param
  /// bytes without a confirmed meaning are just labelled by index.
  label?: string;
};

export type BleDecoded = {
  /// True only when the frame is the full 20 bytes AND the stored XOR
  /// checksum matches the computed one. Truncated frames report false:
  /// callers gating on this should treat "couldn't verify" as "not ok".
  ok: boolean;
  bytes: Uint8Array;
  /// Human-readable summary: family + opcode + parameters when known.
  /// Falls back to a generic family tag for unknown opcodes.
  summary: string;
  /// Short tag for the header badge. Always one line.
  tag: string;
  family: "write" | "read" | "live-blob" | "handshake" | "unknown";
  /// Computed XOR checksum vs stored byte[19]. null when frame is shorter
  /// than 20 bytes (truncated).
  checksum: { stored: number; computed: number; ok: boolean } | null;
  annotations: ByteAnnotation[];
};

export type IotDecoded = {
  /// Top-level cmd extracted from `msg.cmd` or top-level cmd. null when the
  /// payload isn't a recognized envelope.
  cmd: string | null;
  /// Govee transaction tag (`tag`) when present; the daemon sets this on
  /// outbound commands and the device echoes it on the reply.
  tag: string | number | null;
  /// SKU + device id (when the envelope is a status notification from a device).
  sku: string | null;
  device: string | null;
  /// Base64-encoded 20-byte BLE frames wrapped inside the envelope. Outgoing
  /// ptReal commands carry these in `data.command`; incoming device status
  /// notifications carry them in `op.command`. Includes which field they
  /// came from so the inspector can label the section.
  wrappedFrames: { source: "data.command" | "op.command"; b64: string }[];
  /// One-line summary for the card header.
  summary: string;
};

const BLE_OPCODES: Record<string, { label: string; family: BleDecoded["family"] }> = {
  // generic 0x33 writes
  "33 01": { label: "power", family: "write" },
  "33 04": { label: "brightness", family: "write" },
  "33 05": { label: "mode / apply", family: "write" },
  // H6093-specific
  "33 11": { label: "H6093 aurora config", family: "write" },
  "33 30": { label: "H6093 settings toggle", family: "write" },
  "33 31": { label: "H6093 timer A", family: "write" },
  "33 32": { label: "H6093 timer B", family: "write" },
  // H5082-specific (research/mitm/H5082-protocol.md)
  "33 13": { label: "H5082 set timer slot", family: "write" },
  "33 b0": { label: "H5082 set countdown", family: "write" },
  "33 b2": { label: "H5082 key probe", family: "write" },
  "33 b5": { label: "H5082 sync time", family: "write" },
  // generic reads / notifications
  "aa 01": { label: "power query", family: "read" },
  "aa 04": { label: "brightness query", family: "read" },
  "aa 05": { label: "mode query", family: "read" },
  "aa 06": { label: "firmware version", family: "read" },
  "aa 07": { label: "hardware version", family: "read" },
  "aa 11": { label: "H6093 aurora notify", family: "read" },
  "aa 12": { label: "H5082 timer count", family: "read" },
  "aa 13": { label: "H5082 timer slots", family: "read" },
  "aa 20": { label: "secondary version A", family: "read" },
  "aa 21": { label: "secondary version B", family: "read" },
  "aa b0": { label: "H5082 countdown slot", family: "read" },
  // V1 handshake
  "e7 01": { label: "V1 session handshake", family: "handshake" },
  "e7 02": { label: "V1 session confirm", family: "handshake" },
};

/// H5082 nibble-packed power val (high nibble = outlet selector mask, low
/// nibble = on-bits within that mask). bit0 = outlet 2, bit1 = outlet 1.
/// Returns null when the val doesn't look like the packed form so the caller
/// can fall back to the generic 1/0 interpretation.
function decodeH5082Power(val: number): string | null {
  const mask = (val >> 4) & 0x0f;
  const on = val & 0x0f;
  if (mask === 0 || (mask & on) !== on) return null;
  const both = mask === 0b11;
  const outlet1 = mask === 0b10;
  const outlet2 = mask === 0b01;
  const isOn = on === mask;
  if (both) return isOn ? "master on" : "master off";
  if (outlet1) return isOn ? "outlet 1 on" : "outlet 1 off";
  if (outlet2) return isOn ? "outlet 2 on" : "outlet 2 off";
  return null;
}

/// H5082 onOff bitmask reply (aa 01 <mask>): bit0 = outlet 2 on, bit1 =
/// outlet 1 on. Same bit layout as the low nibble of the power write.
function decodeH5082OnOffMask(mask: number): string {
  const o1 = (mask & 0b10) !== 0;
  const o2 = (mask & 0b01) !== 0;
  if (o1 && o2) return "both on";
  if (o1) return "outlet 1 on";
  if (o2) return "outlet 2 on";
  return "both off";
}

/// H5082 outlet wire byte (0x01 = outlet 1, 0x00 = outlet 2). Inverse of the
/// power-write nibble selector. The same encoding is used by 33 b0 / aa b0
/// (countdowns) and aa 12 / aa 13 (timer reads).
function decodeH5082Outlet(b: number): string | null {
  if (b === 0x01) return "outlet 1";
  if (b === 0x00) return "outlet 2";
  return null;
}

/// Per-byte parameter label for a known opcode. Returns undefined when
/// nothing useful can be said; the caller renders the bare value.
function paramLabel(opKey: string, bytes: Uint8Array, i: number): string | undefined {
  switch (opKey) {
    case "33 01":
      if (i === 2)
        return (
          decodeH5082Power(bytes[2]) ??
          (bytes[2] === 1 ? "on" : bytes[2] === 0 ? "off" : `val ${bytes[2]}`)
        );
      return undefined;
    case "33 04":
      if (i === 2) return `brightness ${bytes[2]}%`;
      return undefined;
    case "33 b0":
    case "aa b0":
      if (i === 2) return decodeH5082Outlet(bytes[2]) ?? undefined;
      if (i === 3)
        return bytes[3] === 0x01 ? "fire on" : bytes[3] === 0x00 ? "fire off" : undefined;
      if (i === 4) return `hh ${bytes[4]}`;
      if (i === 5) return `mm ${bytes[5]}`;
      return undefined;
    case "aa 01":
      if (i === 2) return decodeH5082OnOffMask(bytes[2]);
      return undefined;
    case "aa 12":
      if (i === 2) return decodeH5082Outlet(bytes[2]) ?? undefined;
      if (i === 3) return `count ${bytes[3]}`;
      return undefined;
    case "33 13":
      if (i === 2) return decodeH5082Outlet(bytes[2]) ?? undefined;
      if (i === 3) return `slot ${bytes[3]}`;
      return undefined;
    default:
      return undefined;
  }
}

function familyOf(byte: number): BleDecoded["family"] {
  if (byte === 0x33) return "write";
  if (byte === 0xaa) return "read";
  if (byte === 0xa3) return "live-blob";
  if (byte === 0xe7) return "handshake";
  return "unknown";
}

function hex2(n: number): string {
  return n.toString(16).padStart(2, "0");
}

export function parseHexString(hex: string): Uint8Array {
  const tokens = hex.trim().split(/\s+/).filter(Boolean);
  const out = new Uint8Array(tokens.length);
  for (let i = 0; i < tokens.length; i++) {
    const v = parseInt(tokens[i], 16);
    out[i] = Number.isNaN(v) ? 0 : v;
  }
  return out;
}

/// Decode a base64 string to bytes. Returns an empty array on malformed input
/// so callers can render "(empty)" instead of throwing into the inspector.
export function parseBase64(b64: string): Uint8Array {
  try {
    const bin = atob(b64);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
  } catch {
    return new Uint8Array(0);
  }
}

function xorChecksum(bytes: Uint8Array, upto: number): number {
  let acc = 0;
  for (let i = 0; i < upto; i++) acc ^= bytes[i];
  return acc;
}

/// Decode a single 20-byte BLE command frame from a space-separated hex
/// string (the daemon's wire format). Tolerates shorter input.
export function decodeBle(hex: string): BleDecoded {
  return decodeBleBytes(parseHexString(hex));
}

/// Same as decodeBle but works directly on bytes — used for base64-wrapped
/// frames pulled out of an IoT envelope, which don't pass through hex.
/// `checksum.ok` only makes sense when the frame is the full 20 bytes long.
export function decodeBleBytes(bytes: Uint8Array): BleDecoded {
  if (bytes.length === 0) {
    return {
      ok: false,
      bytes,
      summary: "(empty)",
      tag: "empty",
      family: "unknown",
      checksum: null,
      annotations: [],
    };
  }

  const family = familyOf(bytes[0]);
  const opKey = bytes.length >= 2 ? `${hex2(bytes[0])} ${hex2(bytes[1])}` : null;
  const known = opKey ? BLE_OPCODES[opKey] : null;

  const checksum =
    bytes.length === 20
      ? {
          stored: bytes[19],
          computed: xorChecksum(bytes, 19),
          ok: xorChecksum(bytes, 19) === bytes[19],
        }
      : null;

  const annotations: ByteAnnotation[] = [];
  for (let i = 0; i < bytes.length; i++) {
    let role: ByteRole = "param";
    let label: string | undefined;
    if (i === 0) {
      role = "family";
      label = `${family} (${hex2(bytes[0])})`;
    } else if (i === 1) {
      role = "subcommand";
      label = known?.label;
    } else if (i === 19 && bytes.length === 20) {
      role = "checksum";
      label = `XOR (${checksum?.ok ? "ok" : "BAD"})`;
    } else if (i >= 2 && i < (bytes.length === 20 ? 19 : bytes.length)) {
      // parameter bytes: pull whatever the opcode-specific decoder knows.
      if (opKey) label = paramLabel(opKey, bytes, i);
    }
    annotations.push({ offset: i, value: bytes[i], role, label });
  }

  // tag/summary for the header badge
  let tag: string;
  let summary: string;
  if (known) {
    tag = `${opKey} · ${known.label}`;
    summary = known.label;
  } else if (family === "live-blob") {
    tag = `a3 · live blob (${bytes.length}B)`;
    summary = `H6093 full-state blob (${bytes.length} bytes)`;
  } else if (family === "unknown") {
    // unrecognized first byte: could be a real unknown opcode, or the bytes
    // we see are pre-decryption ciphertext for a supportEnc device (V1/V2
    // frames have no recognizable family byte). Daemon doesn't tell us
    // which, so the summary mentions both.
    tag = opKey ? `${opKey}` : `${hex2(bytes[0])}`;
    summary = "unknown opcode (or pre-decryption ciphertext)";
  } else {
    tag = opKey ? `${opKey} · ${family}` : `${hex2(bytes[0])} · ${family}`;
    summary = `${family} opcode ${opKey ?? hex2(bytes[0])}`;
  }

  return {
    ok: checksum?.ok ?? false,
    bytes,
    summary,
    tag,
    family,
    checksum,
    annotations,
  };
}

/// Reach into an IoT envelope and pull out the bits a debugger cares about.
/// Accepts either `{ msg: {...} }` (subscriber-side) or the bare `msg` object
/// (publisher-side); checks both before giving up.
export function decodeIot(text: string): IotDecoded {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return {
      cmd: null,
      tag: null,
      sku: null,
      device: null,
      wrappedFrames: [],
      summary: "(unparseable json)",
    };
  }
  const obj = (parsed && typeof parsed === "object" ? parsed : {}) as Record<string, unknown>;
  const inner = (obj.msg && typeof obj.msg === "object" ? obj.msg : obj) as Record<string, unknown>;

  const cmd = typeof inner.cmd === "string" ? inner.cmd : null;
  const rawTag = inner.tag;
  const tag = typeof rawTag === "string" || typeof rawTag === "number" ? rawTag : null;

  const sku = typeof obj.sku === "string" ? obj.sku : null;
  const device = typeof obj.device === "string" ? obj.device : null;

  // pull `data.val` only when data is an object; firmware variants where data
  // is a scalar (or absent) just return undefined.
  const dataVal: unknown =
    inner.data && typeof inner.data === "object" && !Array.isArray(inner.data)
      ? (inner.data as Record<string, unknown>).val
      : undefined;

  const wrappedFrames: IotDecoded["wrappedFrames"] = [];
  // outgoing ptReal: base64 BLE frames live under data.command
  if (inner.data && typeof inner.data === "object" && !Array.isArray(inner.data)) {
    const command = (inner.data as Record<string, unknown>).command;
    if (Array.isArray(command)) {
      for (const v of command) {
        if (typeof v === "string") wrappedFrames.push({ source: "data.command", b64: v });
      }
    }
  }
  // incoming status notifications carry the same base64-of-20-bytes shape
  // under op.command; same wire layout, different field name.
  if (inner.op && typeof inner.op === "object" && !Array.isArray(inner.op)) {
    const command = (inner.op as Record<string, unknown>).command;
    if (Array.isArray(command)) {
      for (const v of command) {
        if (typeof v === "string") wrappedFrames.push({ source: "op.command", b64: v });
      }
    }
  }

  let summary: string;
  if (cmd) {
    const bits: string[] = [cmd];
    if (wrappedFrames.length > 0) {
      bits.push(`${wrappedFrames.length} wrapped`);
    } else if ((cmd === "turn" || cmd === "brightness") && dataVal !== undefined) {
      bits.push(`val=${JSON.stringify(dataVal)}`);
    }
    summary = bits.join(" · ");
  } else if (sku && device) {
    summary =
      wrappedFrames.length > 0
        ? `status notification · ${wrappedFrames.length} wrapped`
        : "status notification";
  } else {
    summary = "(unrecognized envelope)";
  }

  return { cmd, tag, sku, device, wrappedFrames, summary };
}

/// One-line, transport-agnostic summary for the card header. Falls back to
/// "raw" hex digits or json snippet when nothing more useful is available.
export function summarizeFrame(
  transport: FrameTransport,
  payload: string,
): { tag: string; summary: string } {
  if (transport === "ble") {
    const d = decodeBle(payload);
    return { tag: d.tag, summary: d.summary };
  }
  const d = decodeIot(payload);
  return { tag: d.cmd ?? "iot", summary: d.summary };
}
