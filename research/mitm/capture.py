"""mitmproxy addon to capture the Govee app's control traffic for one device.

Two channels carry control, depending on device type:
  - shared devices (smart plugs/switches): HTTPS REST relay to
    app2.govee.com/bff-app/v1/fx-device/iot-msgs. mitmproxy parses this natively
    over HTTP/2, so do NOT put :443 / the EC2 host in tcp_hosts -- if you do, the
    HTTP/2 stream gets treated as raw TCP and the control POST never fires the
    request hook.
  - owned devices: outbound commands publish straight to AWS IoT MQTT (:8883,
    mutual TLS). That can't be read as HTTP, so run the IoT host through tcp_hosts
    (TLS decrypt + raw TCP) with the account client cert presented upstream; the
    tcp_message hook below parses MQTT PUBLISH frames out of the byte stream.

Run with:
  mitmdump --mode wireguard -s capture.py
Config via env (all optional):
  GOVEE_DEVICE  device MAC to single out of a busy account (e.g. AA:BB:...)
  GOVEE_SKU     device SKU (e.g. H6093), matched in URLs/bodies
  GOVEE_OUT     output dir for the jsonl streams (default: this script's dir)
  GOVEE_SKIP    comma-separated host fragments to ignore (local/non-Govee hosts)
"""
import json
import os
import time

from mitmproxy import ctx

DEVICE = os.environ.get("GOVEE_DEVICE", "").upper()
SKU = os.environ.get("GOVEE_SKU", "").upper()
OUT_DIR = os.environ.get("GOVEE_OUT") or os.path.dirname(os.path.abspath(__file__))
SKIP_HOST_FRAGMENTS = tuple(
    f.strip() for f in os.environ.get("GOVEE_SKIP", "").split(",") if f.strip()
)

HTTP_OUT = os.path.join(OUT_DIR, "control.jsonl")
MQTT_OUT = os.path.join(OUT_DIR, "mqtt-pub.jsonl")

# url fragments that mark a control/scene/effect call worth full logging
CONTROL_HINTS = (
    "iot-msgs", "turn", "control", "command", "scene", "effect", "diy",
    "ptreal", "mode", "color-mode", "light-effect", "device/control",
)

# reassembly buffer per (connection, direction); MQTT frames split across segments
_buf = {}


def _skip(host: str) -> bool:
    return any(frag in host for frag in SKIP_HOST_FRAGMENTS)


def _hits_device(text: str) -> bool:
    """True if this text references the configured device. With no device set,
    everything that passed the control/skip filters is kept."""
    if not DEVICE and not SKU:
        return False
    t = text.upper()
    if DEVICE and (DEVICE in t or DEVICE.replace(":", "") in t):
        return True
    return bool(SKU and SKU in t)


def _record(path, obj):
    try:
        with open(path, "a") as f:
            f.write(json.dumps(obj) + "\n")
    except Exception as e:  # logging must never kill the capture
        ctx.log.warn(f"[capture write failed: {e}]")


def _http(kind, flow, body):
    r = flow.request
    if _skip(r.pretty_host):
        return
    url_l = r.pretty_url.lower()
    is_control = any(k in url_l for k in CONTROL_HINTS)
    hits = _hits_device(body) or _hits_device(r.pretty_url)
    if not (is_control or hits):
        return
    tag = SKU or "ctrl" if hits else "ctrl"
    status = flow.response.status_code if flow.response else None
    ctx.log.warn(f"[{kind.upper()} {tag}] {r.method} {status or ''} {r.pretty_url}")
    if body and hits:
        ctx.log.warn(f"  {body[:1500]}")
    _record(HTTP_OUT, {
        "t": time.strftime("%H:%M:%S"),
        "kind": kind,
        "method": r.method,
        "url": r.pretty_url,
        "status": status,
        "body": body[:8000],
    })


def request(flow):
    _http("req", flow, flow.request.get_text(strict=False) or "")


def response(flow):
    body = flow.response.get_text(strict=False) or "" if flow.response else ""
    _http("resp", flow, body)


def _decode_varint(data, i):
    """MQTT remaining-length varint. Returns (value, new_index) or (None, i)."""
    mult = 1
    val = 0
    while True:
        if i >= len(data):
            return None, i
        b = data[i]
        i += 1
        val += (b & 0x7F) * mult
        if (b & 0x80) == 0:
            return val, i
        mult *= 128
        if mult > 128 ** 4:
            return None, i


def _parse_mqtt(data, direction):
    """Pull PUBLISH frames out of an MQTT byte stream. Best-effort: skips
    non-PUBLISH packets, returns any trailing partial frame for reassembly."""
    i = 0
    n = len(data)
    while i < n:
        pkt_type = (data[i] >> 4) & 0x0F
        flags = data[i] & 0x0F
        rem, j = _decode_varint(data, i + 1)
        if rem is None or j + rem > n:
            return data[i:]  # incomplete frame, hand the tail back
        body = data[j:j + rem]
        if pkt_type == 3 and len(body) >= 2:  # PUBLISH
            tlen = (body[0] << 8) | body[1]
            topic = body[2:2 + tlen].decode("utf-8", "replace")
            k = 2 + tlen
            if ((flags >> 1) & 0x03) > 0:  # qos > 0 -> 2-byte packet id
                k += 2
            payload = body[k:].decode("utf-8", "replace")
            if not (DEVICE or SKU) or _hits_device(payload) or _hits_device(topic):
                ctx.log.warn(f"[MQTT {direction} PUBLISH] {topic}")
                if payload:
                    ctx.log.warn(f"  {payload[:600]}")
                _record(MQTT_OUT, {
                    "t": time.strftime("%H:%M:%S"),
                    "dir": direction,
                    "topic": topic,
                    "payload": payload[:8000],
                })
        i = j + rem
    return b""


def tcp_message(flow):
    msg = flow.messages[-1]
    direction = "c->s" if msg.from_client else "s->c"
    key = (id(flow), direction)
    data = _buf.get(key, b"") + bytes(msg.content)
    _buf[key] = _parse_mqtt(data, direction)
