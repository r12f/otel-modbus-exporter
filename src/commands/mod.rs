pub mod install;
pub mod pull;
pub mod run;
pub mod watch;

use anyhow::Result;
use regex::Regex;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::config;
use crate::reader::{MetricReaderFactory, MetricReaderFactoryImpl};

/// Filter collectors by name regex and metrics by metric regex.
/// Returns a new list of collectors with only matching metrics.
pub fn filter_collectors(
    collectors: &[config::CollectorConfig],
    collector_filter: Option<&str>,
    metric_filter: Option<&str>,
) -> Result<Vec<config::CollectorConfig>> {
    let collector_re = collector_filter
        .map(Regex::new)
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid --collector regex: {e}"))?;
    let metric_re = metric_filter
        .map(Regex::new)
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid --metric regex: {e}"))?;

    let mut filtered: Vec<config::CollectorConfig> = Vec::new();
    for c in collectors {
        if let Some(ref re) = collector_re {
            if !re.is_match(&c.name) {
                continue;
            }
        }
        let mut cc = c.clone();
        if let Some(ref re) = metric_re {
            cc.metrics.retain(|m| re.is_match(&m.name));
        }
        if !cc.metrics.is_empty() {
            filtered.push(cc);
        }
    }
    Ok(filtered)
}

/// Shared collection logic used by both `pull` and `watch`.
///
/// Connects to each collector, reads metrics, and returns JSON objects plus
/// counters. Returns `(collectors_json, total_metrics, successful, failed)`.
///
/// TODO: Consider keeping persistent connections across iterations instead of
/// connect/disconnect per call. The current approach is correct but slower for
/// short polling intervals.
pub async fn collect_once(
    collectors: &[config::CollectorConfig],
    cancel: &CancellationToken,
) -> (Vec<serde_json::Value>, usize, usize, usize) {
    let factory = MetricReaderFactoryImpl;
    let mut total_metrics: usize = 0;
    let mut successful: usize = 0;
    let mut failed: usize = 0;
    let mut collectors_json = Vec::new();

    for collector in collectors {
        let mut reader = match factory.create(collector) {
            Ok(r) => r,
            Err(e) => {
                let mut metrics_json = Vec::new();
                for metric_cfg in &collector.metrics {
                    total_metrics += 1;
                    failed += 1;
                    metrics_json.push(json!({
                        "name": metric_cfg.name,
                        "value": null,
                        "raw_value": null,
                        "error": format!("collector create failed: {e}")
                    }));
                }
                collectors_json.push(json!({
                    "name": collector.name,
                    "protocol": collector.protocol.to_string(),
                    "metrics": metrics_json
                }));
                continue;
            }
        };
        reader.set_metrics(collector.metrics.clone());
        if let Err(e) = reader.connect().await {
            let mut metrics_json = Vec::new();
            for metric_cfg in &collector.metrics {
                total_metrics += 1;
                failed += 1;
                metrics_json.push(json!({
                    "name": metric_cfg.name,
                    "value": null,
                    "raw_value": null,
                    "error": format!("connect failed: {e}")
                }));
            }
            collectors_json.push(json!({
                "name": collector.name,
                "protocol": collector.protocol.to_string(),
                "metrics": metrics_json
            }));
            continue;
        }
        let results = reader.read(cancel).await;
        let _ = reader.disconnect().await;

        let mut metrics_json = Vec::new();
        for metric_cfg in &collector.metrics {
            total_metrics += 1;
            match results.metrics.get(&metric_cfg.name) {
                Some(Ok((raw_value, scaled_value))) => {
                    successful += 1;
                    metrics_json.push(json!({
                        "name": metric_cfg.name,
                        "value": scaled_value,
                        "raw_value": raw_value,
                        "error": null
                    }));
                }
                Some(Err(e)) => {
                    failed += 1;
                    metrics_json.push(json!({
                        "name": metric_cfg.name,
                        "value": null,
                        "raw_value": null,
                        "error": e.to_string()
                    }));
                }
                None => {
                    failed += 1;
                    metrics_json.push(json!({
                        "name": metric_cfg.name,
                        "value": null,
                        "raw_value": null,
                        "error": "metric not in results"
                    }));
                }
            }
        }

        collectors_json.push(json!({
            "name": collector.name,
            "protocol": collector.protocol.to_string(),
            "metrics": metrics_json
        }));
    }

    (collectors_json, total_metrics, successful, failed)
}
