#!/usr/bin/env bash
# Assemble a source-building dev app, then hand off to the Home Assistant
# devcontainer bootstrap that boots Supervisor and scans local apps.
#
# The Supervisor builds a local app using the app's own directory as the
# Docker build context, and Docker can't COPY from outside that context, nor
# follow symlinks that leave it. The merged Dockerfile compiles the Rust source
# at the repo root, so the source has to be reachable inside the app dir.
#
# Rather than copy the source (which goes stale the moment you edit and then the
# HA "Rebuild" button builds old code), we bind-mount the live source paths into
# addon-dev/. A bind mount appears to Docker as a real in-context directory, so
# the build context always reflects the current working tree with no re-staging.
# Only config.yaml is generated (the image: key is dropped so the Supervisor
# builds from the Dockerfile instead of pulling a published image).
#
# Usage:
#   bootstrap.sh          stage addon-dev/, then boot Supervisor (postStartCommand)
#   bootstrap.sh stage    (re)create the mounts and config only, for use inside a
#                         running container; rarely needed since the mounts are
#                         live, but recreates them if a path was added
set -euo pipefail

# WORKSPACE_DIRECTORY is the repo root mounted into the Supervisor's local
# apps path (set in devcontainer.json).
REPO="${WORKSPACE_DIRECTORY:?WORKSPACE_DIRECTORY not set}"
DEV="${REPO}/addon-dev"

# Source paths the Docker build reads (builder COPY . . compiles these; the app
# stage copies common/run.sh). Bind-mounted live from the repo so edits are
# reflected without re-staging. Everything else in the repo (.github, docs, the
# other app dirs, build output) is deliberately absent from the build context:
# notably .github would otherwise make the Supervisor's recursive
# build.<yaml|yml|json> scan pick up .github/workflows/build.yml and warn about a
# deprecated build config.
SOURCES=(src crates common Cargo.toml Cargo.lock build.rs Dockerfile .dockerignore)

# (Re)create addon-dev/ with the source bind-mounted in. Run as the --stage-impl
# re-exec below because mount --bind needs root and the devcontainer runs us as
# the vscode user.
stage() {
  # unmount any previous bind mounts, then clear addon-dev/ so stale content from
  # an earlier approach (or removed source paths) does not linger in the build
  # context. then recreate from scratch.
  if [ -d "${DEV}" ]; then
    while read -r mnt; do
      umount "${mnt}" 2>/dev/null || true
    done < <(mount | awk -v d="${DEV}/" '$3 ~ "^"d {print $3}' | sort -r)
    rm -rf "${DEV}"
  fi
  mkdir -p "${DEV}"

  for path in "${SOURCES[@]}"; do
    target="${DEV}/${path}"
    if [ -d "${REPO}/${path}" ]; then
      mkdir -p "${target}"
    else
      mkdir -p "$(dirname "${target}")"
      : > "${target}"
    fi
    mount --bind "${REPO}/${path}" "${target}"
  done

  # config.yaml without the image: key, slug/name marked dev so it can't be
  # confused with the published apps that show up in the same local repo. this
  # is generated, not mounted, because it differs from addon/config.yaml.
  sed -e '/^image:/d' \
      -e 's/^name:.*/name: Govee2MQTT Dev/' \
      -e 's/^slug:.*/slug: govee2mqtt_dev/' \
      -e 's/^version:.*/version: "dev"/' \
      "${REPO}/addon/config.yaml" > "${DEV}/config.yaml"
}

# internal: the privileged staging body, invoked via sudo re-exec below
if [ "${1:-}" = "--stage-impl" ]; then
  stage
  exit 0
fi

# mount --bind needs root; the devcontainer runs this as vscode, so re-exec the
# staging under sudo, passing WORKSPACE_DIRECTORY through (sudo strips the env)
sudo --preserve-env=WORKSPACE_DIRECTORY bash "$0" --stage-impl

# re-staging inside an already-running container stops here; the full
# postStartCommand run continues into the Supervisor boot
if [ "${1:-}" = "stage" ]; then
  echo "addon-dev/ mounts are live; rebuild the Govee2MQTT Dev app to pick up edits"
  exit 0
fi

exec bash devcontainer_bootstrap
