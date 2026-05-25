# govee2mqtt Dev stack

Infrastructure for testing govee2mqtt against a real Home Assistant instance: a mosquitto broker and a Home Assistant instance. govee2mqtt itself is not in the stack; it runs on the host via `cargo run` so it has direct LAN access for device discovery.

## Bring up the stack

```shell
docker compose -f dev/docker-compose.dev.yml up -d
```

- Home Assistant: <http://localhost:8123> (first run walks you through onboarding)
- Mosquitto: localhost:1883, anonymous access

In Home Assistant, add the MQTT integration (Settings -> Devices & Services -> Add Integration -> MQTT) pointing at broker host `mosquitto`, port `1883`. HA reaches the broker over the compose network; the host-run govee2mqtt reaches it at `localhost:1883`.

## Run govee2mqtt against it

```shell
cp dev/.env.example .env
$EDITOR .env          # fill in Govee credentials
cargo run -- serve
```

`cargo run` loads `.env` automatically (via dotenvy). The broker host/port in the template already point at the stack. With the Nix dev shell:

```shell
nix develop --command cargo run -- serve
```

## Check discovery landed

Once govee2mqtt is running and registered, the devices appear under Settings -> Devices & Services -> MQTT. To watch the raw device-based discovery messages on the broker:

```shell
docker compose -f dev/docker-compose.dev.yml exec mosquitto \
  mosquitto_sub -t 'homeassistant/device/+/config' -v
```

Each device publishes one retained message to `homeassistant/device/<id>/config` containing the `dev` / `o` / `cmps` structure.

## Tear down

```shell
docker compose -f dev/docker-compose.dev.yml down         # keep volumes
docker compose -f dev/docker-compose.dev.yml down -v      # wipe HA config + broker data
```
