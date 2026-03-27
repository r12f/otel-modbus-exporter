# Multi-stage Dockerfile for modbus-exporter
# Supports linux/amd64 and linux/arm64

FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release

FROM alpine:3.20
RUN apk add --no-cache ca-certificates
COPY --from=builder /src/target/release/modbus-exporter /usr/local/bin/
EXPOSE 9090
HEALTHCHECK NONE
ENTRYPOINT ["modbus-exporter"]
CMD ["--config", "/etc/modbus-exporter/config.yaml"]
