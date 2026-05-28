# LAN API

[Govee's LAN control API](https://app-h5.govee.com/user-manual/wlan-guide) is a UDP-based protocol. Requirements on your network:

- govee2mqtt must be able to bind UDP port `4002` on its host.
- Each Govee device must have LAN API access enabled individually in the Govee Home app.
- UDP ports `4001` and `4003` must be reachable on each device.
- The device replies from port `4002` regardless of the source port, so your network must allow that reply path back to govee2mqtt.

LAN-capable devices are all lights. Appliances don't expose LAN control; that's a Govee firmware limitation.

## Discovery

Devices with the LAN protocol enabled listen on UDP port `4001` and join the multicast group `239.255.255.250`. In theory a client only needs to multicast to that group.

In practice, multicast UDP is patchy across routers, especially when traffic crosses WiFi access points. See [LAN API config](../README.md#lan-api) for the flags, env vars, and app options that switch govee2mqtt over to broadcasts or to direct unicast probes against a list of IPs.

## Router and network tips

- Some routers drop multicast UDP between WLAN and LAN. Check your router's options. Don't confuse this with multicast DNS (mDNS); having mDNS working doesn't imply general multicast does.
- Try `broadcast_all`, which sends UDP broadcasts to each non-loopback interface instead of relying on multicast.
- Assign a static IP to the device in your DHCP setup and add that IP to the [scan list](../README.md#lan-api). Heavier on the network but works on isolated VLANs.
- If you have an IoT VLAN, make sure your firewall isn't blocking the ports above between the VLAN and the host running govee2mqtt.
