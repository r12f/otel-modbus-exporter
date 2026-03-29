use anyhow::{bail, Context, Result};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use std::path::Path;

use crate::config::{find_config_file, Config};
use crate::logging::{init_logging, map_logging_config, LogOutput, LoggingConfig};

use super::{collect_once, filter_collectors};

/// Entry point for the `pull` subcommand.
///
/// Loads config, initialises logging (forced to stderr), and delegates to
/// [`run_pull`] for the actual collection work.
pub async fn pull_command(
    cli_config: Option<&Path>,
    collector: Option<&str>,
    metric: Option<&str>,
) -> Result<()> {
    let config_path = find_config_file(cli_config).context("failed to find configuration file");
    let config_path = match config_path {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Fatal: {e:#}");
            std::process::exit(1);
        }
    };
    let config = match Config::load_for_pull(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Fatal: failed to load configuration: {e:#}");
            std::process::exit(1);
        }
    };
    let logging_cfg = map_logging_config(&config.logging);
    // For pull, force stderr output
    let pull_logging = LoggingConfig {
        level: logging_cfg.level,
        output: LogOutput::Stderr,
        syslog_facility: logging_cfg.syslog_facility,
    };
    init_logging(&pull_logging).context("failed to initialize logging")?;

    let exit_code = match run_pull(&config, collector, metric).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Fatal: {e:#}");
            std::process::exit(1);
        }
    };
    std::process::exit(exit_code);
}

pub async fn run_pull(
    config: &Config,
    collector_filter: Option<&str>,
    metric_filter: Option<&str>,
) -> Result<i32> {
    let filtered_collectors =
        filter_collectors(&config.collectors, collector_filter, metric_filter)?;

    if filtered_collectors.is_empty() {
        bail!("no collectors/metrics match the given filters");
    }

    let cancel = CancellationToken::new();
    let (collectors_json, total_metrics, successful, failed) =
        collect_once(&filtered_collectors, &cancel).await;

    let output = json!({
        "collectors": collectors_json,
        "summary": {
            "total_collectors": filtered_collectors.len(),
            "total_metrics": total_metrics,
            "successful": successful,
            "failed": failed
        }
    });

    println!("{}", serde_json::to_string_pretty(&output)?);

    if failed > 0 {
        Ok(2)
    } else {
        Ok(0)
    }
}
