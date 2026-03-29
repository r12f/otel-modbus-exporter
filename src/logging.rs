use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::str::FromStr;

use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

use crate::config;

// ── Config → logging mapping ──────────────────────────────────────────

/// Map the user-facing config::LoggingConfig to the internal LoggingConfig
/// used by the tracing subscriber.
pub fn map_logging_config(cfg: &config::LoggingConfig) -> LoggingConfig {
    let level = match cfg.level {
        config::LogLevel::Trace => "trace",
        config::LogLevel::Debug => "debug",
        config::LogLevel::Info => "info",
        config::LogLevel::Warn => "warn",
        config::LogLevel::Error => "error",
    }
    .to_string();

    let output = match cfg.output {
        config::LogOutput::Stdout => LogOutput::Stdout,
        config::LogOutput::Stderr => LogOutput::Stderr,
        // Syslog output is not yet implemented as a native syslog transport.
        // We map it to structured JSON as an interim solution, because JSON
        // is the closest machine-readable format and is easy to forward into
        // syslog-compatible collectors (e.g. Vector, Fluentd, journald).
        config::LogOutput::Syslog => LogOutput::Json,
    };

    LoggingConfig { level, output }
}

/// Logging output target.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogOutput {
    Stdout,
    Stderr,
    /// Structured JSON output to stderr.
    /// Real syslog support (via the `syslog` crate) is planned for a future release.
    Json,
}

impl FromStr for LogOutput {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "json" => Ok(Self::Json),
            other => Err(anyhow!("invalid log output: {other}")),
        }
    }
}

/// Logging configuration matching the `logging` section in config.yaml.
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub output: LogOutput,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            output: LogOutput::Json,
        }
    }
}

/// Initialize the tracing subscriber based on the provided logging configuration.
///
/// - `stdout` / `stderr`: Uses `tracing_subscriber::fmt` layer with the appropriate writer.
/// - `json`: Uses structured JSON format to stderr, suitable for systemd/journald capture.
///
/// Respects the `RUST_LOG` environment variable when set, falling back to the configured level.
///
/// Call once at startup before any tracing events are emitted.
pub fn init_logging(config: &LoggingConfig) -> Result<()> {
    let _level: Level = config
        .level
        .parse()
        .map_err(|_| anyhow!("invalid log level: {}", config.level))?;

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.level));

    match &config.output {
        LogOutput::Stdout | LogOutput::Stderr => {
            let use_stdout = config.output == LogOutput::Stdout;
            let layer = if use_stdout {
                fmt::layer().with_writer(std::io::stdout).boxed()
            } else {
                fmt::layer().with_writer(std::io::stderr).boxed()
            };
            tracing_subscriber::registry()
                .with(filter)
                .with(layer)
                .try_init()
                .map_err(|e| anyhow!("failed to initialize logging: {e}"))?;
        }
        LogOutput::Json => {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .with_writer(std::io::stderr)
                        .json()
                        .with_target(true)
                        .with_current_span(true),
                )
                .try_init()
                .map_err(|e| anyhow!("failed to initialize logging: {e}"))?;
        }
    }

    tracing::info!(
        output = ?config.output,
        level = %config.level,
        "logging initialized"
    );

    Ok(())
}

#[cfg(test)]
#[path = "logging_tests.rs"]
mod tests;
