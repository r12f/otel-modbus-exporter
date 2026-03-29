use anyhow::{bail, Result};
use regex::Regex;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::config::{self, Config};
use crate::reader::MetricReaderFactory;
use crate::reader::MetricReaderFactoryImpl;

fn protocol_str(protocol: &config::Protocol) -> &'static str {
    match protocol {
        config::Protocol::ModbusTcp { .. } => "modbus-tcp",
        config::Protocol::ModbusRtu { .. } => "modbus-rtu",
        config::Protocol::I2c { .. } => "i2c",
        config::Protocol::Spi { .. } => "spi",
        config::Protocol::I3c { .. } => "i3c",
    }
}

pub async fn run_pull(
    config: &Config,
    collector_filter: Option<&str>,
    metric_filter: Option<&str>,
) -> Result<i32> {
    // Compile regexes
    let collector_re = collector_filter
        .map(Regex::new)
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid --collector regex: {e}"))?;
    let metric_re = metric_filter
        .map(Regex::new)
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid --metric regex: {e}"))?;

    // Filter collectors
    let mut filtered_collectors: Vec<config::CollectorConfig> = Vec::new();
    for c in &config.collectors {
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
            filtered_collectors.push(cc);
        }
    }

    if filtered_collectors.is_empty() {
        bail!("no collectors/metrics match the given filters");
    }

    let factory = MetricReaderFactoryImpl;
    let cancel = CancellationToken::new();
    let mut total_metrics: usize = 0;
    let mut successful: usize = 0;
    let mut failed: usize = 0;
    let mut collectors_json = Vec::new();

    for collector in &filtered_collectors {
        let mut reader = match factory.create(collector) {
            Ok(r) => r,
            Err(e) => {
                // Report all metrics as failed for this collector
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
                let protocol_name = protocol_str(&collector.protocol);
                collectors_json.push(json!({
                    "name": collector.name,
                    "protocol": protocol_name,
                    "metrics": metrics_json
                }));
                continue;
            }
        };
        reader.set_metrics(collector.metrics.clone());
        if let Err(e) = reader.connect().await {
            // Report all metrics as failed for this collector
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
            let protocol_name = protocol_str(&collector.protocol);
            collectors_json.push(json!({
                "name": collector.name,
                "protocol": protocol_name,
                "metrics": metrics_json
            }));
            continue;
        }
        let results = reader.read(&cancel).await;
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

        let protocol_name = protocol_str(&collector.protocol);

        collectors_json.push(json!({
            "name": collector.name,
            "protocol": protocol_name,
            "metrics": metrics_json
        }));
    }

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
        Ok(1)
    } else {
        Ok(0)
    }
}
