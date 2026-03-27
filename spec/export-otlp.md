# OTLP Export Specification

## Overview

Exports metrics to an OpenTelemetry Collector via OTLP protobuf over HTTP (POST to `/v1/metrics`).

**The OTLP exporter is a pure consumer.** It reads exclusively from the in-memory `MetricStore` (which aggregates per-collector caches). It never triggers Modbus calls or interacts with collectors directly.

## Protocol

- Transport: HTTP/1.1
- Content-Type: `application/x-protobuf`
- Endpoint: configured `endpoint` + `/v1/metrics`
- Additional headers from config (e.g., Authorization).

## Batching Strategy

- Export runs on a fixed interval (10s default, tied to the fastest collector interval).
- Each export sends ALL current metric values in a single `ExportMetricsServiceRequest`.
- One `ResourceMetrics` with resource attributes from `global_labels`.
- One `ScopeMetrics` with scope name `modbus-exporter`.
- One `Metric` entry per metric, containing a single data point.

## Metric Mapping

| Internal Type | OTLP Type | Temporality |
|---------------|-----------|-------------|
| Gauge | Gauge | N/A |
| Counter | Sum | Cumulative, monotonic |

Each data point includes:
- `time_unix_nano`: timestamp of last poll
- `attributes`: merged labels
- `value`: as double

## Retry with Backoff

- On HTTP error (5xx, timeout, connection refused): retry with backoff.
- Backoff: 1s → 2s → 4s → max 30s.
- On 4xx (except 429): log error, do not retry.
- On 429: respect `Retry-After` header if present.
- Reset backoff after successful export.

## Configuration

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | bool | false | Enable OTLP export |
| `endpoint` | string | — | Base URL of the OTLP collector |
| `timeout` | duration | 10s | HTTP request timeout |
| `headers` | map | {} | Additional HTTP headers |
