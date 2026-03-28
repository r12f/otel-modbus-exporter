# Docker Specification

## Multi-stage Dockerfile

### Build Stage

```dockerfile
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
RUN cargo build --release
```

### Runtime Stage

Uses [Google's distroless](https://github.com/GoogleContainerTools/distroless) base image for a minimal, secure runtime with no shell or package manager. The `nonroot` tag runs as a non-root user by default. Since the binary is statically linked (musl), no additional system libraries are needed.

```dockerfile
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /src/target/release/bus-exporter /usr/local/bin/
ENTRYPOINT ["bus-exporter"]
CMD ["--config", "/etc/bus-exporter/config.yaml"]
```

## Multi-arch Build

- Supported architectures: `linux/amd64`, `linux/arm64`
- Built using `docker buildx`:

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t r12f/bus-exporter:latest \
  --push .
```

## Config Mount Point

- Default config path inside container: `/etc/bus-exporter/config.yaml`
- Mount via: `-v /host/path/config.yaml:/etc/bus-exporter/config.yaml:ro`

## Serial Device Access

- For RTU collectors, pass the serial device: `--device /dev/ttyUSB0:/dev/ttyUSB0`
- May require `--privileged` or appropriate device cgroup rules.

## Health Check

- If Prometheus exporter is enabled, use it as a health check:

  ```dockerfile
  HEALTHCHECK CMD wget -q -O /dev/null http://localhost:9090/metrics || exit 1
  ```
