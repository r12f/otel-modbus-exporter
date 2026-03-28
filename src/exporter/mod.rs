pub mod mqtt;
pub mod otlp;
pub mod prometheus;

use std::collections::HashMap;
use std::time::SystemTime;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::{ExportersConfig, MetricConfig};
use crate::metrics::{MetricType, MetricValue};

/// Common trait for all metric exporters.
///
/// Each implementation receives metric configs and cached read results,
/// then formats and sends them in its own protocol-specific way.
#[async_trait]
pub trait MetricExporter: Send {
    /// Export cached metric results.
    async fn export(
        &mut self,
        metrics: &[MetricConfig],
        results: &HashMap<String, Result<f64>>,
    ) -> Result<()>;

    /// Graceful shutdown.
    async fn shutdown(&mut self) -> Result<()>;
}

/// Convert metric configs + cached results into `MetricValue` list.
///
/// Shared by all exporters — skips metrics without a successful result.
pub fn results_to_metric_values(
    metrics: &[MetricConfig],
    results: &HashMap<String, Result<f64>>,
) -> Vec<MetricValue> {
    let now = SystemTime::now();
    metrics
        .iter()
        .filter_map(|cfg| {
            let value = match results.get(&cfg.name) {
                Some(Ok(v)) => *v,
                _ => return None,
            };
            Some(MetricValue {
                name: cfg.name.clone(),
                value,
                metric_type: match cfg.metric_type {
                    crate::config::MetricType::Gauge => MetricType::Gauge,
                    crate::config::MetricType::Counter => MetricType::Counter,
                },
                labels: std::collections::BTreeMap::new(),
                description: cfg.description.clone(),
                unit: cfg.unit.clone(),
                updated_at: now,
            })
        })
        .collect()
}

/// Create the appropriate exporter(s) from the top-level exporter config.
///
/// Returns a `Vec` because multiple exporters can be enabled simultaneously.
pub fn create_exporters(config: &ExportersConfig) -> Result<Vec<Box<dyn MetricExporter>>> {
    let mut exporters: Vec<Box<dyn MetricExporter>> = Vec::new();

    if let Some(ref otlp_cfg) = config.otlp {
        if otlp_cfg.enabled {
            exporters.push(Box::new(otlp::OtlpMetricExporter::new(otlp_cfg.clone())?));
        }
    }

    if let Some(ref prom_cfg) = config.prometheus {
        if prom_cfg.enabled {
            exporters.push(Box::new(prometheus::PrometheusMetricExporter::new(
                prom_cfg.clone(),
            )));
        }
    }

    if let Some(ref mqtt_cfg) = config.mqtt {
        if mqtt_cfg.enabled {
            exporters.push(Box::new(mqtt::MqttMetricExporter::new(mqtt_cfg.clone())?));
        }
    }

    Ok(exporters)
}
