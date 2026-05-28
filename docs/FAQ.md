# FAQ

## Why can't I turn off a single segment?

Govee's segment API only takes brightness and color, not on/off. Home Assistant's Light entity assumes every light has a power control, so segments are exposed with a power toggle that does nothing. The toggle can't be removed from the HA UI.

## Why is my control over a segment limited?

govee2mqtt forwards the segment commands to Govee and what happens next depends on the device. Some devices can't set a segment brightness to zero; others tie segment brightness to the overall light brightness. govee2mqtt can't override that.

## How do I enable video effects on a light?

The Govee API doesn't return video effects, so they aren't listed in HA's effect list. Workaround: use the Govee Home app to create a Tap-to-Run shortcut or a saved Snapshot that activates the effect.

- Tap-to-Run shortcuts appear as Scene entities in Home Assistant.
- Snapshots appear in the device's effect list.

After creating them in the Govee app, click "Purge Caches" on the `Govee2MQTT` device in the MQTT integration to pull them in.

## My device shows up as greyed out / unavailable in Home Assistant

Usually a registration glitch. Check the HA logs for entries from `govee2mqtt` or `mqtt`, and the govee2mqtt log itself.

If you suspect state isn't being received, set the debug filter to `govee2mqtt=trace,govee_api=trace` and watch the inbound traffic. If nothing arrives for the device at all, it's a transport problem, not a registration one.

As a last resort, delete the device from the MQTT integration in Home Assistant, then click "Purge Caches" on the `Govee2MQTT` device to re-publish discovery.

## Is my device supported?

See [device support](SKUS.md).

## Please add support for HXXXX

govee2mqtt is mostly a port of codec work from other open-source projects plus some of our own RE for specific shapes; see [device support](SKUS.md) for the framing. If a device isn't supported, that usually means no upstream has the protocol either. Captured wire traces help; the trace capture instructions are at the bottom of [device support](SKUS.md).

## The device MAC addresses in the logs don't match the MACs on my network

Govee device IDs aren't MAC addresses. For some devices the device ID contains the BLE MAC as a substring, but the ID itself is larger than a MAC.

## My device should support LAN but isn't responding to probes

See [LAN.md](LAN.md) for protocol requirements and discovery options.

## "devices not belong you" error in logs

Returned by Govee's Platform API when a BLE-only device with no WiFi is queried. Please file an issue with the SKU so we can add a quirks entry that skips the Platform call for that model.
