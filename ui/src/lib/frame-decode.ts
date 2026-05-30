// Frame inspection helpers for the wire-frames page. Two transports:
//
//   ble  - the 20-byte Govee command frame. byte[0] is the family
//          (0x33 write / 0xAA read-notify / 0xA3 live blob / 0xE7 handshake /
//          0xEE device notify), byte[1] is the opcode, bytes[2..18] are data,
//          byte[19] is the XOR of bytes[0..18]. The daemon ships these as a
//          space-separated lower-hex string (see hex_pretty in the daemon).
//
//   iot  - JSON; the daemon sends the bare `msg` object as published, the
//          subscriber side delivers the full `{msg: {...}}` envelope. We
//          accept either by checking both.
//
// Opcode semantics (what aa12 means on this SKU, the per-byte field names)
// come from the daemon as a `FrameAnnotation` on each BLE frame, sourced from
// the same codec that decodes it. This module no longer keeps its own opcode
// table: that was a second, SKU-blind copy that mislabeled frames. What stays
// here is the transport-format decode (family byte, checksum, byte grid) used
// to render frames the daemon didn't annotate (IoT-wrapped frames, or BLE
// frames whose SKU wasn't resolved).

import type { FrameTransport, FieldRole, FrameAnnotation } from "./types";

export type ByteRole = FieldRole;

export type ByteAnnotation = {
  offset: number;
  value: number;
  role: ByteRole;
  /// Short label shown next to the byte. Optional; structural decode only
  /// names the family, opcode, padding and checksum bytes.
  label?: string;
};

export type BleDecoded = {
  /// True only when the frame is the full 20 bytes AND the stored XOR
  /// checksum matches the computed one. Truncated frames report false.
  ok: boolean;
  bytes: Uint8Array;
  /// Human-readable summary. Structural-only: the family name.
  summary: string;
  /// Short tag for the header badge. Always one line.
  tag: string;
  family: string;
  /// Computed XOR checksum vs stored byte[19]. null when the frame is shorter
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

/// Human family name for byte 0, matching the daemon's family_label so the two
/// agree. The high-level frame type the leading byte marks.
function familyName(byte: number): string {
  switch (byte) {
    case 0x33:
      return "write";
    case 0xaa:
      return "read/notify";
    case 0xa3:
      return "live blob";
    case 0xe7:
      return "handshake";
    case 0xee:
      return "device notify";
    default:
      return "family";
  }
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

/// Structural decode of a 20-byte BLE frame from a space-separated hex string.
/// Names only the format bytes (family, opcode, padding, checksum); opcode
/// semantics come from the daemon annotation, not here.
export function decodeBle(hex: string): BleDecoded {
  return decodeBleBytes(parseHexString(hex));
}

/// Same as decodeBle but works on bytes — used for base64-wrapped frames pulled
/// out of an IoT envelope, which don't pass through hex.
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

  const family = familyName(bytes[0]);
  const full = bytes.length === 20;
  const bodyEnd = full ? 19 : bytes.length;
  const checksum = full
    ? {
        stored: bytes[19],
        computed: xorChecksum(bytes, 19),
        ok: xorChecksum(bytes, 19) === bytes[19],
      }
    : null;

  // last data byte (offset >= 2, before the checksum) that carries a non-zero
  // value. zeros past it are the trailing pad; a zero before it is a field
  // value of 0, not padding, so we don't call it padding.
  let lastNonzero = -1;
  for (let i = 2; i < bodyEnd; i++) {
    if (bytes[i] !== 0) lastNonzero = i;
  }

  const annotations: ByteAnnotation[] = [];
  for (let i = 0; i < bytes.length; i++) {
    let role: ByteRole;
    let label: string | undefined;
    if (i === 0) {
      role = "family";
      label = family;
    } else if (i === 1) {
      role = "opcode";
      label = `opcode 0x${hex2(bytes[1])}`;
    } else if (full && i === 19) {
      role = "checksum";
      label = "xor checksum";
    } else if (i > lastNonzero) {
      // trailing zero run: the frame's zero padding
      role = "padding";
      label = "padding";
    } else {
      // interior byte we can't name without the codec, including an interior 0
      role = "unknown";
    }
    annotations.push({ offset: i, value: bytes[i], role, label });
  }

  const opTag = bytes.length >= 2 ? `${hex2(bytes[0])} ${hex2(bytes[1])}` : `${hex2(bytes[0])}`;
  return {
    ok: checksum?.ok ?? false,
    bytes,
    summary: family === "family" ? "unknown frame (or ciphertext)" : `${family} frame`,
    tag: `${opTag} · ${family}`,
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

/// One-line, transport-agnostic summary for the card header. For BLE it prefers
/// the daemon's per-frame annotation (SKU-correct); without one it falls back to
/// the structural family tag. For IoT it parses the envelope.
export function summarizeFrame(
  transport: FrameTransport,
  payload: string,
  annotation?: FrameAnnotation,
): { tag: string; summary: string } {
  if (transport === "ble") {
    if (annotation) {
      return { tag: annotation.summary, summary: annotation.summary };
    }
    const d = decodeBle(payload);
    return { tag: d.tag, summary: d.summary };
  }
  const d = decodeIot(payload);
  return { tag: d.cmd ?? "iot", summary: d.summary };
}

/// Coarse message kind for the frames-page filter. Derived structurally so it
/// works without the daemon annotation: BLE frames key off the family byte,
/// with the aa 00 ping/echo split out as keep-alive; IoT frames split into
/// command (carries a cmd) vs status notification. Direction is a separate
/// axis, handled by the frames-view direction filter.
export type FrameKind =
  | "keep-alive"
  | "write"
  | "read/notify"
  | "device notify"
  | "handshake"
  | "live blob"
  | "command"
  | "status"
  | "unknown";

/// Canonical display order for the kind filter. The view lists only the kinds
/// actually present in the buffer, in this order.
export const FRAME_KINDS: FrameKind[] = [
  "keep-alive",
  "write",
  "read/notify",
  "device notify",
  "handshake",
  "live blob",
  "command",
  "status",
  "unknown",
];

export function frameKind(transport: FrameTransport, payload: string): FrameKind {
  if (transport === "ble") {
    const toks = payload.trim().split(/\s+/);
    const fam = parseInt(toks[0] ?? "", 16);
    switch (fam) {
      case 0x33:
        return "write";
      case 0xa3:
        return "live blob";
      case 0xe7:
        return "handshake";
      case 0xee:
        return "device notify";
      case 0xaa:
        return parseInt(toks[1] ?? "", 16) === 0x00 ? "keep-alive" : "read/notify";
      default:
        return "unknown";
    }
  }
  const d = decodeIot(payload);
  if (d.cmd) return "command";
  if (d.sku && d.device) return "status";
  return "unknown";
}
