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
ci: fmt-check check clippy test

docker:
    docker build .

# build an add-on image locally for testing. the build inputs live in common/;
# copy them into the target dir, build, then clean up. pass addon-edge for the
# edge variant
addon dir="addon":
    cp common/Dockerfile common/build.yaml common/run.sh {{dir}}/
    docker run \
        --rm \
        --privileged \
        -v /var/run/docker.sock:/var/run/docker.sock \
        -v ./{{dir}}:/data \
            ghcr.io/home-assistant/amd64-builder:latest \
            --all \
            --test \
            --target /data
    rm -f {{dir}}/Dockerfile {{dir}}/build.yaml {{dir}}/run.sh

# regenerate addon-edge/config.yaml from the canonical addon/config.yaml. only
# the identity, image, and version differ; schema/options/map are shared. rerun
# after editing addon/config.yaml; ci fails if this is stale
addon-edge-config:
    mkdir -p addon-edge
    cp addon/config.yaml addon-edge/config.yaml
    sed -i 's/^name:.*/name: Govee2MQTT Edge/' addon-edge/config.yaml
    sed -i 's/^slug:.*/slug: govee2mqtt_edge/' addon-edge/config.yaml
    sed -i 's/^version:.*/version: "edge"/' addon-edge/config.yaml
    sed -i 's#^image:.*#image: ghcr.io/tetra-fox/govee2mqtt-edge-{arch}#' addon-edge/config.yaml

# starts hass on http://localhost:7123
container:
    npm install @devcontainers/cli
    npx @devcontainers/cli up --workspace-folder .
    npx @devcontainers/cli exec --workspace-folder . supervisor_run
