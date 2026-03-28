#![allow(dead_code)]
use anyhow::{bail, Context, Result};
use clap::Parser;
use indexmap::IndexMap;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "bus-exporter")]
pub struct Cli {
    /// Path to the configuration file
    #[arg(short, long)]
    pub config: Option<PathBuf>,
}

/// Default search paths for the config file (in priority order).
pub const CONFIG_SEARCH_PATHS: &[&str] = &[
    "./config.yaml",
    "~/.config/bus-exporter/config.yaml",
    "/etc/bus-exporter/config.yaml",
];

/// Find the config file using the fallback search order.
/// If `explicit` is Some, use that exact path (error if missing).
/// Otherwise search the default locations.
pub fn find_config_file(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        bail!("specified config file not found: {}", p.display());
    }

    let home = std::env::var("HOME").unwrap_or_default();
    for pattern in CONFIG_SEARCH_PATHS {
        let expanded = pattern.replace('~', &home);
        let path = PathBuf::from(&expanded);
        if path.exists() {
            info!(path = %path.display(), "found config file");
            return Ok(path);
        }
    }

    let searched: Vec<String> = CONFIG_SEARCH_PATHS
        .iter()
        .map(|p| p.replace('~', &home))
        .collect();
    bail!("no config file found; searched:\n{}", searched.join("\n"));
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

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LogOutput {
    Stdout,
    Stderr,
    Syslog,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SyslogFacility {
    Daemon,
    Local0,
    Local1,
    Local2,
    Local3,
    Local4,
    Local5,
    Local6,
    Local7,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Logging {
    #[serde(default = "default_log_level")]
    pub level: LogLevel,
    #[serde(default = "default_log_output")]
    pub output: LogOutput,
    #[serde(default = "default_syslog_facility")]
    pub syslog_facility: SyslogFacility,
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

fn default_log_level() -> LogLevel {
    LogLevel::Info
}
fn default_log_output() -> LogOutput {
    LogOutput::Syslog
}
fn default_syslog_facility() -> SyslogFacility {
    SyslogFacility::Daemon
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Exporters {
    #[serde(default)]
    pub otlp: Option<OtlpExporter>,
    #[serde(default)]
    pub prometheus: Option<PrometheusExporter>,
    #[serde(default)]
    pub mqtt: Option<MqttExporter>,
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
    #[serde(default = "default_otlp_interval", with = "humantime_serde")]
    pub interval: Duration,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}

fn default_otlp_interval() -> Duration {
    Duration::from_secs(10)
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttExporter {
    #[serde(default)]
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub client_id: Option<String>,
    #[serde(default = "default_mqtt_topic_prefix")]
    pub topic_prefix: String,
    pub auth: Option<MqttAuth>,
    pub tls: Option<MqttTls>,
    #[serde(default = "default_mqtt_qos")]
    pub qos: u8,
    #[serde(default)]
    pub retain: bool,
    #[serde(default = "default_mqtt_interval", with = "humantime_serde")]
    pub interval: Duration,
    #[serde(default = "default_mqtt_timeout", with = "humantime_serde")]
    pub timeout: Duration,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttAuth {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttTls {
    pub ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    #[serde(default)]
    pub insecure: bool,
}

fn default_mqtt_topic_prefix() -> String {
    "modbus/metrics".to_string()
}

fn default_mqtt_qos() -> u8 {
    1
}

fn default_mqtt_interval() -> Duration {
    Duration::from_secs(10)
}

fn default_mqtt_timeout() -> Duration {
    Duration::from_secs(5)
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Collector {
    pub name: String,
    pub protocol: Protocol,
    #[serde(default)]
    pub slave_id: Option<u8>,
    #[serde(default = "default_polling_interval", with = "humantime_serde")]
    pub polling_interval: Duration,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub metrics_files: Option<Vec<String>>,
    #[serde(default)]
    pub metrics: Vec<Metric>,
}

fn default_polling_interval() -> Duration {
    Duration::from_secs(10)
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum Protocol {
    #[serde(rename = "modbus-tcp")]
    ModbusTcp { endpoint: String },
    #[serde(rename = "modbus-rtu")]
    ModbusRtu {
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
    #[serde(rename = "i2c")]
    I2c { bus: String, address: u8 },
    #[serde(rename = "spi")]
    Spi {
        device: String,
        #[serde(default = "default_spi_speed_hz")]
        speed_hz: u32,
        #[serde(default = "default_spi_mode")]
        mode: u8,
        #[serde(default = "default_spi_bits_per_word")]
        bits_per_word: u8,
    },
}

fn default_spi_speed_hz() -> u32 {
    1_000_000
}
fn default_spi_mode() -> u8 {
    0
}
fn default_spi_bits_per_word() -> u8 {
    8
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
    pub register_type: Option<RegisterType>,
    pub address: Option<u16>,
    pub data_type: DataType,
    #[serde(default = "default_byte_order")]
    pub byte_order: ByteOrder,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub unit: String,
    /// SPI-only: command bytes to transmit (TX buffer).
    #[serde(default)]
    pub command: Vec<u8>,
    /// SPI-only: total response bytes. Defaults to command length (full-duplex).
    #[serde(default)]
    pub response_length: Option<u16>,
    /// SPI-only: skip first N bytes of response before decoding.
    #[serde(default)]
    pub response_offset: u16,
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
    U8,
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

impl DataType {
    /// Returns the number of 16-bit Modbus registers this data type occupies.
    pub fn register_count(self) -> u16 {
        match self {
            DataType::U8 | DataType::U16 | DataType::I16 | DataType::Bool => 1,
            DataType::U32 | DataType::I32 | DataType::F32 => 2,
            DataType::U64 | DataType::I64 | DataType::F64 => 4,
        }
    }

    /// Returns the number of raw bytes this data type occupies.
    pub fn byte_size(self) -> usize {
        match self {
            DataType::Bool | DataType::U8 => 1,
            DataType::U16 | DataType::I16 => 2,
            DataType::U32 | DataType::I32 | DataType::F32 => 4,
            DataType::U64 | DataType::I64 | DataType::F64 => 8,
        }
    }
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

/// Metrics file for reusable metric definitions.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MetricsFile {
    #[serde(default)]
    pub defaults: Option<MetricDefaults>,
    pub metrics: Vec<RawMetric>,
}

/// Default values applied to all metrics in a metrics file.
#[derive(Debug, Deserialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct MetricDefaults {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub metric_type: Option<MetricType>,
    pub register_type: Option<RegisterType>,
    pub data_type: Option<DataType>,
    pub byte_order: Option<ByteOrder>,
    pub scale: Option<f64>,
    pub offset: Option<f64>,
    pub unit: Option<String>,
}

/// A metric with all optional fields, used for metrics file parsing.
/// Required fields are filled from defaults or must be present.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RawMetric {
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub metric_type: Option<MetricType>,
    pub register_type: Option<RegisterType>,
    pub address: Option<u16>,
    pub data_type: Option<DataType>,
    pub byte_order: Option<ByteOrder>,
    pub scale: Option<f64>,
    pub offset: Option<f64>,
    pub unit: Option<String>,
    #[serde(default)]
    pub command: Vec<u8>,
    #[serde(default)]
    pub response_length: Option<u16>,
    #[serde(default)]
    pub response_offset: u16,
}

impl RawMetric {
    /// Apply defaults and convert to a full Metric.
    fn into_metric(
        self,
        defaults: &Option<MetricDefaults>,
        collector_name: &str,
        file_path: &str,
    ) -> Result<Metric> {
        let d = defaults.as_ref();

        let metric_type = self
            .metric_type
            .or_else(|| d.and_then(|d| d.metric_type))
            .with_context(|| {
                format!(
                    "collector '{}': metric '{}' in '{}': missing required field 'type'",
                    collector_name, self.name, file_path
                )
            })?;

        let register_type = self
            .register_type
            .or_else(|| d.and_then(|d| d.register_type));

        let address = self.address;

        let data_type = self
            .data_type
            .or_else(|| d.and_then(|d| d.data_type))
            .with_context(|| {
                format!(
                    "collector '{}': metric '{}' in '{}': missing required field 'data_type'",
                    collector_name, self.name, file_path
                )
            })?;

        Ok(Metric {
            name: self.name,
            description: self
                .description
                .or_else(|| d.and_then(|d| d.description.clone()))
                .unwrap_or_default(),
            metric_type,
            register_type,
            address,
            data_type,
            byte_order: self
                .byte_order
                .or_else(|| d.and_then(|d| d.byte_order))
                .unwrap_or(ByteOrder::BigEndian),
            scale: self
                .scale
                .or_else(|| d.and_then(|d| d.scale))
                .unwrap_or(1.0),
            offset: self
                .offset
                .or_else(|| d.and_then(|d| d.offset))
                .unwrap_or(0.0),
            unit: self
                .unit
                .or_else(|| d.and_then(|d| d.unit.clone()))
                .unwrap_or_default(),
            command: self.command,
            response_length: self.response_length,
            response_offset: self.response_offset,
        })
    }
}

impl Collector {
    /// Load and merge metrics from metrics_files and inline metrics.
    pub fn resolve_metrics_files(&mut self, config_dir: &Path) -> Result<()> {
        let mut merged: IndexMap<String, Metric> = IndexMap::new();

        let files = match &self.metrics_files {
            Some(f) if !f.is_empty() => f.clone(),
            _ => vec![],
        };

        for file_path_str in &files {
            let file_path = if Path::new(file_path_str).is_absolute() {
                PathBuf::from(file_path_str)
            } else {
                config_dir.join(file_path_str)
            };

            let content = std::fs::read_to_string(&file_path).with_context(|| {
                format!(
                    "collector '{}': reading metrics file '{}'",
                    self.name,
                    file_path.display()
                )
            })?;

            let metrics_file: MetricsFile = serde_yaml::from_str(&content).with_context(|| {
                format!(
                    "collector '{}': parsing metrics file '{}'",
                    self.name,
                    file_path.display()
                )
            })?;

            if metrics_file.metrics.is_empty() {
                bail!(
                    "collector '{}': metrics file '{}' contains no metrics",
                    self.name,
                    file_path.display()
                );
            }

            info!(
                collector = %self.name,
                file = %file_path.display(),
                count = metrics_file.metrics.len(),
                "loaded metrics file"
            );

            for raw in metrics_file.metrics {
                let metric = raw.into_metric(
                    &metrics_file.defaults,
                    &self.name,
                    &file_path.display().to_string(),
                )?;
                merged.insert(metric.name.clone(), metric);
            }
        }

        // Inline metrics have highest priority
        let inline_metrics = std::mem::take(&mut self.metrics);
        for metric in inline_metrics {
            merged.insert(metric.name.clone(), metric);
        }

        self.metrics = merged.into_values().collect();
        Ok(())
    }
}

impl Config {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        let content =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut config: Config =
            serde_yaml::from_str(&content).with_context(|| "parsing config YAML")?;

        let config_dir = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        for collector in &mut config.collectors {
            collector.resolve_metrics_files(&config_dir)?;
        }

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
        let mqtt_on = self.exporters.mqtt.as_ref().is_some_and(|e| e.enabled);
        if !otlp_on && !prom_on && !mqtt_on {
            bail!("at least one exporter must be enabled");
        }
        if let Some(otlp) = &self.exporters.otlp {
            if otlp.enabled && otlp.endpoint.is_none() {
                bail!("otlp exporter is enabled but no endpoint is set");
            }
        }
        if let Some(mqtt) = &self.exporters.mqtt {
            if mqtt.enabled {
                match &mqtt.endpoint {
                    None => bail!("mqtt exporter is enabled but no endpoint is set"),
                    Some(ep) => {
                        if !ep.starts_with("mqtt://") && !ep.starts_with("mqtts://") {
                            bail!("mqtt endpoint must start with mqtt:// or mqtts://");
                        }
                    }
                }
            }
            if mqtt.qos > 2 {
                bail!("mqtt qos must be 0, 1, or 2, got {}", mqtt.qos);
            }
            if let Some(tls) = &mqtt.tls {
                let has_cert = tls.client_cert.is_some();
                let has_key = tls.client_key.is_some();
                if has_cert != has_key {
                    bail!("mqtt tls: client_cert and client_key must both be set for mutual TLS");
                }
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
            // Protocol-specific validation
            match &c.protocol {
                Protocol::ModbusTcp { endpoint } => {
                    // slave_id required for Modbus
                    match c.slave_id {
                        Some(id) if id == 0 || id > 247 => {
                            bail!("collector '{}': slave_id must be 1-247, got {}", c.name, id);
                        }
                        None => {
                            bail!(
                                "collector '{}': slave_id is required for Modbus protocols",
                                c.name
                            );
                        }
                        _ => {}
                    }
                    let valid = endpoint
                        .rsplit_once(':')
                        .is_some_and(|(_, port)| port.parse::<u16>().is_ok());
                    if !valid {
                        bail!(
                            "collector '{}': invalid TCP endpoint '{}' (expected host:port, e.g. 127.0.0.1:502)",
                            c.name,
                            endpoint
                        );
                    }
                }
                Protocol::ModbusRtu {
                    data_bits,
                    stop_bits,
                    ..
                } => {
                    // slave_id required for Modbus
                    match c.slave_id {
                        Some(id) if id == 0 || id > 247 => {
                            bail!("collector '{}': slave_id must be 1-247, got {}", c.name, id);
                        }
                        None => {
                            bail!(
                                "collector '{}': slave_id is required for Modbus protocols",
                                c.name
                            );
                        }
                        _ => {}
                    }
                    if !(5..=8).contains(data_bits) {
                        bail!(
                            "collector '{}': data_bits must be 5-8, got {}",
                            c.name,
                            data_bits
                        );
                    }
                    if !(1..=2).contains(stop_bits) {
                        bail!(
                            "collector '{}': stop_bits must be 1-2, got {}",
                            c.name,
                            stop_bits
                        );
                    }
                }
                Protocol::I2c { bus, address } => {
                    if bus.is_empty() {
                        bail!("collector '{}': I2C bus path must not be empty", c.name);
                    }
                    if *address < 0x03 || *address > 0x77 {
                        bail!(
                            "collector '{}': I2C address must be 0x03-0x77, got {:#04x}",
                            c.name,
                            address
                        );
                    }
                }
                Protocol::Spi {
                    device,
                    speed_hz,
                    mode,
                    bits_per_word,
                } => {
                    if device.is_empty() {
                        bail!("collector '{}': SPI device path must not be empty", c.name);
                    }
                    if *speed_hz == 0 {
                        bail!("collector '{}': SPI speed_hz must be > 0", c.name);
                    }
                    if *mode > 3 {
                        bail!("collector '{}': SPI mode must be 0-3, got {}", c.name, mode);
                    }
                    if *bits_per_word == 0 || *bits_per_word > 32 {
                        bail!(
                            "collector '{}': SPI bits_per_word must be 1-32, got {}",
                            c.name,
                            bits_per_word
                        );
                    }
                }
            }
            // Validate polling_interval minimum (1ms)
            if c.polling_interval.as_millis() < 1 {
                bail!(
                    "collector '{}': polling_interval must be at least 1ms, got {:?}",
                    c.name,
                    c.polling_interval
                );
            }
            if c.metrics.is_empty() {
                bail!("collector '{}': at least one metric required", c.name);
            }
            let is_i2c = matches!(c.protocol, Protocol::I2c { .. });
            let is_spi = matches!(c.protocol, Protocol::Spi { .. });
            let is_modbus = !is_i2c && !is_spi;
            for m in &c.metrics {
                // Address is required for Modbus and I2C protocols
                if !is_spi && m.address.is_none() {
                    bail!(
                        "collector '{}', metric '{}': address is required for {} protocol",
                        c.name,
                        m.name,
                        if is_i2c { "I2C" } else { "Modbus" }
                    );
                }
                // Modbus-specific validations
                if is_modbus {
                    let register_type = m.register_type.unwrap_or(RegisterType::Holding);
                    if (register_type == RegisterType::Coil
                        || register_type == RegisterType::Discrete)
                        && m.data_type != DataType::Bool
                    {
                        bail!(
                            "collector '{}', metric '{}': coil/discrete register must use data_type bool",
                            c.name,
                            m.name
                        );
                    }
                    if m.data_type == DataType::Bool
                        && register_type != RegisterType::Coil
                        && register_type != RegisterType::Discrete
                    {
                        bail!(
                            "collector '{}', metric '{}': bool data_type must use coil or discrete register",
                            c.name,
                            m.name
                        );
                    }
                    if m.metric_type == MetricType::Counter
                        && (register_type == RegisterType::Coil
                            || register_type == RegisterType::Discrete)
                    {
                        bail!(
                            "collector '{}', metric '{}': coil/discrete registers only support gauge metric type",
                            c.name,
                            m.name
                        );
                    }
                    if m.register_type.is_none() {
                        bail!(
                            "collector '{}', metric '{}': register_type is required for Modbus protocols",
                            c.name,
                            m.name
                        );
                    }
                    // u8 data type is not valid for Modbus (registers are 16-bit)
                    if m.data_type == DataType::U8 {
                        bail!(
                            "collector '{}', metric '{}': data_type u8 is not supported for Modbus protocols (minimum register size is 16-bit)",
                            c.name,
                            m.name
                        );
                    }
                }
                // I2C-specific validations
                if is_i2c {
                    // Validate metric address fits in u8 (I2C register addresses are 8-bit)
                    if m.address.unwrap() > 0xFF {
                        bail!(
                            "collector '{}', metric '{}': I2C register address {:#06x} exceeds u8 range (max 0xFF)",
                            c.name,
                            m.name,
                            m.address.unwrap()
                        );
                    }
                    // Mid-endian byte orders are Modbus-specific (word-swapped)
                    if matches!(
                        m.byte_order,
                        ByteOrder::MidBigEndian | ByteOrder::MidLittleEndian
                    ) {
                        bail!(
                            "collector '{}', metric '{}': mid-endian byte order is not supported for I2C (Modbus-specific)",
                            c.name,
                            m.name
                        );
                    }
                }
                // SPI-specific validations
                if is_spi {
                    if m.command.is_empty() {
                        bail!(
                            "collector '{}', metric '{}': command is required for SPI metrics",
                            c.name,
                            m.name
                        );
                    }
                    // Mid-endian byte orders are Modbus-specific
                    if matches!(
                        m.byte_order,
                        ByteOrder::MidBigEndian | ByteOrder::MidLittleEndian
                    ) {
                        bail!(
                            "collector '{}', metric '{}': mid-endian byte order is not supported for SPI (Modbus-specific)",
                            c.name,
                            m.name
                        );
                    }
                    let resp_len = m.response_length.unwrap_or(m.command.len() as u16);
                    let data_bytes = m.data_type.byte_size() as u16;
                    if m.response_offset + data_bytes > resp_len {
                        bail!(
                            "collector '{}', metric '{}': response_offset ({}) + data_type bytes ({}) exceeds response_length ({})",
                            c.name,
                            m.name,
                            m.response_offset,
                            data_bytes,
                            resp_len
                        );
                    }
                }
                if m.metric_type == MetricType::Counter && m.data_type == DataType::Bool {
                    bail!(
                        "collector '{}', metric '{}': counter metric type cannot be used with bool data_type",
                        c.name,
                        m.name
                    );
                }
                // Validate scale != 0.0
                if m.scale == 0.0 {
                    bail!(
                        "collector '{}', metric '{}': scale must not be 0.0",
                        c.name,
                        m.name
                    );
                }
                // Validate multi-register address overflow (Modbus only)
                if is_modbus {
                    let reg_count = m.data_type.register_count();
                    if m.address.unwrap() as u32 + reg_count as u32 > 65536 {
                        bail!(
                            "collector '{}', metric '{}': address {} + {} registers exceeds 65535",
                            c.name,
                            m.name,
                            m.address.unwrap(),
                            reg_count
                        );
                    }
                }
                // Warn if byte_order is set to non-default for single-register types
                // (byte_order is meaningless for u16/i16/bool which occupy only 1 register)
                if is_modbus
                    && m.data_type.register_count() == 1
                    && m.byte_order != ByteOrder::BigEndian
                {
                    eprintln!(
                        "warning: collector '{}', metric '{}': byte_order has no effect for single-register type {:?}",
                        c.name, m.name, m.data_type
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
