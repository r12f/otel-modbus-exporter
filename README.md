# modbus-exporter

An OpenTelemetry-native Modbus exporter that polls Modbus RTU and TCP devices and exports metrics via OTLP (protobuf/HTTP) and Prometheus scrape endpoint.

## Features

- **Modbus RTU** over serial ports (RS-485/RS-232)
- **Modbus TCP** over Ethernet
- **OTLP export** — push metrics to any OpenTelemetry Collector via protobuf/HTTP
- **Prometheus export** — built-in `/metrics` HTTP endpoint for pull-based scraping
- **Flexible decoding** — supports u16, i16, u32, i32, f32, u64, i64, f64, bool with configurable byte order
- **Scale & offset** — linear transform: `value = raw * scale + offset`
- **Per-collector polling** — independent async tasks with configurable intervals
- **Global and per-collector labels** — hierarchical label merging
- **Reconnect with backoff** — automatic retry on connection failures
- **Docker-ready** — multi-arch images (amd64 + arm64)

## Configuration

Create a `config.yaml`:

```yaml
global_labels:
  environment: "production"
  site: "factory-01"

exporters:
  otlp:
    enabled: true
    endpoint: "http://otel-collector:4318"
    timeout: "10s"
    headers:
      Authorization: "Bearer xxx"
  prometheus:
    enabled: true
    listen: "0.0.0.0:9090"
    path: "/metrics"

collectors:
  - name: "power-meter-01"
    protocol:
      type: tcp
      endpoint: "192.168.1.100:502"
    slave_id: 1
    polling_interval: "5s"
    labels:
      building: "A"
      floor: "2"
    metrics:
      - name: "voltage_phase_a"
        description: "Phase A voltage"
        type: gauge
        register_type: holding
        address: 0x0000
        data_type: u16
        byte_order: big_endian
        scale: 0.1
        offset: 0.0
        unit: "V"
      - name: "total_energy"
        description: "Total energy consumption"
        type: counter
        register_type: holding
        address: 0x0048
        data_type: u32
        byte_order: mid_big_endian
        scale: 0.01
        offset: 0.0
        unit: "kWh"

  - name: "temp-sensor-rtu"
    protocol:
      type: rtu
      device: "/dev/ttyUSB0"
      bps: 9600
      data_bits: 8
      stop_bits: 1
      parity: "none"
    slave_id: 2
    polling_interval: "10s"
    labels:
      zone: "cold-storage"
    metrics:
      - name: "temperature"
        description: "Ambient temperature"
        type: gauge
        register_type: input
        address: 0x0001
        data_type: i16
        byte_order: big_endian
        scale: 0.1
        offset: -40.0
        unit: "°C"
```

## Build

Requires Rust 1.75+ and Make.

```bash
make build          # cargo build --release
make run            # cargo run -- --config config.yaml
make fmt            # cargo fmt
make lint           # cargo clippy -- -D warnings
make test           # cargo test
make docker         # docker buildx build (amd64 + arm64)
make e2e            # run E2E tests (docker-compose)
make clean          # cargo clean
```

## Docker

```bash
docker run -d \
  -v /path/to/config.yaml:/etc/modbus-exporter/config.yaml:ro \
  -p 9090:9090 \
  --device /dev/ttyUSB0:/dev/ttyUSB0 \
  r12f/modbus-exporter:latest
```

For TCP-only collectors, the `--device` flag is not needed.

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime |
| `tokio-modbus` | Modbus RTU/TCP client |
| `tokio-serial` | Serial port for RTU |
| `serde` / `serde_yaml` | Configuration parsing |
| `opentelemetry` / `opentelemetry-otlp` | OTLP metric export |
| `prometheus` | Prometheus metric exposition |
| `axum` | HTTP server for Prometheus endpoint |
| `tracing` | Structured logging |
| `tracing-syslog` | Syslog output layer for tracing |
| `clap` | CLI argument parsing |

## License

Apache-2.0 — see [LICENSE](LICENSE).
