# BUILD_ARCH is injected by the home-assistant builder action and the Supervisor
# native build as amd64 or aarch64; it selects the matching alpine base in the
# addon stage and the rust musl target in the builder. declared in the global
# scope so it can be used in the addon FROM line. default keeps a plain
# `docker build` working without the arg.
ARG BUILD_ARCH=amd64

####################################################################################################
## Builder: compiles the daemon once (musl static-ish), shared by both targets below
####################################################################################################
FROM rust:1-alpine AS builder

# aws-lc-sys (rustls crypto provider) builds C and asm via cmake/nasm, and
# rusqlite (bundled) builds SQLite C with cc. musl-dev provides the C toolchain;
# clang is needed by aws-lc-sys's bindgen. nasm is for aws-lc's x86 asm.
RUN apk add --no-cache musl-dev cmake make perl clang clang-dev nasm pkgconf

# runtime user, copied into the alpine standalone image below
RUN adduser -D -H -u 1000 -s /sbin/nologin govee \
    && install -d -o govee -g govee /seed-data

# build.rs embeds this as the version string. the docker.yml workflow builds
# from a git context with no .git to fall back to, so the version is passed in.
ARG GOVEE_CI_TAG=""
ENV GOVEE_CI_TAG=${GOVEE_CI_TAG}

# map the HA build arch to the rust musl target
ARG BUILD_ARCH
RUN case "${BUILD_ARCH}" in \
      amd64)   echo "x86_64-unknown-linux-musl"  > /tmp/rust-target ;; \
      aarch64) echo "aarch64-unknown-linux-musl" > /tmp/rust-target ;; \
      *) echo "unsupported BUILD_ARCH: ${BUILD_ARCH}" >&2; exit 1 ;; \
    esac \
    && rustup target add "$(cat /tmp/rust-target)"

WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    target="$(cat /tmp/rust-target)" \
    && cargo build --release --bin govee2mqtt --target "${target}" \
    && cp "target/${target}/release/govee2mqtt" /govee2mqtt

####################################################################################################
## standalone: minimal alpine image for the docker-compose / plain-docker deployment
####################################################################################################
FROM alpine:3.21 AS standalone

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

####################################################################################################
## addon: Home Assistant add-on image, runs the daemon under bashio via run.sh
####################################################################################################
FROM ghcr.io/home-assistant/${BUILD_ARCH}-base:3.21 AS addon

COPY common/run.sh /run.sh
COPY --from=builder /govee2mqtt /app/govee2mqtt
COPY assets /app/assets/

LABEL \
  org.opencontainers.image.title="Home Assistant Add-on: Govee2MQTT" \
  org.opencontainers.image.description="Acts as a bridge between Govee devices and Home Assistant, via the Home Assistant MQTT Integration." \
  org.opencontainers.image.source="https://github.com/tetra-fox/govee2mqtt" \
  org.opencontainers.image.licenses="MIT"

CMD [ "/run.sh" ]
