# CLI Subcommands

## Overview

`bus-exporter` supports three modes of operation via subcommands:

- **`run`** (default) — Start as a daemon, continuously polling collectors and exporting metrics.
- **`pull`** — Single-shot read: connect, read metrics once, print JSON, exit.
- **`install`** — Install bus-exporter as a system service (systemd).

When no subcommand is given, `run` is assumed (backward-compatible).

## Global Options

```
bus-exporter [OPTIONS] <COMMAND>

Options:
  -c, --config <PATH>    Path to configuration file
  -h, --help             Print help
  -V, --version          Print version
```

## `pull` Subcommand

Single-shot metric read. Connects to devices, reads once, prints JSON to stdout, exits.

```
bus-exporter pull [OPTIONS]

Options:
  --collector <REGEX>    Filter collectors by name (regex, partial match)
  --metric <REGEX>       Filter metrics by name (regex, partial match)
```

### Behavior

1. Load config file (same search path as `run`).
2. Filter collectors: if `--collector` is set, keep only collectors whose name matches the regex.
3. For matching collectors, filter metrics: if `--metric` is set, keep only metrics whose name matches the regex.
4. If no collectors or metrics remain after filtering, exit with error.
5. For each matching collector:
   a. Create reader via `MetricReaderFactory`.
   b. Call `set_metrics()` with filtered metrics.
   c. Call `connect()`.
   d. Call `read()` once.
   e. Call `disconnect()`.
6. Print JSON to stdout.
7. Exit code 0 if all reads succeed, 1 if any read fails.

### Regex Behavior

- Uses the `regex` crate.
- Partial match (contains semantics) — pattern `volt` matches `voltage_l1`.
- Case-sensitive by default. User can use `(?i)` prefix for case-insensitive.
- Invalid regex → exit with error message immediately.

### JSON Output Format

```json
{
  "collectors": [
    {
      "name": "sdm630",
      "protocol": "modbus-tcp",
      "metrics": [
        {
          "name": "voltage_l1",
          "value": 230.5,
          "raw_value": 2305,
          "error": null
        },
        {
          "name": "voltage_l2",
          "value": null,
          "raw_value": null,
          "error": "connection refused"
        }
      ]
    }
  ],
  "summary": {
    "total_collectors": 1,
    "total_metrics": 2,
    "successful": 1,
    "failed": 1
  }
}
```

Field definitions:
- `name` — Metric name from config.
- `value` — Scaled value (`raw * scale + offset`), `null` on error.
- `raw_value` — Raw value before scale/offset, `null` on error.
- `error` — Error message string, `null` on success.
- `summary` — Aggregated counts for quick status check.

### Logging

- Logs go to stderr (same logging config as `run`).
- JSON output goes to stdout only.
- This allows `bus-exporter pull 2>/dev/null` for clean JSON.

## `install` Subcommand

Install bus-exporter as a systemd service.

```
bus-exporter install [OPTIONS]

Options:
  --user              Install as user service (systemctl --user) instead of system service
  --config <PATH>     Config file path to embed in service file (default: /etc/bus-exporter/config.yaml)
  --bin <PATH>        Path to bus-exporter binary (default: auto-detect from current executable)
  --uninstall         Remove the service instead of installing
```

### Behavior

1. Generate a systemd unit file from template.
2. Write to `/etc/systemd/system/bus-exporter.service` (system) or `~/.config/systemd/user/bus-exporter.service` (user).
3. Run `systemctl daemon-reload`.
4. Run `systemctl enable bus-exporter`.
5. Print instructions to start: `systemctl start bus-exporter`.

### Systemd Unit Template

```ini
[Unit]
Description=Bus Exporter - Industrial bus metrics exporter
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={bin_path} run -c {config_path}
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=bus-exporter

[Install]
WantedBy=multi-user.target
```

### `--uninstall` Behavior

1. Run `systemctl stop bus-exporter` (ignore if not running).
2. Run `systemctl disable bus-exporter`.
3. Remove the unit file.
4. Run `systemctl daemon-reload`.

### Platform Check

- If not on Linux or systemd is not available, print error and exit.
- Future: support other init systems (OpenRC, launchd) via `--type` flag.

## `run` Subcommand

Default behavior — start as daemon. No changes from current behavior.

```
bus-exporter run [OPTIONS]
```

This is the implicit default when no subcommand is given.

## Implementation Notes

### CLI Structure (clap)

```rust
#[derive(Parser)]
#[command(name = "bus-exporter", version, about)]
struct Cli {
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start as daemon (default)
    Run,
    /// Single-shot metric read
    Pull {
        /// Filter collectors by name (regex)
        #[arg(long)]
        collector: Option<String>,
        /// Filter metrics by name (regex)
        #[arg(long)]
        metric: Option<String>,
    },
    /// Install as system service
    Install {
        /// Install as user service
        #[arg(long)]
        user: bool,
        /// Config file path for service
        #[arg(long)]
        config: Option<PathBuf>,
        /// Binary path for service
        #[arg(long)]
        bin: Option<PathBuf>,
        /// Remove service instead of installing
        #[arg(long)]
        uninstall: bool,
    },
}
```

### Dependencies

- `regex` crate (add to Cargo.toml)
- `serde_json` for pull output (already a transitive dep, add as direct)

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success (all reads OK for pull) |
| 1 | Partial failure (some reads failed for pull) |
| 2 | Fatal error (bad config, bad regex, no matches) |
