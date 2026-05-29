# BUILD_ARCH is injected by the HA builder action; default keeps plain
# `docker build` working without the arg.
ARG BUILD_ARCH=amd64

FROM node:24-alpine AS ui-builder
RUN corepack enable && corepack prepare pnpm@latest --activate
WORKDIR /ui
COPY ui/package.json ui/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY ui/ ./
RUN pnpm build

FROM rust:1-alpine AS chef

# musl-dev + clang for aws-lc-sys (rustls) bindgen and rusqlite cc, nasm for
# aws-lc x86 asm, dbus-dev for btleplug's BlueZ backend.
RUN apk add --no-cache musl-dev cmake make perl clang clang-dev nasm pkgconf dbus-dev
RUN cargo install cargo-chef --locked

ARG BUILD_ARCH
RUN case "${BUILD_ARCH}" in \
      amd64)   echo "x86_64-unknown-linux-musl"  > /tmp/rust-target ;; \
      aarch64) echo "aarch64-unknown-linux-musl" > /tmp/rust-target ;; \
      *) echo "unsupported BUILD_ARCH: ${BUILD_ARCH}" >&2; exit 1 ;; \
    esac \
    && rustup target add "$(cat /tmp/rust-target)"

WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

# runtime user, copied into the standalone image below
RUN adduser -D -H -u 1000 -s /sbin/nologin govee \
    && install -d -o govee -g govee /seed-data

# crt-static off because btleplug links libdbus and Alpine has no static
# libdbus; the runtime stages install the matching shared libs. RUSTFLAGS and
# --target must match between cook and the final build or cook is wasted.
COPY --from=planner /src/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    target="$(cat /tmp/rust-target)" \
    && RUSTFLAGS="-C target-feature=-crt-static" \
       cargo chef cook --release --recipe-path recipe.json --target "${target}"

# declared after cook so a new release tag or sha doesn't bust the dep cache
ARG GOVEE2MQTT_RELEASE_TAG=""
ARG GOVEE2MQTT_BUILD_SHA=""
ENV GOVEE2MQTT_RELEASE_TAG=${GOVEE2MQTT_RELEASE_TAG}
ENV GOVEE2MQTT_BUILD_SHA=${GOVEE2MQTT_BUILD_SHA}

COPY . .
COPY --from=ui-builder /ui/dist ./ui/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    target="$(cat /tmp/rust-target)" \
    && RUSTFLAGS="-C target-feature=-crt-static" \
       cargo build --release --bin govee2mqtt --target "${target}" \
    && cp "target/${target}/release/govee2mqtt" /govee2mqtt

FROM alpine:3.21 AS standalone

# libdbus-1 for btleplug, libgcc_s for rust unwinding
RUN apk add --no-cache dbus-libs libgcc

COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group

WORKDIR /app

COPY --from=builder /govee2mqtt /app/govee2mqtt
COPY --from=builder --chown=govee:govee /seed-data /data

USER govee:govee
ENV \
  RUST_BACKTRACE=full \
  PATH=/app:$PATH \
  XDG_CACHE_HOME=/data

VOLUME /data

CMD ["/app/govee2mqtt", \
  "serve", \
  "--govee-iot-key=/data/iot.key", \
  "--govee-iot-cert=/data/iot.cert"]

FROM ghcr.io/home-assistant/${BUILD_ARCH}-base:3.21 AS addon

RUN apk add --no-cache dbus-libs libgcc

COPY common/run.sh /run.sh
COPY --from=builder /govee2mqtt /app/govee2mqtt

CMD [ "/run.sh" ]
