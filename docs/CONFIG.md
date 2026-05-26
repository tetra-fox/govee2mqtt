# Configuration Options

## Govee Credentials

While `govee2mqtt` can run without any govee credentials, it can only discover
and control the devices for which you have already enabled LAN control.

It is recommended that you configure at least your Govee username and password
prior to your first run, as that is the only way for `govee2mqtt` to determine
room names to pre-assign your lights into the appropriate Home Assistant areas.

For scene control, for devices that don't support the LAN API, a Govee API Key
is required. If you don't already have one, [you can find instructions on
obtaining one
here](https://developer.govee.com/reference/apply-you-govee-api-key).

| CLI                | ENV                   | AddOn            | Purpose                                                  |
| ------------------ | --------------------- | ---------------- | -------------------------------------------------------- |
| `--govee-email`    | `GOVEE2MQTT_EMAIL`    | `govee_email`    | The email address you registered with your govee account |
| `--govee-password` | `GOVEE2MQTT_PASSWORD` | `govee_password` | The password you registered for your govee account       |
| `--api-key`        | `GOVEE2MQTT_API_KEY`  | `govee_api_key`  | The API key you requested from Govee support             |

_Concerned about sharing your credentials? See [Privacy](PRIVACY.md) for
information about how data is used and retained by `govee2mqtt`_

## LAN API Control

A number of Govee's devices support a local control protocol that doesn't require
your primary internet connection to be online. This offers the lowest latency
for control and is the preferred way for `govee2mqtt` to interact with your
devices.

The [Govee LAN API is described in more detail
here](https://app-h5.govee.com/user-manual/wlan-guide), including a list of
supported devices.

_Note that you must use the Govee Home app to enable the LAN API for each
individual device before it will be possible for `govee2mqtt` to control
it via the LAN API._

In theory the LAN API is zero-configuration and auto-discovery, but this
relies on your network supporting multicast-UDP, which is challenging
on some networks, especially across wifi access points and routers.

| CLI                  | ENV                                     | AddOn              | Purpose                                                                                                                                                                                                                                                                                                                                                                                                              |
| -------------------- | --------------------------------------- | ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--no-multicast`     | `GOVEE2MQTT_LAN_NO_MULTICAST=true`      | `no_multicast`     | Do not multicast discovery packets to the Govee multicast group `239.255.255.250`. It is not recommended to use this option.                                                                                                                                                                                                                                                                                         |
| `--broadcast-all`    | `GOVEE2MQTT_LAN_BROADCAST_ALL=true`     | `broadcast_all`    | Enumerate all non-loopback network interfaces and send discovery packets to the broadcast address of each one, individually. This may be a good option if multicast-UDP doesn't work well on your network                                                                                                                                                                                                            |
| `--global-broadcast` | `GOVEE2MQTT_LAN_BROADCAST_GLOBAL=true`  | `global_broadcast` | Send discovery packets to the global broadcast address `255.255.255.255`. This may be a possible solution if multicast-UDP doesn't work well on your network.                                                                                                                                                                                                                                                        |
| `--scan`             | `GOVEE2MQTT_LAN_SCAN=10.0.0.1,10.0.0.2` | `scan`             | Specify a list of addresses that should be scanned by sending them discovery packets. Each element in the list can be an individual IP address (eg: the address of a specific device: be sure to assign it a static IP in your DHCP or other network setup!) or a network broadcast address like `10.0.0.255` for networks that are reachable but not directly plumbed on the machine where `govee2mqtt` is running. |

[Read more about LAN API Requirements here](LAN.md)

## MQTT Configuration

In order to make your devices appear in Home Assistant, you will need to have configured Home Assistant with an MQTT broker.

- [follow these steps](https://www.home-assistant.io/integrations/mqtt/#configuration)

You will also need to configure `govee2mqtt` to use the same broker:

| CLI                 | ENV                          | AddOn           | Purpose                                                                                                                                                                                                                       |
| ------------------- | ---------------------------- | --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--mqtt-host`       | `GOVEE2MQTT_MQTT_HOST`       | `mqtt_host`     | The host name or IP address of your mqtt broker. This should be the same broker that you have configured in Home Assistant.                                                                                                   |
| `--mqtt-port`       | `GOVEE2MQTT_MQTT_PORT`       | `mqtt_port`     | The port number of the mqtt broker. The default is `1883`                                                                                                                                                                     |
| `--mqtt-username`   | `GOVEE2MQTT_MQTT_USER`       | `mqtt_username` | If your broker requires authentication, the username to use                                                                                                                                                                   |
| `--mqtt-password`   | `GOVEE2MQTT_MQTT_PASSWORD`   | `mqtt_password` | If your broker requires authentication, the password to use                                                                                                                                                                   |
| `--mqtt-base-topic` | `GOVEE2MQTT_MQTT_BASE_TOPIC` | `base_topic`    | The prefix for all MQTT topics and Home Assistant entity unique ids. Defaults to `govee2mqtt`. If you are migrating from an upstream `wez/govee2mqtt` install and want to keep your existing entities, set this to `gv2mqtt`. |

## Device Availability

A device is reported unavailable in Home Assistant once `govee2mqtt` hasn't heard from it for `availability_timeout` seconds. Lower values detect an unplugged or offline device faster, at the cost of polling each device for its status more often over the (free) AWS IoT channel. The Govee cloud itself marks a device offline within about a minute.

| CLI                      | ENV                              | AddOn                  | Purpose                                                                       |
| ------------------------ | -------------------------------- | ---------------------- | ----------------------------------------------------------------------------- |
| `--availability-timeout` | `GOVEE2MQTT_AVAILABILITY_TIMEOUT`| `availability_timeout` | Seconds of silence before a device is reported offline. Defaults to `300`.    |
