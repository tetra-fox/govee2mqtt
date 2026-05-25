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

addon:
    docker run \
        --rm \
        --privileged \
        -v /var/run/docker.sock:/var/run/docker.sock \
        -v ./addon:/data \
            ghcr.io/home-assistant/amd64-builder:latest \
            --all \
            --test \
            --target /data

# starts hass on http://localhost:7123
container:
    npm install @devcontainers/cli
    npx @devcontainers/cli up --workspace-folder .
    npx @devcontainers/cli exec --workspace-folder . supervisor_run
