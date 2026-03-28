<p align="center">
  <img src="assets/logo.png" alt="bus-exporter logo" width="200">
</p>

# bus-exporter

A hardware bus metrics exporter that polls Modbus RTU/TCP, I2C, SPI, and I3C devices and exports metrics via OTLP (protobuf/HTTP), Prometheus scrape endpoint, and MQTT.

## Installation

### Cargo

```bash
cargo install bus-exporter
```

### Docker

```bash
docker run -d \
  -v /path/to/config.yaml:/etc/bus-exporter/config.yaml:ro \
  -p 9090:9090 \
  --device /dev/ttyUSB0:/dev/ttyUSB0 \
  r12f/bus-exporter:latest
```

For TCP-only collectors, the `--device` flag is not needed.

Multi-arch images available: `linux/amd64` and `linux/arm64`. See the [Docker spec](spec/docker.md) for details.

## Features

- **[Modbus RTU & TCP](spec/modbus.md)** — poll devices over serial (RS-485/RS-232) or Ethernet
- **[I2C](spec/i2c.md)** — read sensors and peripherals on I2C buses (Linux)
- **[SPI](spec/spi.md)** — read ADCs and peripherals via SPI (Linux)
- **[I3C](spec/i3c.md)** — read sensors on I3C buses with dynamic addressing (Linux)
- **[Flexible decoding](spec/decoder.md)** — u16, i16, u32, i32, f32, u64, i64, f64, bool with configurable byte order and scale/offset transform
- **[OTLP export](spec/export-otlp.md)** — push metrics to any OpenTelemetry Collector via protobuf/HTTP
- **[Prometheus export](spec/export-prometheus.md)** — built-in `/metrics` HTTP endpoint for pull-based scraping
- **[MQTT export](spec/export-mqtt.md)** — publish metric values to an MQTT broker
- **[Per-collector polling](spec/collector.md)** — independent async tasks with configurable intervals and automatic reconnect
- **[Reusable metric definitions](spec/metrics.md)** — share metric files across collectors with defaults and override support
- **[Internal metrics](spec/internal-metrics.md)** — self-observability: poll counts, error rates, uptime, and more
- **[Structured logging](spec/logging.md)** — configurable log levels with tracing
- **[CI/CD](spec/ci.md)** — lint, test, E2E, and [publish](spec/publish.md) pipelines

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   Collector                      │
│         (async polling + caching)                │
│                      │                           │
│              MetricReader trait                   │
│         ┌────┬───┬───┬────┐                      │
│         │    │   │   │    │                      │
│       Modbus I2C SPI I3C ...                     │
│      TCP/RTU                                     │
└──────────────────┬──────────────────────────────┘
                   │ cached metrics
       ┌───────────┼───────────┐
       │           │           │
   Exporter    Exporter    Exporter
    (OTLP)   (Prometheus)  (MQTT)
```

Each protocol implements the `MetricReader` trait (`read` + optional `batch_read`). The collector calls readers, caches results, and exporters read from cache. See protocol specs for details: [Modbus](spec/modbus.md) · [I2C](spec/i2c.md) · [SPI](spec/spi.md) · [I3C](spec/i3c.md).

## Configuration

See the full [Configuration Specification](spec/config.md) for all options, validation rules, and metrics file format.

```yaml
global_labels:
  environment: "production"
  site: "factory-01"

exporters:
  otlp:
    enabled: true
    endpoint: "http://otel-collector:4318"
  prometheus:
    enabled: true
    listen: "0.0.0.0:9090"

collectors:
  - name: "power-meter"
    protocol:
      type: modbus-tcp
      endpoint: "192.168.1.100:502"
    slave_id: 1
    polling_interval: "5s"
    metrics_files:
      - "devices/sdm630.yaml"
    metrics:
      - name: "voltage_phase_a"
        type: gauge
        register_type: holding
        address: 0
        data_type: u16
        scale: 0.1
        unit: "V"
```

See [`config/example.yaml`](config/example.yaml) for a complete annotated example.

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

## License

Apache-2.0 — see [LICENSE](LICENSE).
