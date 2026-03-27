# Multi-stage Dockerfile for otel-modbus-exporter
# Supports linux/amd64 and linux/arm64

FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release

FROM alpine:latest
RUN apk add --no-cache ca-certificates
COPY --from=builder /src/target/release/otel-modbus-exporter /usr/local/bin/
COPY config/example.yaml /etc/otel-modbus-exporter/config.yaml
EXPOSE 9090
HEALTHCHECK CMD wget -q -O /dev/null http://localhost:9090/metrics || exit 1
ENTRYPOINT ["otel-modbus-exporter"]
CMD ["--config", "/etc/otel-modbus-exporter/config.yaml"]
