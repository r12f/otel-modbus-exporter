# MQTT Export Specification

## Overview

The MQTT exporter publishes metric values to an MQTT broker. Each metric is published as a plain value to a topic derived from the collector and metric names.

## Topic Structure

```
<topic_prefix>/<collector_name>/<metric_name>
```

Example: `modbus/metrics/power-meter-01/voltage_phase_a`

### Status Topic

```
<topic_prefix>/status
```

Publishes `online` on connect, `offline` as Last Will & Testament (LWT) on unexpected disconnect. Both published with `retain: true` regardless of the global `retain` setting.

## Payload Format

Plain value as UTF-8 string:

```
230.5
```

- Floating-point values use standard decimal notation (no trailing zeros forced).
- Boolean values: `0` or `1`.

## Publish Behavior

The MQTT exporter reads from the metric cache on a configurable interval (same model as OTLP). It iterates all cached metrics and publishes each to its topic.

- Publish interval is independent of collector polling intervals.
- Only the latest cached value is published (no history/buffering of intermediate polls).
- If the cache is empty (no data yet), nothing is published.

## Connection Management

### Initial Connection

Connect to the broker on startup. If the broker is unavailable, retry with exponential backoff (1s initial, 2× multiplier, 60s cap).

### Reconnect

On unexpected disconnect, reconnect with the same exponential backoff. During disconnect:

- Publish attempts are **dropped** (not buffered) to avoid unbounded memory growth.
- On successful reconnect, publishes resume from the next cache read cycle.

### Last Will & Testament (LWT)

Set at connect time:

| Field | Value |
|-------|-------|
| Topic | `<topic_prefix>/status` |
| Payload | `offline` |
| QoS | Same as configured `qos` |
| Retain | `true` |

On successful connect, publish `online` to the same topic with `retain: true`.

## TLS

When `endpoint` uses `mqtts://`:

- `tls.ca_cert` — CA certificate for server verification (optional; uses system roots if omitted).
- `tls.client_cert` + `tls.client_key` — for mutual TLS (optional).
- `tls.insecure` — skip server certificate verification (default `false`).

## Configuration

See [config.md](config.md#exportersmqtt) for the full configuration schema.

## Error Handling

- Connection errors → log at `warn`, retry with backoff.
- Publish errors → log at `warn`, continue to next metric.
- Auth failures → log at `error`, retry with backoff (broker may recover).

## Crate

Use [`rumqttc`](https://crates.io/crates/rumqttc) — async MQTT 3.1.1/5 client, well-maintained, works with tokio.
