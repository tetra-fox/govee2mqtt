# contributing

## reverse-engineering toolchain

most of this project's protocol knowledge (the BLE frame codecs, the IoT/cloud
control routing, the per-SKU command encoders) was reverse-engineered from the
Govee Android app and from captured device traffic. the tools for that live in a
separate nix dev shell, kept apart from the rust build env:

```sh
nix develop .#re
```

that shell gives you:

- **mitmproxy** -- capture the app's HTTPS and MQTT traffic. this is how the cloud
  control path and the per-device command payloads were mapped. see "capturing app
  traffic" below for the bring-up.
- **jadx** -- decompile the app's APK (and its feature splits) from dex to java.
  the app ships device-family code as separate "pact" splits, so decompile those
  too, not just `base.apk`.
- **apktool** -- decode resources and the merged `AndroidManifest.xml` from an APK
  (`apktool d -s` to skip smali when you only want resources/strings).
- **unzip** -- pull `.so` libs or split APKs out of an app bundle.

the rust build env is the default shell, unchanged:

```sh
nix develop
```

## capturing app traffic

the cleanest way to see what the app sends is mitmproxy's WireGuard mode: the phone
joins a WireGuard tunnel that mitmproxy serves, so all its traffic is decrypted with
no proxy settings and no root on the phone.

1. start mitmproxy in WireGuard mode:

   ```sh
   mitmdump --mode wireguard -s research/mitm/capture.py
   ```

   on first start it prints a ready-to-use WireGuard client config (a freshly
   generated keypair, the host's IP, port 51820). do NOT commit that config: the
   private key is a live secret, and mitmproxy regenerates it each run anyway.

2. put that config on the phone (paste it into the WireGuard app, or render the
   printed config as a QR code and scan it) and bring the tunnel up.

3. install mitmproxy's CA cert on the phone (`http://mitm.it` once the tunnel is up)
   so TLS interception works.

4. drive the Govee app. the `capture.py` addon writes two streams into
   `research/mitm/`: `control.jsonl` (the HTTPS REST control path) and
   `mqtt-pub.jsonl` (AWS IoT MQTT PUBLISH frames). configure it with env vars to
   single out one device: `GOVEE_DEVICE`, `GOVEE_SKU`, `GOVEE_OUT`, `GOVEE_SKIP`
   (comma-separated host fragments to ignore, e.g. your LAN / non-Govee hosts).

two channel gotchas the addon's header documents in full:

- shared devices (smart plugs) send control as an HTTPS POST to
  `app2.govee.com/.../fx-device/iot-msgs`. let mitmproxy parse it as HTTP/2 -- do
  NOT add Govee's host to `tcp_hosts`, or the HTTP/2 stream is treated as raw TCP
  and the control POST never appears.
- owned devices publish to AWS IoT MQTT (:8883, mutual TLS). that one DOES need the
  IoT host in `tcp_hosts` (TLS decrypt + raw TCP) with the account client cert
  presented upstream; the addon parses MQTT PUBLISH frames out of the byte stream.

## a note on scratch

the decompiles, traffic captures, and protocol notes are kept under a gitignored
`research/` directory. they are not in the repo: they contain a specific app
version's decompiled bytecode and captured account traffic. the toolchain above
lets you regenerate equivalent artifacts yourself from your own capture.
