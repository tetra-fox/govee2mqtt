ARG BUILD_FROM
# the app image tag to wrap. the home-assistant builder injects BUILD_VERSION
# from the add-on's config.yaml version: "edge" for the dev add-on, the release
# version for the stable one. both resolve to a published govee2mqtt image tag
ARG BUILD_VERSION
FROM ghcr.io/tetra-fox/govee2mqtt:${BUILD_VERSION} AS govee2mqtt
FROM $BUILD_FROM
COPY run.sh /run.sh
COPY --from=govee2mqtt /app/govee2mqtt /app/
COPY --from=govee2mqtt /app/assets /app/assets/
CMD [ "/run.sh" ]
