# govee2mqtt

Control your Govee devices via MQTT

## Features

- **LAN-first transport selection** - `govee2mqtt` switches transports per command, preferring the LAN API where it can carry the command, and falling back to IoT MQTT or the platform REST API otherwise. Many device capabilities (and some whole devices) aren't exposed over the LAN API at all; those always go through one of the other transports.
- **AWS IoT interface** - the same undocumented MQTT transport the Govee Home app uses; carries sub-second push updates when device state changes and acts as a control path for capabilities the LAN API doesn't reach. Gated behind your Govee account email and password.
- **[Platform API](https://developer.govee.com/reference/get-you-devices) fallback** - Govee's documented HTTP API; the only path to DIY scenes, music modes, and per-segment color, and the fallback when LAN and IoT can't carry a command. Gated behind a Govee API key.
- **Direct BLE control** - owned devices can be controlled directly over Bluetooth, used ahead of the cloud when the device is in range and falling back to the cloud transports otherwise. Opt-in via `enable_ble` and requires a Bluetooth adapter on the host.
- **Shared device support** - devices shared to your account by another Govee user are controlled and polled through Govee's REST relay over IoT, so they appear and work alongside your own devices even though the platform API doesn't return them.
- **Modes, scenes, and effects** - each device family's own controls (fan speed, kettle preset, humidifier mist level, light effect modes) surface as native Home Assistant select, number, and effect entities. Light scenes, DIY scenes, music modes, and Tap-to-Run shortcuts from the Govee app appear as effect or scene entries.

## Configuration

What you provide determines what you get. None of these are strictly required, but the device surface shrinks fast without them.

| You provide                                                                                           | You get                                                                                |
| ----------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| Govee account email + password                                                                        | Low-latency state updates over AWS IoT; Tap-to-Run scenes; shared devices              |
| [Govee API key](https://developer.govee.com/reference/apply-you-govee-api-key)                        | DIY scenes, music modes, and per-segment color on devices that aren't reachable by LAN |
| [LAN API toggled per device](https://app-h5.govee.com/user-manual/wlan-guide) (in the Govee Home app) | Local control of supported lights, no cloud round-trip                                 |
| Bluetooth adapter on the host                                                                         | Direct BLE control of owned devices in range, ahead of the cloud                       |

Each setting below is available as a CLI flag, an env var, or a Home Assistant app option. Pick the column that matches how you're running govee2mqtt.

### Govee Credentials

Credentials are optional but unlock most of the device surface. Without them, only LAN-enabled devices are reachable. Without an API key, scenes and per-segment color on non-LAN devices are unavailable. See [Privacy](docs/PRIVACY.md) for how credentials are used.

| CLI                | ENV                   | App              | Purpose                                                  |
| ------------------ | --------------------- | ---------------- | -------------------------------------------------------- |
| `--govee-email`    | `GOVEE2MQTT_EMAIL`    | `govee_email`    | The email address you registered with your Govee account |
| `--govee-password` | `GOVEE2MQTT_PASSWORD` | `govee_password` | The password for your Govee account                      |
| `--govee-api-key`  | `GOVEE2MQTT_API_KEY`  | `govee_api_key`  | Your Govee Platform API key                              |

### MQTT Broker

govee2mqtt needs the same broker that Home Assistant's MQTT integration is pointed at.

| CLI                 | ENV                          | App               | Purpose                                              |
| ------------------- | ---------------------------- | ----------------- | ---------------------------------------------------- |
| `--mqtt-host`       | `GOVEE2MQTT_MQTT_HOST`       | `mqtt_host`       | Broker host or IP                                    |
| `--mqtt-port`       | `GOVEE2MQTT_MQTT_PORT`       | `mqtt_port`       | Broker port. Defaults to `1883`                      |
| `--mqtt-username`   | `GOVEE2MQTT_MQTT_USERNAME`   | `mqtt_username`   | Username, if your broker requires auth               |
| `--mqtt-password`   | `GOVEE2MQTT_MQTT_PASSWORD`   | `mqtt_password`   | Password, if your broker requires auth               |
| `--mqtt-base-topic` | `GOVEE2MQTT_MQTT_BASE_TOPIC` | `mqtt_base_topic` | Topic and entity-id prefix. Defaults to `govee2mqtt` |

If you're migrating from an upstream `wez/govee2mqtt` install and want to keep your existing entities, set the base topic to `gv2mqtt`.

### LAN API

UDP-based local control. Each device must have its LAN API toggled on in the Govee Home app first. See [LAN.md](docs/LAN.md) for protocol details and network requirements.

| CLI                  | ENV                                     | App                | Purpose                                                                              |
| -------------------- | --------------------------------------- | ------------------ | ------------------------------------------------------------------------------------ |
| `--no-multicast`     | `GOVEE2MQTT_LAN_NO_MULTICAST=true`      | `no_multicast`     | Skip the `239.255.255.250` multicast group. Not recommended.                         |
| `--broadcast-all`    | `GOVEE2MQTT_LAN_BROADCAST_ALL=true`     | `broadcast_all`    | Broadcast discovery to every non-loopback interface. Try when multicast is flaky.    |
| `--global-broadcast` | `GOVEE2MQTT_LAN_GLOBAL_BROADCAST=true`  | `global_broadcast` | Send discovery to `255.255.255.255`.                                                 |
| `--scan`             | `GOVEE2MQTT_LAN_SCAN=10.0.0.1,10.0.0.2` | `scan`             | Comma-separated list of unicast IPs or subnet broadcast addresses to probe directly. |

### Device Availability

A device is reported unavailable in Home Assistant once it's been silent for `availability_timeout` seconds. Lower values catch unplugged devices faster, at the cost of more IoT polling. Govee's own cloud marks a device offline within about a minute.

| CLI                      | ENV                               | App                    | Purpose                                                                    |
| ------------------------ | --------------------------------- | ---------------------- | -------------------------------------------------------------------------- |
| `--availability-timeout` | `GOVEE2MQTT_AVAILABILITY_TIMEOUT` | `availability_timeout` | Seconds of silence before a device is reported offline. Defaults to `300`. |

### Direct BLE

Owned devices in range of the host's Bluetooth adapter are controlled directly over BLE, ahead of the cloud transports. No-op if no adapter is found.

| CLI            | ENV                     | App          | Purpose                                     |
| -------------- | ----------------------- | ------------ | ------------------------------------------- |
| `--enable-ble` | `GOVEE2MQTT_ENABLE_BLE` | `enable_ble` | Enable direct BLE control of owned devices. |

## Running

### Home Assistant App

0. If you don't already have one, set up an MQTT broker in Home Assistant. Go to `Settings -> Apps -> Install app`, search for `Mosquitto broker` under `Official apps`. Install and start the broker.
1. Go back to the App Store, click ⋮ -> Repositories, click `Add`, fill in `https://github.com/tetra-fox/govee2mqtt` and click Add in the dialog.
   - Or use this convenient button: [![Open your Home Assistant instance and show the add app repository dialog with a specific repository URL pre-filled.](https://my.home-assistant.io/badges/supervisor_add_addon_repository.svg)](https://my.home-assistant.io/redirect/supervisor_add_addon_repository/?repository_url=https://github.com/tetra-fox/govee2mqtt)
2. Go back to the App Store and you will see two new apps
   - **Govee2MQTT** - stable release that tracks tagged releases
   - **Govee2MQTT Edge** - tracks the `main` branch for testing purposes. This is not recommended as unstable and or half-working code may be committed to this branch, potentially breaking your device entries or automations.
3. Install **Govee2MQTT**, set your Govee credentials and MQTT details on its Configuration tab (see [Configuration](#configuration)), then start the app.

### Docker

Drop the settings you want into a `.env`:

```bash
GOVEE2MQTT_EMAIL=user@example.com
GOVEE2MQTT_PASSWORD=secret
GOVEE2MQTT_API_KEY=UUID

GOVEE2MQTT_MQTT_HOST=mqtt
GOVEE2MQTT_MQTT_PORT=1883
#GOVEE2MQTT_MQTT_USERNAME=user
#GOVEE2MQTT_MQTT_PASSWORD=password

GOVEE2MQTT_TEMPERATURE_SCALE=C

# Always colorize log output
RUST_LOG_STYLE=always
# Uncomment to bump log verbosity
#RUST_LOG=govee2mqtt=trace,govee_api=trace

TZ=America/Phoenix
```

`docker-compose.yml`:

```yaml
name: govee2mqtt
services:
  govee2mqtt:
    image: ghcr.io/tetra-fox/govee2mqtt:latest
    container_name: govee2mqtt
    restart: unless-stopped
    env_file:
      - .env
    # Host networking is required for LAN discovery
    network_mode: host
    # Bind-mount the data volume on the host instead of using a Docker volume
    #volumes:
    #  - '/path/to/data:/data'
```

Then `docker compose up -d`. Logs: `docker logs govee2mqtt --follow`.

## More

- [Is my device supported?](docs/SKUS.md)
- [LAN API troubleshooting](docs/LAN.md)
- [FAQ](docs/FAQ.md)
- [Privacy](docs/PRIVACY.md)

## Credits

- Hard-forked from [wez/govee2mqtt](https://github.com/wez/govee2mqtt) by Wez Furlong; descended in part from his earlier [Govee LAN Control](https://github.com/wez/govee-lan-hass/). This fork has since diverged substantially in architecture, scope, and licensing.
- AWS IoT support follows the approach in @bwp91's [homebridge-govee](https://github.com/bwp91/homebridge-govee/).
- BLE conventions drawn from [Bluetooth-Devices/govee-ble](https://github.com/Bluetooth-Devices/govee-ble).
- Protocol notes from [lasswellt/govee-homeassistant](https://github.com/lasswellt/govee-homeassistant/blob/master/docs/govee-protocol-reference.md).

## License

This fork is licensed under the GNU General Public License version 3 or later; see [LICENSE](LICENSE). Portions originate from wez/govee2mqtt under the MIT License, preserved in [LICENSE.MIT](LICENSE.MIT).
