use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::str::FromStr;

use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

use crate::config;

// ── Syslog tracing layer ──────────────────────────────────────────────

mod syslog_layer {
    use std::fmt::Write as _;
    use std::sync::Mutex;
    use syslog::{Facility, Formatter3164, LoggerBackend};
    use tracing::field::{Field, Visit};

    /// A `tracing_subscriber::Layer` that forwards events to syslog.
    pub struct SyslogLayer {
        writer: Mutex<syslog::Logger<LoggerBackend, Formatter3164>>,
    }

    impl SyslogLayer {
        pub fn new(facility: Facility) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            let formatter = Formatter3164 {
                facility,
                hostname: None,
                process: "bus-exporter".to_string(),
                pid: std::process::id(),
            };
            let logger = syslog::unix(formatter).map_err(|e| {
                Box::new(std::io::Error::other(format!(
                    "failed to connect to syslog: {e}"
                ))) as Box<dyn std::error::Error + Send + Sync>
            })?;
            Ok(Self {
                writer: Mutex::new(logger),
            })
        }
    }

    /// Visitor that collects tracing fields into a string.
    struct FieldCollector(String);

    impl Visit for FieldCollector {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                let _ = write!(self.0, "{value:?}");
            } else {
                if !self.0.is_empty() {
                    self.0.push(' ');
                }
                let _ = write!(self.0, "{}={:?}", field.name(), value);
            }
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.0.push_str(value);
            } else {
                if !self.0.is_empty() {
                    self.0.push(' ');
                }
                let _ = write!(self.0, "{}={}", field.name(), value);
            }
        }
    }

    impl<S> tracing_subscriber::Layer<S> for SyslogLayer
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut collector = FieldCollector(String::new());
            event.record(&mut collector);

            let target = event.metadata().target();
            let msg = if target.is_empty() {
                collector.0
            } else {
                format!("{}: {}", target, collector.0)
            };

            if let Ok(mut writer) = self.writer.lock() {
                let _ = match *event.metadata().level() {
                    tracing::Level::ERROR => writer.err(&msg),
                    tracing::Level::WARN => writer.warning(&msg),
                    tracing::Level::INFO => writer.info(&msg),
                    tracing::Level::DEBUG | tracing::Level::TRACE => writer.debug(&msg),
                };
            }
        }
    }
}

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
        config::LogOutput::Json => LogOutput::Json,
        config::LogOutput::Syslog => LogOutput::Syslog,
    };

    let syslog_facility = cfg.syslog_facility;

    LoggingConfig {
        level,
        output,
        syslog_facility,
    }
}

/// Logging output target.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogOutput {
    Stdout,
    Stderr,
    /// Structured JSON output to stderr.
    Json,
    /// Native syslog via unix socket.
    Syslog,
}

impl FromStr for LogOutput {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "json" => Ok(Self::Json),
            "syslog" => Ok(Self::Syslog),
            other => Err(anyhow!("invalid log output: {other}")),
        }
    }
}

/// Logging configuration matching the `logging` section in config.yaml.
#[derive(Debug, Clone, Deserialize)]
pub struct LoggingConfig {
    pub level: String,
    pub output: LogOutput,
    #[serde(default = "default_syslog_facility")]
    pub syslog_facility: config::SyslogFacility,
}

fn default_syslog_facility() -> config::SyslogFacility {
    config::SyslogFacility::Daemon
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            output: LogOutput::Syslog,
            syslog_facility: config::SyslogFacility::Daemon,
        }
    }
}

/// Map config SyslogFacility to the syslog crate Facility.
fn map_syslog_facility(f: config::SyslogFacility) -> syslog::Facility {
    match f {
        config::SyslogFacility::Daemon => syslog::Facility::LOG_DAEMON,
        config::SyslogFacility::Local0 => syslog::Facility::LOG_LOCAL0,
        config::SyslogFacility::Local1 => syslog::Facility::LOG_LOCAL1,
        config::SyslogFacility::Local2 => syslog::Facility::LOG_LOCAL2,
        config::SyslogFacility::Local3 => syslog::Facility::LOG_LOCAL3,
        config::SyslogFacility::Local4 => syslog::Facility::LOG_LOCAL4,
        config::SyslogFacility::Local5 => syslog::Facility::LOG_LOCAL5,
        config::SyslogFacility::Local6 => syslog::Facility::LOG_LOCAL6,
        config::SyslogFacility::Local7 => syslog::Facility::LOG_LOCAL7,
    }
}

/// Initialize the tracing subscriber based on the provided logging configuration.
///
/// - `stdout` / `stderr`: Uses `tracing_subscriber::fmt` layer with the appropriate writer.
/// - `json`: Uses structured JSON format to stderr, suitable for systemd/journald capture.
/// - `syslog`: Sends log messages to the system syslog daemon via unix socket.
///   Falls back to stderr with a warning if the syslog connection fails.
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
        LogOutput::Syslog => {
            let facility = map_syslog_facility(config.syslog_facility);
            match syslog_layer::SyslogLayer::new(facility) {
                Ok(layer) => {
                    tracing_subscriber::registry()
                        .with(filter)
                        .with(layer)
                        .try_init()
                        .map_err(|e| anyhow!("failed to initialize logging: {e}"))?;
                }
                Err(e) => {
                    // Fall back to stderr with a warning
                    eprintln!("warning: failed to connect to syslog ({e}), falling back to stderr");
                    tracing_subscriber::registry()
                        .with(filter)
                        .with(fmt::layer().with_writer(std::io::stderr))
                        .try_init()
                        .map_err(|e| anyhow!("failed to initialize logging: {e}"))?;
                }
            }
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
