#!/usr/bin/with-contenv bashio
# shellcheck shell=bash

export RUST_BACKTRACE=full
export RUST_LOG_STYLE=always
export XDG_CACHE_HOME=/data

wait_for_mqtt() {
  local max_attempts=30
  local attempt=1

  bashio::log.info "mqtt_host not configured; waiting for the Mosquitto broker App to become available"

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
  GOVEE2MQTT_MQTT_HOST="$(bashio::config mqtt_host)"
  export GOVEE2MQTT_MQTT_HOST
else
  if ! wait_for_mqtt ; then
    bashio::exit.nok "Mosquitto MQTT broker is not available"
  fi
  GOVEE2MQTT_MQTT_HOST="$(bashio::services mqtt 'host')"
  GOVEE2MQTT_MQTT_PORT="$(bashio::services mqtt 'port')"
  GOVEE2MQTT_MQTT_USER="$(bashio::services mqtt 'username')"
  GOVEE2MQTT_MQTT_PASSWORD="$(bashio::services mqtt 'password')"
  export GOVEE2MQTT_MQTT_HOST GOVEE2MQTT_MQTT_PORT GOVEE2MQTT_MQTT_USER GOVEE2MQTT_MQTT_PASSWORD
fi

if bashio::config.has_value mqtt_port ; then
  GOVEE2MQTT_MQTT_PORT="$(bashio::config mqtt_port)"
  export GOVEE2MQTT_MQTT_PORT
fi

if bashio::config.has_value mqtt_username ; then
  GOVEE2MQTT_MQTT_USER="$(bashio::config mqtt_username)"
  export GOVEE2MQTT_MQTT_USER
fi

if bashio::config.has_value mqtt_password ; then
  GOVEE2MQTT_MQTT_PASSWORD="$(bashio::config mqtt_password)"
  export GOVEE2MQTT_MQTT_PASSWORD
fi

if bashio::config.has_value base_topic ; then
  GOVEE2MQTT_MQTT_BASE_TOPIC="$(bashio::config base_topic)"
  export GOVEE2MQTT_MQTT_BASE_TOPIC
fi

if bashio::config.has_value debug_level ; then
  RUST_LOG="$(bashio::config debug_level)"
  export RUST_LOG
fi

if bashio::config.has_value govee_email ; then
  GOVEE2MQTT_EMAIL="$(bashio::config govee_email)"
  export GOVEE2MQTT_EMAIL
fi

if bashio::config.has_value govee_password ; then
  GOVEE2MQTT_PASSWORD="$(bashio::config govee_password)"
  export GOVEE2MQTT_PASSWORD
fi

if bashio::config.has_value govee_api_key ; then
  GOVEE2MQTT_API_KEY="$(bashio::config govee_api_key)"
  export GOVEE2MQTT_API_KEY
fi

if bashio::config.has_value no_multicast ; then
  GOVEE2MQTT_LAN_NO_MULTICAST="$(bashio::config no_multicast)"
  export GOVEE2MQTT_LAN_NO_MULTICAST
fi

if bashio::config.has_value broadcast_all ; then
  GOVEE2MQTT_LAN_BROADCAST_ALL="$(bashio::config broadcast_all)"
  export GOVEE2MQTT_LAN_BROADCAST_ALL
fi

if bashio::config.has_value global_broadcast ; then
  GOVEE2MQTT_LAN_BROADCAST_GLOBAL="$(bashio::config global_broadcast)"
  export GOVEE2MQTT_LAN_BROADCAST_GLOBAL
fi

if bashio::config.has_value scan ; then
  GOVEE2MQTT_LAN_SCAN="$(bashio::config scan)"
  export GOVEE2MQTT_LAN_SCAN
fi

if bashio::config.has_value temperature_scale ; then
  GOVEE2MQTT_TEMPERATURE_SCALE="$(bashio::config temperature_scale)"
  export GOVEE2MQTT_TEMPERATURE_SCALE
fi

if bashio::config.has_value availability_timeout ; then
  GOVEE2MQTT_AVAILABILITY_TIMEOUT="$(bashio::config availability_timeout)"
  export GOVEE2MQTT_AVAILABILITY_TIMEOUT
fi

# log the resolved config with secrets redacted. this is diagnostic only, so it
# must not abort startup: grep exits non-zero when nothing matches, and the
# script runs under bashio's set -e, so swallow that.
env | grep GOVEE2MQTT_ | sed -r 's/_(EMAIL|KEY|PASSWORD)=.*/_\1=REDACTED/' || true
set -x

cd /app || bashio::exit.nok "could not cd to /app"
exec /app/govee2mqtt serve
