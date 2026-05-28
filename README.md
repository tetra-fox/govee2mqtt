# govee2mqtt

Control your Govee devices via MQTT

## Features

- **LAN-first transport selection** - `govee2mqtt` switches transports per command, preferring the LAN API where it can carry the command, and falling back to IoT MQTT or the platform REST API otherwise. Many device capabilities (and some whole devices) aren't exposed over the LAN API at all; those always go through one of the other transports.
- **Shared device support** - devices shared to your account by another Govee user are controlled and polled through Govee's REST relay over IoT, so they appear and work alongside your own devices even though the platform API doesn't return them.
- **Direct BLE control** - owned devices can be controlled directly over Bluetooth, used ahead of the cloud when the device is in range and falling back to the cloud transports otherwise. Opt-in via `enable_ble` and requires a Bluetooth adapter on the host.
- Support for per-device modes and scenes.
- Support for the undocumented AWS IoT interface to your devices, provides low latency status updates.
- Support for the new [Platform API](https://developer.govee.com/reference/get-you-devices) in case the AWS IoT or LAN control is unavailable.

| Feature                      | Requires       | Notes                                                                                          |
| ---------------------------- | -------------- | ---------------------------------------------------------------------------------------------- |
| DIY Scenes                   | API Key        | Find in the list of Effects for the light in Home Assistant                                    |
| Music Modes                  | API Key        | Find in the list of Effects for the light in Home Assistant                                    |
| Tap-to-Run / One Click Scene | IoT            | Find in the overall list of Scenes in Home Assistant, as well as under the `Govee2MQTT` device |
| Live Device Status Updates   | LAN and/or IoT | Devices typically report most changes within a couple of seconds.                              |
| Segment Color                | API Key        | Find the `Segment 00X` light entities associated with your main light device in Home Assistant |

- `API Key` means that you have [applied for a key from Govee](https://developer.govee.com/reference/apply-you-govee-api-key) and have configured it for use in govee2mqtt
- `IoT` means that you have configured your Govee account email and password for use in govee2mqtt, which will then attempt to use the _undocumented and likely unsupported_ AWS MQTT-based IoT service
- `LAN` means that you have enabled the [Govee LAN API](https://app-h5.govee.com/user-manual/wlan-guide) on supported devices and that the LAN API protocol is functional on your network

## Usage

### Home Assistant App

0. If you don't already have one, set up an MQTT broker in Home Assistant. Go to `Settings -> Apps -> Install app`, search for `Mosquitto broker` under `Official apps`. Install and start the broker.
1. Go back to the App Store, click ⋮ -> Repositories, click `Add`, fill in `https://github.com/tetra-fox/govee2mqtt` and click Add in the dialog.
   - Or use this convenient button: [![Open your Home Assistant instance and show the add app repository dialog with a specific repository URL pre-filled.](https://my.home-assistant.io/badges/supervisor_add_addon_repository.svg)](https://my.home-assistant.io/redirect/supervisor_add_addon_repository/?repository_url=https%3A%2F%2Fgithub.com%2Ftetra-fox%2Fgovee2mqtt)
2. Go back to the App Store and you will see two new apps
   - **Govee2MQTT** - stable release that tracks tagged releases
   - **Govee2MQTT Edge** - tracks the `main` branch for testing purposes. This is not recommended as unstable and or half-working code may be committed to this branch, potentially breaking your device entries or automations.
3. Install **Govee2MQTT**, set your Govee credentials and MQTT details on its Configuration tab (see [Configuration](docs/CONFIG.md)), then start the add-on.

### Docker

- [Running it in Docker](docs/DOCKER.md)
- [Configuration](docs/CONFIG.md)

## Have a question?

- [Is my device supported?](docs/SKUS.md)
- [Check out the FAQ](docs/FAQ.md)

## Credits

This project began as a hard fork of [wez/govee2mqtt](https://github.com/wez/govee2mqtt) by Wez Furlong and has since significantly diverged in architecture, tooling, APIs, licensing, and project direction. It builds on his [Govee LAN Control](https://github.com/wez/govee-lan-hass/).

The AWS IoT support was made possible by the work of @bwp91 in [homebridge-govee](https://github.com/bwp91/homebridge-govee/).

BLE work referenced [Bluetooth-Devices/govee-ble](https://github.com/Bluetooth-Devices/govee-ble) for Govee BLE conventions.

<https://raw.githubusercontent.com/lasswellt/govee-homeassistant/refs/heads/master/docs/govee-protocol-reference.md>

## License

This fork is licensed under the GNU General Public License version 3 or later; see [LICENSE](LICENSE). Portions originate from wez/govee2mqtt under the MIT License, preserved in [LICENSE.MIT](LICENSE.MIT).
