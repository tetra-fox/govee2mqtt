default:
    @just --list

check:
    cargo check --all

clippy:
    cargo clippy --all --all-targets -- -D warnings

test:
    cargo test --all

# imports_granularity = Module in rustfmt.toml needs nightly rustfmt;
# the dev shell points RUSTFMT at it
fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

# pre-merge sweep, mirrors .github/workflows/ci.yml. ordered cheapest-fail-first
ci: fmt-check check clippy test version-check

# the root package version in Cargo.toml is the single source of truth for the
# version. read it for the recipes below
cargo-version := `cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys; print(next(p['version'] for p in json.load(sys.stdin)['packages'] if p['name']=='govee2mqtt'))"`

# sync the add-on configs to the Cargo.toml version: write it into
# addon/config.yaml and regenerate addon-edge. run after bumping the version in
# Cargo.toml
version-sync: addon-edge-config
    sed -i 's/^version:.*/version: "{{cargo-version}}"/' addon/config.yaml

# fail if the add-on configs have drifted from the Cargo.toml version. runs the
# same generation as version-sync, then checks the tree is unchanged, so the
# generation logic lives in one place. mirrors the check in ci.yml
version-check: version-sync
    git diff --exit-code -- addon/config.yaml addon-edge/config.yaml \
      || { echo "add-on configs are stale; run 'just version-sync' and commit" >&2; exit 1; }

# build the standalone daemon image (the distroless target the docker-compose
# deployment uses)
docker:
    docker build --target standalone .

# build the add-on image locally for a quick sanity check. BUILD_ARCH selects
# the HA alpine base in the Dockerfile; this builds for the host arch only. CI
# (build.yml) does the real per-arch multi-arch build and publish
addon arch="amd64":
    docker build --target addon --build-arg BUILD_ARCH={{arch}} .

# regenerate addon-edge/config.yaml from the canonical addon/config.yaml. only
# the identity, image, and version differ; schema/options/map are shared. rerun
# after editing addon/config.yaml; ci fails if this is stale
addon-edge-config:
    mkdir -p addon-edge
    cp addon/config.yaml addon-edge/config.yaml
    sed -i 's/^name:.*/name: Govee2MQTT Edge/' addon-edge/config.yaml
    sed -i 's/^slug:.*/slug: govee2mqtt_edge/' addon-edge/config.yaml
    sed -i 's/^version:.*/version: "edge"/' addon-edge/config.yaml
    sed -i 's#^image:.*#image: ghcr.io/tetra-fox/govee2mqtt-addon-edge#' addon-edge/config.yaml

# boot Home Assistant with the dev add-on built from local source, on
# http://localhost:7123. the devcontainer's postStartCommand (bootstrap.sh)
# stages addon-dev/ and runs devcontainer_bootstrap; supervisor_run then starts
# the Supervisor
container:
    npm install @devcontainers/cli
    npx @devcontainers/cli up --workspace-folder .
    npx @devcontainers/cli exec --workspace-folder . supervisor_run

# rebuild the dev add-on against the current working tree. run this INSIDE the
# devcontainer after editing code. the add-on's source is bind-mounted live (see
# .devcontainer/bootstrap.sh), so this just triggers the rebuild; the HA UI
# "Rebuild" button works identically now that the context is always live
dev-rebuild:
    ha apps rebuild local_govee2mqtt_dev
