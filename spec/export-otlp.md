# OTLP Export Specification

## Overview

Exports metrics to an OpenTelemetry Collector via the `opentelemetry-otlp` SDK over HTTP. The exporter uses the official `opentelemetry-sdk` pipeline — no hand-crafted protobuf encoding.

**The OTLP exporter is a pure consumer.** It reads exclusively from the in-memory `MetricStore` (which aggregates per-collector caches). It never triggers bus calls or interacts with collectors directly.

## SDK Pipeline

The exporter builds an `SdkMeterProvider` wired to a `PeriodicReader` with an OTLP HTTP metric exporter:

1. **`opentelemetry_otlp::MetricExporter`** — HTTP exporter configured with endpoint, headers, and timeout from config.
2. **`PeriodicReader`** — Drives collection on a configurable interval.
3. **`SdkMeterProvider`** — Hosts the meter and manages the pipeline lifecycle.
4. **`Resource`** — Built from `global_labels` config (key-value attributes).

## Observable Instruments

Metrics are exposed to the SDK via **observable instruments** (callback-based):

| Internal Type | OTel Instrument | Notes |
|---------------|-----------------|-------|
| Gauge | `f64_observable_gauge` | Callback reports current absolute value |
| Counter | `f64_observable_counter` | Callback reports cumulative total; SDK computes deltas internally |

Instruments are registered lazily — as new metric names appear, they are registered once and cached. The callbacks read from shared state (`Arc<RwLock<Vec<MetricValue>>>`) which the main loop updates each interval.

## Export Flow

1. Each interval tick, the exporter reads all metrics from `MetricStore`.
2. Registers observable instruments for any newly-discovered metric names.
3. Updates the shared state that observable callbacks read from.
4. The `PeriodicReader` handles actual export scheduling, serialization, and transmission — no manual `force_flush` is needed in the main loop.
5. On shutdown, a final state update + `provider.shutdown()` ensures the last values are flushed.

Internal metrics are exported under a separate scope (`bus-exporter-internal`).

## Retry Behavior

Retry behavior is delegated to the OpenTelemetry SDK defaults (exponential backoff with jitter: 5 second initial interval, maximum 30 second interval, up to 5 retry attempts — per the OTLP exporter specification). The exporter does not implement custom retry logic. The SDK's built-in retry and timeout handling applies.

## Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | false | Enable OTLP export |
| `endpoint` | string | — | Base URL of the OTLP collector |
| `timeout` | duration | 10s | HTTP request timeout |
| `headers` | map | {} | Additional HTTP headers |
