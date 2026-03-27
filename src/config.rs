#![allow(dead_code)]
use anyhow::{bail, Context, Result};
use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "otel-modbus-exporter")]
pub struct Cli {
    /// Path to the configuration file
    #[arg(short, long, default_value = "config.yaml")]
    pub config: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub global_labels: HashMap<String, String>,
    #[serde(default)]
    pub logging: Logging,
    pub exporters: Exporters,
    pub collectors: Vec<Collector>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Logging {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_output")]
    pub output: String,
    #[serde(default = "default_syslog_facility")]
    pub syslog_facility: String,
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            output: default_log_output(),
            syslog_facility: default_syslog_facility(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_string()
}
fn default_log_output() -> String {
    "syslog".to_string()
}
fn default_syslog_facility() -> String {
    "daemon".to_string()
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Exporters {
    #[serde(default)]
    pub otlp: Option<OtlpExporter>,
    #[serde(default)]
    pub prometheus: Option<PrometheusExporter>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct OtlpExporter {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default = "default_otlp_timeout", with = "humantime_serde")]
    pub timeout: Duration,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_otlp_timeout() -> Duration {
    Duration::from_secs(10)
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct PrometheusExporter {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_prom_listen")]
    pub listen: String,
    #[serde(default = "default_prom_path")]
    pub path: String,
}

fn default_prom_listen() -> String {
    "0.0.0.0:9090".to_string()
}
fn default_prom_path() -> String {
    "/metrics".to_string()
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Collector {
    pub name: String,
    pub protocol: Protocol,
    pub slave_id: u8,
    #[serde(default = "default_polling_interval", with = "humantime_serde")]
    pub polling_interval: Duration,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub metrics: Vec<Metric>,
}

fn default_polling_interval() -> Duration {
    Duration::from_secs(10)
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Protocol {
    Tcp {
        endpoint: String,
    },
    Rtu {
        device: String,
        #[serde(default = "default_bps")]
        bps: u32,
        #[serde(default = "default_data_bits")]
        data_bits: u8,
        #[serde(default = "default_stop_bits")]
        stop_bits: u8,
        #[serde(default)]
        parity: Parity,
    },
}

fn default_bps() -> u32 {
    9600
}
fn default_data_bits() -> u8 {
    8
}
fn default_stop_bits() -> u8 {
    1
}

#[derive(Debug, Deserialize, Clone, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    #[default]
    None,
    Even,
    Odd,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "type")]
    pub metric_type: MetricType,
    pub register_type: RegisterType,
    pub address: u16,
    pub data_type: DataType,
    #[serde(default = "default_byte_order")]
    pub byte_order: ByteOrder,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub unit: String,
}

fn default_byte_order() -> ByteOrder {
    ByteOrder::BigEndian
}
fn default_scale() -> f64 {
    1.0
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Counter,
    Gauge,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RegisterType {
    Holding,
    Input,
    Coil,
    Discrete,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    U16,
    I16,
    U32,
    I32,
    F32,
    U64,
    I64,
    F64,
    Bool,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
pub enum ByteOrder {
    BigEndian,
    LittleEndian,
    MidBigEndian,
    MidLittleEndian,
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let config: Config =
            serde_yaml::from_str(&content).with_context(|| "parsing config YAML")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        let otlp_on = self.exporters.otlp.as_ref().is_some_and(|e| e.enabled);
        let prom_on = self
            .exporters
            .prometheus
            .as_ref()
            .is_some_and(|e| e.enabled);
        if !otlp_on && !prom_on {
            bail!("at least one exporter must be enabled");
        }
        if let Some(otlp) = &self.exporters.otlp {
            if otlp.enabled && otlp.endpoint.is_none() {
                bail!("otlp exporter is enabled but no endpoint is set");
            }
        }
        if self.collectors.is_empty() {
            bail!("at least one collector must be defined");
        }
        let mut cnames = std::collections::HashSet::new();
        for c in &self.collectors {
            if !cnames.insert(&c.name) {
                bail!("duplicate collector name: {}", c.name);
            }
            if c.slave_id == 0 || c.slave_id > 247 {
                bail!(
                    "collector '{}': slave_id must be 1-247, got {}",
                    c.name,
                    c.slave_id
                );
            }
            if c.metrics.is_empty() {
                bail!("collector '{}': at least one metric required", c.name);
            }
            let mut mnames = std::collections::HashSet::new();
            for m in &c.metrics {
                if !mnames.insert(&m.name) {
                    bail!("collector '{}': duplicate metric name: {}", c.name, m.name);
                }
                if (m.register_type == RegisterType::Coil
                    || m.register_type == RegisterType::Discrete)
                    && m.data_type != DataType::Bool
                {
                    bail!(
                        "collector '{}', metric '{}': coil/discrete register must use data_type bool",
                        c.name,
                        m.name
                    );
                }
                if m.data_type == DataType::Bool
                    && m.register_type != RegisterType::Coil
                    && m.register_type != RegisterType::Discrete
                {
                    bail!(
                        "collector '{}', metric '{}': bool data_type must use coil or discrete register",
                        c.name,
                        m.name
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
