####################################################################################################
## Builder
####################################################################################################
FROM rust:1-bookworm AS builder

# the openssl crate (vendored) builds OpenSSL from source via perl, and
# rumqttc's rustls provider (aws-lc-sys) and rusqlite (bundled) build C with
# cmake. the rust base image already carries perl and pkg-config; cmake is the
# only one missing. matches flake.nix.
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake \
    && rm -rf /var/lib/apt/lists/*

# runtime user, copied into the distroless image below
RUN useradd --uid 1000 --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin govee \
    && install -d -o govee -g govee /seed-data

# build.rs embeds this as the version string. the workflow builds from a git
# context with no .git to fall back to, so the release tag is passed in.
ARG GOVEE_CI_TAG=""
ENV GOVEE_CI_TAG=${GOVEE_CI_TAG}

WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --bin govee2mqtt \
    && cp target/release/govee2mqtt /govee2mqtt

####################################################################################################
## Final image
####################################################################################################
FROM gcr.io/distroless/cc-debian12

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /app

COPY --from=builder /govee2mqtt /app/govee2mqtt
COPY --from=builder --chown=govee:govee /seed-data /data
COPY assets /app/assets

USER govee:govee
LABEL org.opencontainers.image.source="https://github.com/tetra-fox/govee2mqtt"
ENV \
  RUST_BACKTRACE=full \
  PATH=/app:$PATH \
  XDG_CACHE_HOME=/data

VOLUME /data

CMD ["/app/govee2mqtt", \
  "serve", \
  "--govee-iot-key=/data/iot.key", \
  "--govee-iot-cert=/data/iot.cert"]
