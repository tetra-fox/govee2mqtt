# Privacy

Beyond the data that necessarily reaches Govee's cloud (hosted on AWS) to control your devices, no data about your devices or your govee2mqtt usage leaves your host. Nothing is sent to the maintainers of govee2mqtt.

The credentials you configure are used only to authenticate with Govee's cloud and aren't used for anything else.

Direct BLE control doesn't talk to the cloud at all. Commands and state updates that fit through BLE never leave your local network.

## Cached data

govee2mqtt caches the device list and its configuration on the local filesystem to stay under Govee's API rate limits. The cache lives in the data volume mount point configured when you start the service (`/data` by default in the Docker image, or the addon-managed path under Home Assistant).
