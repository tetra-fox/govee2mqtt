#!/usr/bin/env bash
# Assemble a source-building dev add-on, then hand off to the Home Assistant
# devcontainer bootstrap that boots Supervisor and scans local add-ons.
#
# The Supervisor builds a local add-on using the add-on's own directory as the
# Docker build context, and Docker can't COPY from outside that context. The
# merged Dockerfile compiles the Rust source at the repo root, so the source has
# to live inside the add-on directory. This stages addon-dev/ with the source,
# the Dockerfile, run.sh, and a config.yaml that omits the image: key (omitting
# it is what makes the Supervisor build from the Dockerfile instead of pulling a
# published image).
set -euo pipefail

# WORKSPACE_DIRECTORY is the repo root mounted into the Supervisor's local
# add-ons path (set in devcontainer.json).
REPO="${WORKSPACE_DIRECTORY:?WORKSPACE_DIRECTORY not set}"
DEV="${REPO}/addon-dev"

mkdir -p "${DEV}"

# Copy the source the builder stage compiles into the add-on dir. addon-dev/ is
# under WORKSPACE_DIRECTORY, which is bind-mounted, so this writes back to the
# host (addon-dev/ is gitignored). --delete keeps re-runs from accumulating
# stale files. Exclude build output, the integration-test scratch dir, git
# metadata, and addon-dev itself so we don't recurse into the dir we populate.
#
# .github is excluded because the Supervisor scans the add-on dir recursively
# for a build.<yaml|yml|json> config, and would otherwise pick up the
# .github/workflows/build.yml CI workflow and warn that the add-on uses a
# deprecated build.yaml. The Docker build context does not need it anyway, nor
# the other non-source dirs below.
rsync -a --delete \
  --exclude 'target' \
  --exclude 'hatest' \
  --exclude '.git' \
  --exclude '.github' \
  --exclude '.direnv' \
  --exclude 'node_modules' \
  --exclude 'docs' \
  --exclude '.devcontainer' \
  --exclude 'addon' \
  --exclude 'addon-edge' \
  --exclude 'addon-dev' \
  "${REPO}/" "${DEV}/"

# config.yaml without the image: key, slug/name marked dev so it can't be
# confused with the published add-ons that show up in the same local repo.
sed -e '/^image:/d' \
    -e 's/^name:.*/name: Govee2MQTT Dev/' \
    -e 's/^slug:.*/slug: govee2mqtt_dev/' \
    -e 's/^version:.*/version: "dev"/' \
    "${REPO}/addon/config.yaml" > "${DEV}/config.yaml"

exec bash devcontainer_bootstrap
