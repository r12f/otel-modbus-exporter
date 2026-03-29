use anyhow::{bail, Context, Result};
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::config::{find_config_file, CollectorConfig, Config};
use crate::logging::{init_logging, map_logging_config, LogOutput, LoggingConfig};

use super::{collect_once, filter_collectors};

/// Entry point for the `watch` subcommand.
pub async fn watch_command(
    cli_config: Option<&Path>,
    collector: Option<&str>,
    metric: Option<&str>,
    interval: Option<&str>,
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
    let watch_logging = LoggingConfig {
        level: logging_cfg.level,
        output: LogOutput::Stderr,
        syslog_facility: logging_cfg.syslog_facility,
    };
    init_logging(&watch_logging).context("failed to initialize logging")?;

    let mut filtered_collectors = filter_collectors(&config.collectors, collector, metric)?;
    if filtered_collectors.is_empty() {
        eprintln!("Fatal: no collectors/metrics match the given filters");
        std::process::exit(1);
    }

    // Parse and apply interval override
    let interval_override = match interval {
        Some(s) => {
            let dur: Duration = s
                .parse::<humantime::Duration>()
                .map_err(|e| anyhow::anyhow!("invalid --interval '{}': {}", s, e))?
                .into();
            if dur.is_zero() {
                bail!("--interval must be > 0");
            }
            Some(dur)
        }
        None => None,
    };

    if let Some(dur) = interval_override {
        for c in &mut filtered_collectors {
            c.polling_interval = dur;
        }
    }

    // Use the smallest polling_interval among filtered collectors as the loop interval
    let loop_interval = filtered_collectors
        .iter()
        .map(|c| c.polling_interval)
        .min()
        .unwrap_or(Duration::from_secs(10));

    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel_clone.cancel();
    });

    let exit_code = run_watch(&filtered_collectors, loop_interval, &cancel).await;
    std::process::exit(exit_code);
}

async fn run_watch(
    collectors: &[CollectorConfig],
    interval: Duration,
    cancel: &CancellationToken,
) -> i32 {
    let mut iteration: u64 = 0;
    let mut total_successful: usize = 0;
    let mut total_failed: usize = 0;

    loop {
        if cancel.is_cancelled() {
            break;
        }

        iteration += 1;
        let timestamp = humantime::format_rfc3339(std::time::SystemTime::now()).to_string();
        let inner_cancel = cancel.child_token();

        let (collectors_json, total_metrics, successful, failed) =
            collect_once(collectors, &inner_cancel).await;

        total_successful += successful;
        total_failed += failed;

        let output = json!({
            "timestamp": timestamp,
            "iteration": iteration,
            "collectors": collectors_json,
            "summary": {
                "total_collectors": collectors.len(),
                "total_metrics": total_metrics,
                "successful": successful,
                "failed": failed
            }
        });

        if let Ok(s) = serde_json::to_string(&output) {
            println!("{}", s);
        }

        // Sleep, but break early on cancellation
        select! {
            _ = cancel.cancelled() => { break; }
            _ = tokio::time::sleep(interval) => {}
        }
    }

    // Final summary
    let summary = json!({
        "watch_summary": {
            "total_iterations": iteration,
            "total_successful": total_successful,
            "total_failed": total_failed
        }
    });
    eprintln!(
        "{}",
        serde_json::to_string_pretty(&summary).unwrap_or_default()
    );

    if total_failed > 0 {
        2
    } else {
        0
    }
}
