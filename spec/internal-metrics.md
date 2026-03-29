# Internal Metrics Specification

## Overview

The exporter itself collects and exports internal operational metrics alongside device metrics. These metrics provide observability into the exporter's health, performance, and error rates.

Internal metrics are exported through the same channels as device metrics — both Prometheus scrape endpoint and OTLP export.

## Metric Prefix

All internal metrics use the prefix `bus_exporter_` to distinguish them from device metrics (which use `bus_`).

## Internal Metrics

### Collector Metrics

| Metric Name | Type | Labels | Description |
|---|---|---|---|
| `bus_exporter_collectors_total` | Gauge | — | Total number of configured collectors |
| `bus_exporter_polls_total` | Counter | `collector` | Total number of poll cycles executed per collector |
| `bus_exporter_polls_success_total` | Counter | `collector` | Number of fully successful poll cycles (all metrics read) |
| `bus_exporter_polls_error_total` | Counter | `collector` | Number of poll cycles with at least one metric read failure |
| `bus_exporter_modbus_requests_total` | Counter | `collector` | Total number of individual read requests |
| `bus_exporter_modbus_errors_total` | Counter | `collector` | Total number of failed read requests |
| `bus_exporter_poll_duration_seconds` | Gauge | `collector` | Duration of the last poll cycle in seconds |

### Export Metrics

| Metric Name | Type | Labels | Description |
|---|---|---|---|
| `bus_exporter_otlp_exports_total` | Counter | — | Total number of OTLP export attempts |
| `bus_exporter_otlp_errors_total` | Counter | — | Total number of failed OTLP exports |
| `bus_exporter_prometheus_scrapes_total` | Counter | — | Total number of Prometheus scrape requests served |

### Uptime

| Metric Name | Type | Labels | Description |
|---|---|---|---|
| `bus_exporter_uptime_seconds` | Gauge | — | Seconds since the exporter process started |

## Implementation

### `InternalMetrics` Struct

A shared `InternalMetrics` struct holds all counters and gauges using `AtomicU64` / `AtomicF64` values. It is wrapped in `Arc` and passed to all collectors, exporters, and the Prometheus handler.

```rust
pub struct InternalMetrics {
    pub start_time: Instant,
    pub collectors_total: AtomicU64,
    pub collector_stats: DashMap<String, CollectorStats>,
    pub otlp_exports_total: AtomicU64,
    pub otlp_errors_total: AtomicU64,
    pub prometheus_scrapes_total: AtomicU64,
}

pub struct CollectorStats {
    pub polls_total: AtomicU64,
    pub polls_success: AtomicU64,
    pub polls_error: AtomicU64,
    pub read_requests: AtomicU64,
    pub read_errors: AtomicU64,
    /// Stored as `f64` bits via `AtomicU64` (`f64::to_bits` / `f64::from_bits`).
    pub last_poll_duration_secs: AtomicU64,
}
```

### Integration Points

1. **Collector poll loop** — At start of each poll cycle, increment `polls_total`. After completion, increment `polls_success` or `polls_error`. For each read, increment `read_requests` (and `read_errors` on failure). Record poll duration.

2. **OTLP exporter** — Increment `otlp_exports_total` on each export attempt, `otlp_errors_total` on failure.

3. **Prometheus handler** — Increment `prometheus_scrapes_total` on each `/metrics` request. Append internal metrics to the response after device metrics.

4. **Startup** — Set `collectors_total` to the number of configured collectors.

### Prometheus Output

Internal metrics are appended after all device metrics in the `/metrics` response, separated by a blank line:

```text
# HELP bus_exporter_collectors_total Total number of configured collectors
# TYPE bus_exporter_collectors_total gauge
bus_exporter_collectors_total 3

# HELP bus_exporter_polls_total Total poll cycles per collector
# TYPE bus_exporter_polls_total counter
bus_exporter_polls_total{collector="meter_1"} 42
bus_exporter_polls_total{collector="meter_2"} 40

# HELP bus_exporter_uptime_seconds Seconds since exporter started
# TYPE bus_exporter_uptime_seconds gauge
bus_exporter_uptime_seconds 3600.5
```

### OTLP Output

Internal metrics are sent as a separate `ScopeMetrics` with scope name `bus-exporter-internal` within the same OTLP export request. They follow the same encoding (protobuf) and export schedule as device metrics.

## Configuration

Internal metrics are always enabled — no configuration toggle. They have negligible overhead (atomic counters only) and are essential for production observability.

## Testing

- Unit test: verify `InternalMetrics` counters increment correctly.
- Unit test: verify Prometheus output includes internal metrics with correct names, types, and labels.
- Integration test: verify internal metrics appear alongside device metrics.
- E2E test: add assertions for `bus_exporter_collectors_total` and `bus_exporter_uptime_seconds` in the E2E test script.
