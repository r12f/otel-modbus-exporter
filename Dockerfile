# Multi-stage Dockerfile for bus-exporter
# Supports linux/amd64 and linux/arm64

FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /src/target/release/bus-exporter /usr/local/bin/
EXPOSE 9090
HEALTHCHECK NONE
ENTRYPOINT ["bus-exporter"]
CMD ["--config", "/etc/bus-exporter/config.yaml"]
