#![allow(dead_code)]
mod collector;
mod config;
mod exporter;
mod internal_metrics;
mod logging;
mod metrics;
mod reader;

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use collector::{CollectorEngine, DEFAULT_SHUTDOWN_TIMEOUT};
use config::{find_config_file, Cli, Config};
use internal_metrics::InternalMetrics;
use logging::{init_logging, LogOutput, LoggingConfig};
use metrics::MetricStore;
use reader::MetricReaderFactoryImpl;

// ── Config → logging mapping ──────────────────────────────────────────

fn map_logging_config(cfg: &config::LoggingConfig) -> LoggingConfig {
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

// ── Shutdown signal ───────────────────────────────────────────────────

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => info!("received SIGINT"),
            _ = sigterm.recv() => info!("received SIGTERM"),
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.expect("failed to listen for Ctrl+C");
        info!("received SIGINT");
    }
}

// ── Entry point ───────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Parse CLI
    let cli = Cli::parse();

    // 2. Find and load config
    let config_path =
        find_config_file(cli.config.as_deref()).context("failed to find configuration file")?;
    let config = Config::load(&config_path).context("failed to load configuration")?;

    // 3. Init logging
    let logging_cfg = map_logging_config(&config.logging);
    init_logging(&logging_cfg).context("failed to initialize logging")?;

    info!(collectors = config.collectors.len(), "configuration loaded");

    // 4. Create shared MetricStore
    let store = MetricStore::new();

    // 4b. Create internal metrics
    let internal_metrics = Arc::new(InternalMetrics::new());
    internal_metrics.collectors_total.store(
        config.collectors.len() as u64,
        std::sync::atomic::Ordering::Relaxed,
    );

    // 5. Spawn collector tasks
    let global_labels: BTreeMap<String, String> =
        config.global_labels.clone().into_iter().collect();
    let factory = MetricReaderFactoryImpl;
    let engine = CollectorEngine::spawn(
        config.collectors.clone(),
        store.clone(),
        global_labels,
        &factory,
        Some(Arc::clone(&internal_metrics)),
    );

    // 6. Start Prometheus exporter (if enabled)
    let cancel = CancellationToken::new();
    let mut prom_handle = None;
    if let Some(ref prom_cfg) = config.exporters.prometheus {
        if prom_cfg.enabled {
            let prom_cfg = prom_cfg.clone();
            let store = store.clone();
            let cancel = cancel.clone();
            let im = Arc::clone(&internal_metrics);
            prom_handle = Some(tokio::spawn(async move {
                if let Err(e) =
                    exporter::prometheus::serve(&prom_cfg, store, cancel, Some(im)).await
                {
                    error!(%e, "Prometheus exporter failed");
                }
            }));
        }
    }

    // 7. Start OTLP exporter (if enabled)
    let mut otlp_handle = None;
    if let Some(ref otlp_cfg) = config.exporters.otlp {
        if otlp_cfg.enabled {
            let otlp_cfg = otlp_cfg.clone();
            let store = store.clone();
            let global_labels = config.global_labels.clone();
            let cancel = cancel.clone();
            let im = Arc::clone(&internal_metrics);
            otlp_handle = Some(tokio::spawn(async move {
                exporter::otlp::run(otlp_cfg, store, global_labels, cancel, Some(im)).await;
            }));
        }
    }

    // 8. Start MQTT exporter (if enabled)
    let mut mqtt_handle = None;
    if let Some(ref mqtt_cfg) = config.exporters.mqtt {
        if mqtt_cfg.enabled {
            let mqtt_cfg = mqtt_cfg.clone();
            let store = store.clone();
            let cancel = cancel.clone();
            mqtt_handle = Some(tokio::spawn(async move {
                exporter::mqtt::run_mqtt_exporter(mqtt_cfg, store, cancel).await;
            }));
        }
    }

    // 9. Wait for shutdown signal
    shutdown_signal().await;
    info!("initiating graceful shutdown");

    // Cancel exporters
    cancel.cancel();

    // Shutdown collectors with timeout
    engine.shutdown(DEFAULT_SHUTDOWN_TIMEOUT).await;

    // Wait for exporter tasks
    if let Some(h) = prom_handle {
        let _ = h.await;
    }
    if let Some(h) = otlp_handle {
        let _ = h.await;
    }
    if let Some(h) = mqtt_handle {
        let _ = h.await;
    }

    info!("shutdown complete");
    Ok(())
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
