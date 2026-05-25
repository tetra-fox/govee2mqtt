ARG BUILD_FROM
FROM ghcr.io/tetra-fox/govee2mqtt:latest AS govee2mqtt
FROM $BUILD_FROM
COPY run.sh /run.sh
COPY --from=govee2mqtt /app/govee2mqtt /app/
COPY --from=govee2mqtt /app/assets /app/assets/
CMD [ "/run.sh" ]
