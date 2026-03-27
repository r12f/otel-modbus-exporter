# Project Structure Specification

## Planned File Tree

```
modbus-exporter/
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── publish.yml
├── .pre-commit-config.yaml
├── Cargo.toml
├── Dockerfile
├── LICENSE
├── Makefile
├── README.md
├── config.yaml                  # Example config
├── config/
│   ├── test.yaml                # Exporter config for E2E tests
│   └── modbus-simulator.json    # Simulator register config for E2E tests
├── docker-compose.test.yml      # E2E test compose stack
├── spec/
│   ├── ci.md
│   ├── collector.md
│   ├── config.md
│   ├── decoder.md
│   ├── docker.md
│   ├── export-otlp.md
│   ├── export-prometheus.md
│   ├── logging.md
│   ├── metrics.md
│   ├── modbus.md
│   ├── project-structure.md
│   ├── publish.md
│   ├── testing.md
│   └── e2e-testing.md
├── src/
│   ├── main.rs                  # CLI entry point, config loading, task orchestration
│   ├── main_tests.rs
│   ├── config.rs                # Config structs, YAML deserialization, validation
│   ├── config_tests.rs
│   ├── modbus/
│   │   ├── mod.rs               # ModbusClient trait
│   │   ├── mod_tests.rs
│   │   ├── tcp.rs               # TCP client impl
│   │   ├── tcp_tests.rs
│   │   ├── rtu.rs               # RTU client impl
│   │   └── rtu_tests.rs
│   ├── decoder.rs               # Byte order reordering, type conversion, scale/offset
│   ├── decoder_tests.rs
│   ├── logging.rs               # Tracing subscriber init, output layer setup
│   ├── logging_tests.rs
│   ├── collector.rs             # Poll engine, per-collector async task
│   ├── collector_tests.rs
│   ├── metrics.rs               # MetricStore, MetricKey, MetricValue
│   ├── metrics_tests.rs
│   ├── export/
│   │   ├── mod.rs               # Export trait and shared types
│   │   ├── otlp.rs              # OTLP protobuf/HTTP exporter
│   │   ├── otlp_tests.rs
│   │   ├── prometheus.rs        # Prometheus /metrics HTTP server
│   │   └── prometheus_tests.rs
└── tests/
    ├── integration_test.rs      # End-to-end with mock Modbus server
    └── e2e/
        └── run.sh               # E2E test script (docker-compose based)
```

## Module Dependency Graph

```
main
├── config
├── logging
├── collector
│   ├── modbus (modbus::tcp, modbus::rtu)
│   ├── decoder
│   └── metrics
├── export::otlp
│   └── metrics
└── export::prometheus
    └── metrics
```
