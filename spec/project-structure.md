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
│   ├── commands/
│   │   ├── mod.rs              # Re-exports install, pull, run, watch; shared helpers (filter_collectors, collect_once)
│   │   ├── install.rs          # systemd install/uninstall
│   │   ├── pull.rs             # One-shot metric pull
│   │   ├── run.rs              # Daemon entry point, logging mapping, shutdown
│   │   └── watch.rs            # Continuous metric watch (NDJSON loop)
│   ├── config.rs
│   ├── config_tests.rs
│   ├── collector.rs
│   ├── collector_tests.rs
│   ├── internal_metrics.rs
│   ├── internal_metrics_tests.rs
│   ├── logging.rs
│   ├── logging_tests.rs
│   ├── metrics.rs
│   ├── metrics_tests.rs
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
│   ├── common/
│   │   └── mod.rs
│   ├── e2e_modbus.rs
│   ├── e2e_modbus_rtu.rs
│   ├── e2e_i2c.rs
│   ├── e2e_spi.rs
└── assets/
    ├── logo.svg
    └── logo.png
```

## Module Dependency Graph

```text
main
├── config
├── logging
├── commands
│   ├── install
│   ├── pull
│   ├── run
│   └── watch
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
