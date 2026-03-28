# MetricExporter Specification

> Part of the [bus-exporter architecture](../README.md#architecture).

## Overview

`MetricExporter` is the common trait for all exporters. Each exporter receives metric configs and cached read results, then formats and sends them.

## Trait

```rust
#[async_trait]
pub trait MetricExporter: Send {
    /// Export cached metric results.
    async fn export(
        &mut self,
        metrics: &[MetricConfig],
        results: &HashMap<String, Result<f64>>,
    ) -> Result<()>;

    /// Graceful shutdown.
    async fn shutdown(&mut self) -> Result<()>;
}
```

## Factory

```rust
pub fn create_exporters(config: &ExportersConfig) -> Result<Vec<Box<dyn MetricExporter>>>;
```

## Implementations

| Exporter | Spec |
|----------|------|
| OTLP | [export-otlp.md](export-otlp.md) |
| Prometheus | [export-prometheus.md](export-prometheus.md) |
| MQTT | [export-mqtt.md](export-mqtt.md) |

## Source Layout

```
src/exporter/
  mod.rs              — MetricExporter trait, MetricExporterFactory
  otlp.rs             — OTLP protobuf/HTTP
  prometheus.rs       — /metrics scrape endpoint
  mqtt.rs             — MQTT publisher
```
