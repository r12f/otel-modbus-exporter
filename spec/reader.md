# MetricReader Specification

> Part of the [bus-exporter architecture](../README.md#architecture).

## Overview

`MetricReader` is the common trait for all protocol readers. The collector uses it to read metrics from devices — it doesn't need to know the underlying protocol.

## Ownership Model

Each collector owns exactly one `MetricReader` instance. Readers are not shared across collectors — there is no concurrent access to a single reader. The collector calls `read()` or `batch_read()` sequentially within its polling loop, so `&mut self` is the correct receiver.

## Trait

```rust
#[async_trait]
pub trait MetricReader: Send + Sync {
    /// What this reader supports.
    fn capabilities(&self) -> ReaderCapabilities;

    /// Connect to the device/bus.
    async fn connect(&mut self) -> Result<()>;

    /// Disconnect.
    async fn disconnect(&mut self) -> Result<()>;

    /// Whether the reader is connected.
    fn is_connected(&self) -> bool;

    /// Read a single metric. Returns the decoded value.
    async fn read(&mut self, metric: &MetricConfig) -> Result<f64>;

    /// Batch read. Returns results in the same order as the input slice.
    /// Default implementation calls read() per metric.
    async fn batch_read(
        &mut self,
        metrics: &[MetricConfig],
    ) -> Vec<(&MetricConfig, Result<f64>)> {
        let mut results = Vec::with_capacity(metrics.len());
        for m in metrics {
            results.push((m, self.read(m).await));
        }
        results
    }
}

pub struct ReaderCapabilities {
    pub batch_read: bool,
}
```

**Error handling:** `Result` uses `anyhow::Result` — the collector logs errors and updates internal metrics (error counters). Transient errors (connection lost, timeout) trigger reconnect; permanent errors (invalid config) are logged once.

## Capabilities

Each reader reports what it supports via `capabilities()`. The collector checks this at runtime:

- If `config.batch_read == true` **and** `reader.capabilities().batch_read == true` → use `batch_read()`
- Otherwise → iterate with `read()`

## Config

```yaml
protocol:
  type: modbus-tcp
  endpoint: "192.168.1.100:502"
  batch_read: true  # optional, default: false
```

The `batch_read` field is available on all protocol types but only takes effect when the reader supports it.

## Implementations

| Protocol | `batch_read` support | Notes |
|----------|---------------------|-------|
| [Modbus TCP/RTU](modbus.md) | ✅ | Coalesces adjacent register ranges |
| [I2C](i2c.md) | ❌ | Single-device reads |
| [SPI](spi.md) | ❌ | Single-device reads |
| [I3C](i3c.md) | ❌ | Single-device reads |

## Source Layout

```
src/reader/
  mod.rs              — MetricReader trait, ReaderCapabilities
  modbus/
    mod.rs            — Modbus MetricReader impl
    tcp.rs            — TCP transport
    tcp_tests.rs
    rtu.rs            — RTU transport
    rtu_tests.rs
    mod_tests.rs
  i2c/
    mod.rs
    mod_tests.rs
  spi/
    mod.rs
    mod_tests.rs
  i3c/
    mod.rs
    mod_tests.rs
```

Renamed from previous layout: `src/modbus/`, `src/i2c/`, `src/spi/`, `src/i3c/` → `src/reader/*`.

The `src/export/` → `src/exporter/` rename is a separate structural change done in the same implementation PR.
