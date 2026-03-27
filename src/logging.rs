use anyhow::{anyhow, Result};
use tracing::Level;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Logging output target.
#[derive(Debug, Clone, PartialEq)]
pub enum LogOutput {
    Stdout,
    Stderr,
    Syslog,
}

impl LogOutput {
    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "stdout" => Ok(Self::Stdout),
            "stderr" => Ok(Self::Stderr),
            "syslog" => Ok(Self::Syslog),
            other => Err(anyhow!("invalid log output: {other}")),
        }
    }
}

/// Logging configuration matching the `logging` section in config.yaml.
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    pub level: String,
    pub output: LogOutput,
    pub syslog_facility: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            output: LogOutput::Syslog,
            syslog_facility: "daemon".to_string(),
        }
    }
}

/// Parse a log level string into a `tracing::Level`.
fn parse_level(level: &str) -> Result<Level> {
    match level.to_lowercase().as_str() {
        "trace" => Ok(Level::TRACE),
        "debug" => Ok(Level::DEBUG),
        "info" => Ok(Level::INFO),
        "warn" => Ok(Level::WARN),
        "error" => Ok(Level::ERROR),
        other => Err(anyhow!("invalid log level: {other}")),
    }
}

/// Initialize the tracing subscriber based on the provided logging configuration.
///
/// - `stdout` / `stderr`: Uses `tracing_subscriber::fmt` layer with the appropriate writer.
/// - `syslog`: Uses structured JSON format to stderr, suitable for systemd/journald capture.
///
/// Call once at startup before any tracing events are emitted.
pub fn init_logging(config: &LoggingConfig) -> Result<()> {
    let level = parse_level(&config.level)?;
    let filter = EnvFilter::new(level.to_string());

    match &config.output {
        LogOutput::Stdout => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stdout))
                .init();
        }
        LogOutput::Stderr => {
            tracing_subscriber::registry()
                .with(filter)
                .with(fmt::layer().with_writer(std::io::stderr))
                .init();
        }
        LogOutput::Syslog => {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    fmt::layer()
                        .with_writer(std::io::stderr)
                        .json()
                        .with_target(true)
                        .with_current_span(true),
                )
                .init();
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
