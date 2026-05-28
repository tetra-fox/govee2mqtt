# Device support

This isn't a project that independently reverse-engineers every Govee device. Most device-specific code is a port of codecs and frame layouts from open-source projects that did the underlying work: [homebridge-govee](https://github.com/bwp91/homebridge-govee/) for the AWS IoT transport and many command payloads, [Bluetooth-Devices/govee-ble](https://github.com/Bluetooth-Devices/govee-ble) for BLE conventions, and the [lasswellt protocol reference](https://github.com/lasswellt/govee-homeassistant/blob/master/docs/govee-protocol-reference.md) for general structure. Where a device needs something none of those upstreams covered, we did our own reverse engineering to confirm the shape on a real device and fill in the new pieces. That's how the H5082 per-outlet countdown semantics, the H6093 aurora and laser model, and a handful of other device-specific surfaces landed.

What "supported" means varies in confidence. Some SKUs have captured wire traces and have been verified end-to-end; others share a chip and capability profile with a verified SKU and are expected to work; a long tail is encoded as a generic family member because that's the most likely shape and there's no signal saying otherwise. The full mapping lives in [`src/service/quirks.rs`](../src/service/quirks.rs); that file is the source of truth for which models are explicitly known and how they're classified.

## By family

| Family                                                       | LAN API                                     | IoT        | Platform API                                     | BLE                                                          |
| ------------------------------------------------------------ | ------------------------------------------- | ---------- | ------------------------------------------------ | ------------------------------------------------------------ |
| Lights and LED strips                                        | Modern WiFi controllers, toggled per-device | Yes (most) | Most WiFi models; covers scenes, music, segments | Generic codec for BLE-only models; H6093 has custom dispatch |
| Plugs and sockets                                            | No                                          | Yes        | Yes                                              | H5082 has custom dispatch; others fall through               |
| Humidifiers, dehumidifiers, aroma diffusers                  | No                                          | Yes        | Yes, sometimes patchy for night lights           | H7160 has custom dispatch                                    |
| Fans, air purifiers, space heaters                           | No                                          | Yes        | Yes                                              | No                                                           |
| Kettles                                                      | No                                          | Yes        | Yes; temperature targets and presets             | No                                                           |
| Sensors (temp, humidity, leak, motion, contact, CO2, button) | No                                          | Yes        | Mixed                                            | No                                                           |
| Ice makers                                                   | No                                          | Yes        | Yes                                              | No                                                           |

All known LAN-API-capable devices are lights; appliances don't support fully local control. That's a Govee firmware limitation, not a govee2mqtt one.

## Standouts

These devices have custom handling beyond the generic family treatment.

### H5082 (dual-outlet plug)

Per-outlet on/off, per-outlet auto-on and auto-off countdown duration entities, and recurring per-outlet timers via the MQTT topic `/h5082/<id>/timer/set`. The codec was reverse-engineered from device traffic. Talks BLE directly when in range; otherwise the BLE frames are wrapped and tunneled over the AWS IoT relay.

Other socket SKUs in `quirks.rs` (H5001, H5080, H5083, H5086, H5160, H7014) don't get the countdown and timer entities, but they work for binary on/off through the cloud transports.

### H6093 (star projector)

Two independent light layers (aurora and laser), each with their own brightness, color modes, effects, and speed; plus stars, orbit, flashing, pairing, and auto-off scheduling. State is held in Govee's cloud `bizType:3` store and refreshed on demand. Same direct-BLE-with-IoT-tunnel-fallback model as H5082.

### H7160 (humidifier)

Adds RGBA night light brightness and color on top of the standard humidifier mist-level controls. The night light is controllable over BLE directly when in range. Shares generic humidifier handling for mist mode and target humidity.

## BLE

Two paths exist when `enable_ble` is on:

- **Rich BLE dispatch** for SKUs with custom command surfaces (H5082, H6093, H7160). These get device-specific encode and decode and synthesized Home Assistant entities.
- **Generic BLE codec** for BLE-only lights (H6052, H6053, H6102, H6119, H617C, H617E, H617F, and similar). Power and scene-code frames only; same control surface as the cloud path, without the cloud round-trip.

BLE control requires a Bluetooth adapter on the host. See [Direct BLE](../README.md#direct-ble) for setup.

## Don't see your SKU?

Grep [`src/service/quirks.rs`](../src/service/quirks.rs). If your model is listed there it's at least known and the entry tells you which transports it's expected to work over. If it isn't listed and you have one in hand, please file an issue with the model number; the more wire traces we have, the better the generic fallback gets.

To capture traces yourself, run with `RUST_LOG=govee2mqtt=trace,govee_api=trace` (or set that as the `debug_level` field in the Home Assistant app's Configuration tab). The log will contain every inbound IoT message and every LAN discovery reply, including from devices govee2mqtt doesn't recognize yet. Topic strings include device IDs and trace output can contain tokens, so redact before sharing.
