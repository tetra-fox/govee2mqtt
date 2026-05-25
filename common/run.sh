#!/usr/bin/with-contenv bashio
# shellcheck shell=bash

export RUST_BACKTRACE=full
export RUST_LOG_STYLE=always
export XDG_CACHE_HOME=/data

wait_for_mqtt() {
  local max_attempts=30
  local attempt=1

  bashio::log.info "mqtt_host was not explicitly configured, waiting for the Mosquitto broker Add-on to become available"

  while [ $attempt -le $max_attempts ]; do
    if bashio::services.available mqtt ; then
      if timeout 2 bash -c "cat < /dev/null > /dev/tcp/$(bashio::services mqtt host)/$(bashio::services mqtt port)" 2>/dev/null; then
        bashio::log.info "MQTT broker is ready!"
        return 0
      fi
    fi

    bashio::log.info "MQTT broker not ready yet (attempt ${attempt}/${max_attempts}), waiting 2 seconds..."
    sleep 2
    attempt=$((attempt + 1))
  done

  bashio::log.error "MQTT broker did not become available after ${max_attempts} attempts"
  return 1
}

if bashio::config.has_value mqtt_host ; then
  GOVEE_MQTT_HOST="$(bashio::config mqtt_host)"
  export GOVEE_MQTT_HOST
else
  if ! wait_for_mqtt ; then
    bashio::exit.nok "Mosquitto MQTT broker is not available"
  fi
  GOVEE_MQTT_HOST="$(bashio::services mqtt 'host')"
  GOVEE_MQTT_PORT="$(bashio::services mqtt 'port')"
  GOVEE_MQTT_USER="$(bashio::services mqtt 'username')"
  GOVEE_MQTT_PASSWORD="$(bashio::services mqtt 'password')"
  export GOVEE_MQTT_HOST GOVEE_MQTT_PORT GOVEE_MQTT_USER GOVEE_MQTT_PASSWORD
fi

if bashio::config.has_value mqtt_port ; then
  GOVEE_MQTT_PORT="$(bashio::config mqtt_port)"
  export GOVEE_MQTT_PORT
fi

if bashio::config.has_value mqtt_username ; then
  GOVEE_MQTT_USER="$(bashio::config mqtt_username)"
  export GOVEE_MQTT_USER
fi

if bashio::config.has_value mqtt_password ; then
  GOVEE_MQTT_PASSWORD="$(bashio::config mqtt_password)"
  export GOVEE_MQTT_PASSWORD
fi

if bashio::config.has_value base_topic ; then
  GOVEE_MQTT_BASE_TOPIC="$(bashio::config base_topic)"
  export GOVEE_MQTT_BASE_TOPIC
fi

if bashio::config.has_value debug_level ; then
  RUST_LOG="$(bashio::config debug_level)"
  export RUST_LOG
fi

if bashio::config.has_value govee_email ; then
  GOVEE_EMAIL="$(bashio::config govee_email)"
  export GOVEE_EMAIL
fi

if bashio::config.has_value govee_password ; then
  GOVEE_PASSWORD="$(bashio::config govee_password)"
  export GOVEE_PASSWORD
fi

if bashio::config.has_value govee_api_key ; then
  GOVEE_API_KEY="$(bashio::config govee_api_key)"
  export GOVEE_API_KEY
fi

if bashio::config.has_value no_multicast ; then
  GOVEE_LAN_NO_MULTICAST="$(bashio::config no_multicast)"
  export GOVEE_LAN_NO_MULTICAST
fi

if bashio::config.has_value broadcast_all ; then
  GOVEE_LAN_BROADCAST_ALL="$(bashio::config broadcast_all)"
  export GOVEE_LAN_BROADCAST_ALL
fi

if bashio::config.has_value global_broadcast ; then
  GOVEE_LAN_BROADCAST_GLOBAL="$(bashio::config global_broadcast)"
  export GOVEE_LAN_BROADCAST_GLOBAL
fi

if bashio::config.has_value scan ; then
  GOVEE_LAN_SCAN="$(bashio::config scan)"
  export GOVEE_LAN_SCAN
fi

if bashio::config.has_value temperature_scale ; then
  GOVEE_TEMPERATURE_SCALE="$(bashio::config temperature_scale)"
  export GOVEE_TEMPERATURE_SCALE
fi

env | grep GOVEE_ | sed -r 's/_(EMAIL|KEY|PASSWORD)=.*/_\1=REDACTED/'
set -x

cd /app || bashio::exit.nok "could not cd to /app"
exec /app/govee2mqtt serve
