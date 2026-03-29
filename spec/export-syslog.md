# Syslog Log Export Specification

> Part of the [bus-exporter logging system](logging.md).

## Overview

bus-exporter can send its tracing logs (not metric values) to the local Linux syslog daemon via a Unix domain socket (`/dev/log`). This is implemented as a `tracing_subscriber::Layer` using the [`syslog`](https://crates.io/crates/syslog) crate (v7).

**This is a log output target, not a metric exporter.** It controls where `tracing` events (INFO, WARN, ERROR, etc.) are delivered. Metric values continue to flow through the metric exporters (OTLP, Prometheus, MQTT).

## Transport

- **Unix socket only** (`/dev/log`) — no UDP or TCP.
- Uses the `syslog::unix()` constructor from the `syslog` crate.
- If the Unix socket is unavailable (e.g., macOS, containerized environment without `/dev/log`), initialization falls back to stderr with a warning printed to stderr.

## Message Format

Messages use **RFC 3164** (BSD syslog) format via `syslog::Formatter3164`:

| Field | Value |
|-------|-------|
| `facility` | Configurable (default: `daemon`) |
| `hostname` | `None` (system-assigned) |
| `process` | `"bus-exporter"` |
| `pid` | Current process ID |

### Severity Mapping

| tracing Level | Syslog Severity |
|---------------|-----------------|
| `ERROR` | `err` (3) |
| `WARN` | `warning` (4) |
| `INFO` | `info` (6) |
| `DEBUG` | `debug` (7) |
| `TRACE` | `debug` (7) |

### Message Body

The message body is constructed from tracing event fields:

1. The `message` field is placed first.
2. Additional fields are appended as `key=value` pairs separated by spaces.
3. The target (module path) is prepended: `target: message key1=value1 key2=value2`.

Example syslog message:

```text
bus_exporter::collector: poll completed collector="power-meter" metrics_count=5 duration_ms=42
```

## Configuration

Syslog output is configured in the `logging` section of `config.yaml`:

```yaml
logging:
  level: "info"
  output: "syslog"
  syslog_facility: "daemon"    # optional, default: daemon
```

### Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `output` | `string` | No | `"syslog"` | Set to `"syslog"` to enable native syslog output |
| `syslog_facility` | `string` | No | `"daemon"` | Syslog facility for messages |

### Supported Facilities

| Value | syslog Facility |
|-------|-----------------|
| `"daemon"` | `LOG_DAEMON` (default) |
| `"local0"` | `LOG_LOCAL0` |
| `"local1"` | `LOG_LOCAL1` |
| `"local2"` | `LOG_LOCAL2` |
| `"local3"` | `LOG_LOCAL3` |
| `"local4"` | `LOG_LOCAL4` |
| `"local5"` | `LOG_LOCAL5` |
| `"local6"` | `LOG_LOCAL6` |
| `"local7"` | `LOG_LOCAL7` |

## Fallback Behavior

If the syslog Unix socket cannot be opened at startup:

1. A warning is printed to stderr: `warning: failed to connect to syslog (<error>), falling back to stderr`
2. Logging falls back to the `stderr` text output (same as `output: "stderr"`).
3. The process continues normally — syslog failure is **not** fatal.

## Implementation

The syslog layer is implemented as an inline module `syslog_layer` in `src/logging.rs`:

- `SyslogLayer` — holds a `Mutex<syslog::Logger<LoggerBackend, Formatter3164>>`
- Implements `tracing_subscriber::Layer<S>` with an `on_event` handler
- `FieldCollector` visitor extracts fields from tracing events into the message string

### Crate Dependencies

| Crate | Version | Role |
|-------|---------|------|
| `syslog` | 7 | Unix socket syslog client |

## Testing

### Unit Tests

- `LogOutput::from_str("syslog")` parses correctly.
- `SyslogFacility` defaults to `Daemon`.
- `map_syslog_facility` maps all variants correctly.
- Default `LoggingConfig` uses `Syslog` output.

### E2E Tests

- Start bus-exporter with `output: "syslog"` in a Linux container with syslog available.
- Verify log messages appear in `/var/log/syslog` or via `journalctl`.
- Verify graceful fallback when `/dev/log` is absent.
