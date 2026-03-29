# Project Structure Specification

## File Tree

```text
bus-exporter/
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── publish.yml
├── Cargo.toml
├── Dockerfile
├── LICENSE
├── Makefile
├── README.md
├── config/
│   ├── example.yaml
│   ├── test.yaml
│   └── devices/
│       └── sdm630.yaml
├── spec/
│   ├── ci.md
│   ├── cli.md
│   ├── collector.md
│   ├── config.md
│   ├── decoder.md
│   ├── docker.md
│   ├── e2e-testing.md
│   ├── export-mqtt.md
│   ├── export-otlp.md
│   ├── export-prometheus.md
│   ├── exporter.md
│   ├── i2c.md
│   ├── i3c.md
│   ├── internal-metrics.md
│   ├── logging.md
│   ├── metrics.md
│   ├── modbus.md
│   ├── project-structure.md
│   ├── publish.md
│   ├── reader.md
│   ├── spi.md
│   └── testing.md
├── src/
│   ├── main.rs
│   ├── main_tests.rs
│   ├── lib.rs
│   ├── config.rs
│   ├── config_tests.rs
│   ├── collector.rs
│   ├── collector_tests.rs
│   ├── install.rs
│   ├── internal_metrics.rs
│   ├── internal_metrics_tests.rs
│   ├── logging.rs
│   ├── logging_tests.rs
│   ├── metrics.rs
│   ├── metrics_tests.rs
│   ├── pull.rs
│   ├── reader/
│   │   ├── mod.rs              # MetricReader trait, MetricReaderFactory
│   │   ├── decoder.rs          # Register/byte decoding
│   │   ├── decoder_tests.rs
│   │   ├── modbus/
│   │   │   ├── mod.rs          # Modbus MetricReader impl
│   │   │   ├── mod_tests.rs
│   │   │   ├── batch.rs        # Register coalescing
│   │   │   ├── batch/
│   │   │   │   └── batch_tests.rs
│   │   │   ├── tcp.rs          # TCP transport
│   │   │   ├── tcp_tests.rs
│   │   │   ├── rtu.rs          # RTU transport
│   │   │   └── rtu_tests.rs
│   │   ├── i2c/
│   │   │   ├── mod.rs
│   │   │   └── mod_tests.rs
│   │   ├── spi/
│   │   │   ├── mod.rs
│   │   │   └── mod_tests.rs
│   │   └── i3c/
│   │       ├── mod.rs
│   │       └── mod_tests.rs
│   └── exporter/
│       ├── mod.rs
│       ├── otlp/
│       │   ├── mod.rs
│       │   └── mod_tests.rs
│       ├── prometheus/
│       │   ├── mod.rs
│       │   └── mod_tests.rs
│       └── mqtt/
│           ├── mod.rs
│           └── mod_tests.rs
├── tests/
│   ├── integration_test.rs
│   ├── e2e_modbus.rs
│   └── e2e_i3c.rs
└── assets/
    ├── logo.svg
    └── logo.png
```

## Module Dependency Graph

```text
main
├── config
├── logging
├── pull
├── install
├── collector
│   ├── reader (MetricReader trait + MetricReaderFactory)
│   │   ├── reader::decoder
│   │   ├── reader::modbus (tcp, rtu, batch)
│   │   ├── reader::i2c
│   │   ├── reader::spi
│   │   └── reader::i3c
│   └── metrics
├── internal_metrics
├── exporter::otlp
│   └── metrics
├── exporter::prometheus
│   └── metrics
└── exporter::mqtt
    └── metrics
```
