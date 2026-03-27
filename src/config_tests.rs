use super::*;
use std::io::Write;

fn minimal_yaml() -> String {
    r#"
exporters:
  prometheus:
    enabled: true
collectors:
  - name: test
    protocol:
      type: tcp
      endpoint: "localhost:502"
    slave_id: 1
    metrics:
      - name: voltage
        type: gauge
        register_type: holding
        address: 0
        data_type: u16
"#
    .to_string()
}

fn parse(yaml: &str) -> Result<Config> {
    let config: Config = serde_yaml::from_str(yaml).context("parsing YAML")?;
    config.validate()?;
    Ok(config)
}

#[test]
fn test_parse_minimal() {
    let c = parse(&minimal_yaml()).unwrap();
    assert_eq!(c.collectors.len(), 1);
    assert_eq!(c.collectors[0].slave_id, 1);
    assert_eq!(c.collectors[0].polling_interval.as_secs(), 10);
    assert_eq!(c.collectors[0].metrics[0].scale, 1.0);
    assert_eq!(c.collectors[0].metrics[0].byte_order, ByteOrder::BigEndian);
}

#[test]
fn test_parse_full() {
    let yaml = r#"
global_labels:
  env: prod
logging:
  level: debug
  output: stdout
  syslog_facility: local0
exporters:
  otlp:
    enabled: true
    endpoint: "http://localhost:4318"
    timeout: "5s"
    headers:
      Authorization: "Bearer t"
  prometheus:
    enabled: true
    listen: "0.0.0.0:8080"
    path: "/prom"
collectors:
  - name: inv
    protocol:
      type: tcp
      endpoint: "192.168.1.10:502"
    slave_id: 1
    polling_interval: "5s"
    labels:
      loc: roof
    metrics:
      - name: dc_v
        description: "DC voltage"
        type: gauge
        register_type: holding
        address: 100
        data_type: f32
        byte_order: big_endian
        scale: 0.1
        offset: 0.0
        unit: "V"
  - name: meter
    protocol:
      type: rtu
      device: "/dev/ttyUSB0"
      bps: 19200
      data_bits: 8
      stop_bits: 1
      parity: even
    slave_id: 2
    metrics:
      - name: coil_s
        type: gauge
        register_type: coil
        address: 0
        data_type: bool
"#;
    let c = parse(yaml).unwrap();
    assert_eq!(c.global_labels.get("env").unwrap(), "prod");
    assert_eq!(c.logging.level, LogLevel::Debug);
    assert_eq!(c.logging.output, LogOutput::Stdout);
    assert_eq!(c.logging.syslog_facility, SyslogFacility::Local0);
    assert_eq!(c.collectors.len(), 2);
    match &c.collectors[1].protocol {
        Protocol::Rtu { bps, parity, .. } => {
            assert_eq!(*bps, 19200);
            assert_eq!(*parity, Parity::Even);
        }
        _ => panic!("expected RTU"),
    }
}

#[test]
fn test_no_exporter_enabled() {
    let y = r#"
exporters:
  prometheus:
    enabled: false
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("at least one exporter"));
}

#[test]
fn test_no_collectors() {
    let y = "exporters:\n  prometheus:\n    enabled: true\ncollectors: []\n";
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("at least one collector"));
}

#[test]
fn test_dup_collector() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: d
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics: [{ name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }]
  - name: d
    protocol: { type: tcp, endpoint: "b:502" }
    slave_id: 2
    metrics: [{ name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }]
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("duplicate collector name"));
}

#[test]
fn test_slave_id_zero() {
    let y = minimal_yaml().replace("slave_id: 1", "slave_id: 0");
    assert!(parse(&y)
        .unwrap_err()
        .to_string()
        .contains("slave_id must be 1-247"));
}

#[test]
fn test_coil_must_bool() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: gauge, register_type: coil, address: 0, data_type: u16 }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("coil/discrete register must use data_type bool"));
}

#[test]
fn test_bool_must_coil_discrete() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: gauge, register_type: holding, address: 0, data_type: bool }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("bool data_type must use coil or discrete"));
}

#[test]
fn test_dup_metric() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: d, type: gauge, register_type: holding, address: 0, data_type: u16 }
      - { name: d, type: counter, register_type: holding, address: 1, data_type: u16 }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("duplicate metric name"));
}

#[test]
fn test_empty_metrics() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics: []
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("at least one metric"));
}

#[test]
fn test_otlp_no_endpoint() {
    let y = r#"
exporters:
  otlp: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    assert!(parse(y).unwrap_err().to_string().contains("endpoint"));
}

#[test]
fn test_defaults() {
    let c = parse(&minimal_yaml()).unwrap();
    assert_eq!(c.logging.level, LogLevel::Info);
    assert_eq!(c.logging.output, LogOutput::Syslog);
    assert_eq!(c.logging.syslog_facility, SyslogFacility::Daemon);
    let p = c.exporters.prometheus.as_ref().unwrap();
    assert_eq!(p.listen, "0.0.0.0:9090");
    assert_eq!(p.path, "/metrics");
}

#[test]
fn test_rtu_defaults() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: rtu, device: "/dev/ttyUSB0" }
    slave_id: 1
    metrics:
      - { name: c, type: gauge, register_type: coil, address: 0, data_type: bool }
"#;
    let c = parse(y).unwrap();
    match &c.collectors[0].protocol {
        Protocol::Rtu {
            bps,
            data_bits,
            stop_bits,
            parity,
            ..
        } => {
            assert_eq!(*bps, 9600);
            assert_eq!(*data_bits, 8);
            assert_eq!(*stop_bits, 1);
            assert_eq!(*parity, Parity::None);
        }
        _ => panic!("expected RTU"),
    }
}

#[test]
fn test_all_data_types() {
    for dt in ["u16", "i16", "u32", "i32", "f32", "u64", "i64", "f64"] {
        let y = format!(
            r#"
exporters:
  prometheus: {{ enabled: true }}
collectors:
  - name: t
    protocol: {{ type: tcp, endpoint: "a:502" }}
    slave_id: 1
    metrics:
      - {{ name: m, type: gauge, register_type: holding, address: 0, data_type: {dt} }}
"#
        );
        parse(&y).unwrap_or_else(|e| panic!("{dt}: {e}"));
    }
}

#[test]
fn test_all_byte_orders() {
    for bo in [
        "big_endian",
        "little_endian",
        "mid_big_endian",
        "mid_little_endian",
    ] {
        let y = format!(
            r#"
exporters:
  prometheus: {{ enabled: true }}
collectors:
  - name: t
    protocol: {{ type: tcp, endpoint: "a:502" }}
    slave_id: 1
    metrics:
      - {{ name: m, type: gauge, register_type: holding, address: 0, data_type: u32, byte_order: {bo} }}
"#
        );
        parse(&y).unwrap_or_else(|e| panic!("{bo}: {e}"));
    }
}

// ===== New tests for review comment fixes =====

#[test]
fn test_invalid_log_level() {
    let y = minimal_yaml().replace("", ""); // Use raw yaml with invalid level
    let y = r#"
logging:
  level: banana
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    assert!(parse(y).is_err(), "invalid log level should fail to parse");
}

#[test]
fn test_invalid_log_output() {
    let y = r#"
logging:
  output: file
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    assert!(parse(y).is_err(), "invalid log output should fail to parse");
}

#[test]
fn test_invalid_syslog_facility() {
    let y = r#"
logging:
  syslog_facility: kern
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    assert!(
        parse(y).is_err(),
        "invalid syslog facility should fail to parse"
    );
}

#[test]
fn test_all_log_levels() {
    for level in ["trace", "debug", "info", "warn", "error"] {
        let y = format!(
            r#"
logging:
  level: {level}
exporters:
  prometheus: {{ enabled: true }}
collectors:
  - name: t
    protocol: {{ type: tcp, endpoint: "a:502" }}
    slave_id: 1
    metrics:
      - {{ name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }}
"#
        );
        parse(&y).unwrap_or_else(|e| panic!("level {level}: {e}"));
    }
}

#[test]
fn test_all_syslog_facilities() {
    for fac in [
        "daemon", "local0", "local1", "local2", "local3", "local4", "local5", "local6", "local7",
    ] {
        let y = format!(
            r#"
logging:
  syslog_facility: {fac}
exporters:
  prometheus: {{ enabled: true }}
collectors:
  - name: t
    protocol: {{ type: tcp, endpoint: "a:502" }}
    slave_id: 1
    metrics:
      - {{ name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }}
"#
        );
        parse(&y).unwrap_or_else(|e| panic!("facility {fac}: {e}"));
    }
}

#[test]
fn test_rtu_data_bits_out_of_range() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: rtu, device: "/dev/ttyUSB0", data_bits: 4 }
    slave_id: 1
    metrics:
      - { name: c, type: gauge, register_type: coil, address: 0, data_type: bool }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("data_bits must be 5-8"));
}

#[test]
fn test_rtu_stop_bits_out_of_range() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: rtu, device: "/dev/ttyUSB0", stop_bits: 3 }
    slave_id: 1
    metrics:
      - { name: c, type: gauge, register_type: coil, address: 0, data_type: bool }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("stop_bits must be 1-2"));
}

#[test]
fn test_scale_zero_rejected() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16, scale: 0.0 }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("scale must not be 0.0"));
}

#[test]
fn test_polling_interval_zero_rejected() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    polling_interval: "0s"
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("polling_interval must be at least 100ms"));
}

#[test]
fn test_polling_interval_too_short() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    polling_interval: "50ms"
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("polling_interval must be at least 100ms"));
}

#[test]
fn test_polling_interval_100ms_ok() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    polling_interval: "100ms"
    metrics:
      - { name: v, type: gauge, register_type: holding, address: 0, data_type: u16 }
"#;
    parse(y).unwrap();
}

#[test]
fn test_counter_on_coil_rejected() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: counter, register_type: coil, address: 0, data_type: bool }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("coil/discrete registers only support gauge"));
}

#[test]
fn test_counter_on_discrete_rejected() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: counter, register_type: discrete, address: 0, data_type: bool }
"#;
    assert!(parse(y)
        .unwrap_err()
        .to_string()
        .contains("coil/discrete registers only support gauge"));
}

#[test]
fn test_address_overflow_u32() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: gauge, register_type: holding, address: 65535, data_type: u32 }
"#;
    assert!(parse(y).unwrap_err().to_string().contains("exceeds 65535"));
}

#[test]
fn test_address_overflow_u64() {
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: gauge, register_type: holding, address: 65533, data_type: u64 }
"#;
    assert!(parse(y).unwrap_err().to_string().contains("exceeds 65535"));
}

#[test]
fn test_address_at_boundary_ok() {
    // u32 takes 2 registers, so address 65534 + 2 = 65536 which is fine (0-indexed)
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: gauge, register_type: holding, address: 65534, data_type: u32 }
"#;
    parse(y).unwrap();
}

#[test]
fn test_address_single_register_max() {
    // u16 at address 65535 — 65535 + 1 = 65536, ok
    let y = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: t
    protocol: { type: tcp, endpoint: "a:502" }
    slave_id: 1
    metrics:
      - { name: m, type: gauge, register_type: holding, address: 65535, data_type: u16 }
"#;
    parse(y).unwrap();
}

// ===== Metrics files tests =====

fn create_temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write_file(dir: &std::path::Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

#[test]
fn test_metrics_files_with_defaults() {
    let dir = create_temp_dir();
    write_file(
        dir.path(),
        "devices/meter.yaml",
        r#"
defaults:
  register_type: holding
  data_type: f32
  type: gauge
metrics:
  - name: voltage
    address: 0
    unit: "V"
  - name: current
    address: 6
    unit: "A"
"#,
    );
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "devices/meter.yaml"
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let config = Config::load(&config_path).unwrap();
    assert_eq!(config.collectors[0].metrics.len(), 2);
    let v = &config.collectors[0].metrics[0];
    assert_eq!(v.name, "voltage");
    assert_eq!(v.register_type, RegisterType::Holding);
    assert_eq!(v.data_type, DataType::F32);
    assert_eq!(v.metric_type, MetricType::Gauge);
    assert_eq!(v.unit, "V");
}

#[test]
fn test_metrics_files_merge_order() {
    let dir = create_temp_dir();
    write_file(
        dir.path(),
        "base.yaml",
        r#"
metrics:
  - name: voltage
    type: gauge
    register_type: holding
    data_type: f32
    address: 0
    unit: "V"
    description: "base voltage"
  - name: current
    type: gauge
    register_type: holding
    data_type: f32
    address: 6
    unit: "A"
"#,
    );
    write_file(
        dir.path(),
        "override.yaml",
        r#"
metrics:
  - name: voltage
    type: gauge
    register_type: input
    data_type: f32
    address: 100
    unit: "V"
"#,
    );
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "base.yaml"
      - "override.yaml"
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let config = Config::load(&config_path).unwrap();
    assert_eq!(config.collectors[0].metrics.len(), 2);
    // voltage from override.yaml
    let v = config.collectors[0]
        .metrics
        .iter()
        .find(|m| m.name == "voltage")
        .unwrap();
    assert_eq!(v.register_type, RegisterType::Input);
    assert_eq!(v.address, 100);
    // description should be empty (full replacement, not inherited)
    assert_eq!(v.description, "");
    // current from base.yaml
    let c = config.collectors[0]
        .metrics
        .iter()
        .find(|m| m.name == "current")
        .unwrap();
    assert_eq!(c.address, 6);
}

#[test]
fn test_inline_metrics_override_file() {
    let dir = create_temp_dir();
    write_file(
        dir.path(),
        "meter.yaml",
        r#"
metrics:
  - name: voltage
    type: gauge
    register_type: holding
    data_type: f32
    address: 0
    unit: "V"
"#,
    );
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "meter.yaml"
    metrics:
      - name: voltage
        type: gauge
        register_type: input
        data_type: u16
        address: 200
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let config = Config::load(&config_path).unwrap();
    assert_eq!(config.collectors[0].metrics.len(), 1);
    let v = &config.collectors[0].metrics[0];
    assert_eq!(v.register_type, RegisterType::Input);
    assert_eq!(v.data_type, DataType::U16);
    assert_eq!(v.address, 200);
}

#[test]
fn test_full_replacement_no_field_inheritance() {
    let dir = create_temp_dir();
    write_file(
        dir.path(),
        "base.yaml",
        r#"
metrics:
  - name: voltage
    type: gauge
    register_type: holding
    data_type: f32
    address: 0
    unit: "V"
    description: "Voltage reading"
    scale: 0.1
"#,
    );
    write_file(
        dir.path(),
        "override.yaml",
        r#"
metrics:
  - name: voltage
    type: gauge
    register_type: holding
    data_type: f32
    address: 0
"#,
    );
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "base.yaml"
      - "override.yaml"
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let config = Config::load(&config_path).unwrap();
    let v = &config.collectors[0].metrics[0];
    // Full replacement: unit, description, scale revert to defaults
    assert_eq!(v.unit, "");
    assert_eq!(v.description, "");
    assert_eq!(v.scale, 1.0);
}

#[test]
fn test_relative_path_resolution() {
    let dir = create_temp_dir();
    write_file(
        dir.path(),
        "subdir/devices/meter.yaml",
        r#"
metrics:
  - name: voltage
    type: gauge
    register_type: holding
    data_type: u16
    address: 0
"#,
    );
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "subdir/devices/meter.yaml"
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let config = Config::load(&config_path).unwrap();
    assert_eq!(config.collectors[0].metrics.len(), 1);
}

#[test]
fn test_missing_metrics_file_error() {
    let dir = create_temp_dir();
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "nonexistent.yaml"
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("reading metrics file"),
        "got: {}",
        err
    );
}

#[test]
fn test_empty_metrics_file_error() {
    let dir = create_temp_dir();
    write_file(dir.path(), "empty.yaml", "metrics: []\n");
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "empty.yaml"
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let err = Config::load(&config_path).unwrap_err();
    assert!(
        err.to_string().contains("contains no metrics"),
        "got: {}",
        err
    );
}

#[test]
fn test_defaults_plus_per_metric_override() {
    let dir = create_temp_dir();
    write_file(
        dir.path(),
        "meter.yaml",
        r#"
defaults:
  register_type: holding
  data_type: f32
  type: gauge
  byte_order: little_endian
  scale: 0.1
metrics:
  - name: voltage
    address: 0
    unit: "V"
  - name: current
    address: 6
    unit: "A"
    data_type: u32
    byte_order: big_endian
    scale: 0.01
"#,
    );
    let config_yaml = r#"
exporters:
  prometheus: { enabled: true }
collectors:
  - name: test
    protocol: { type: tcp, endpoint: "localhost:502" }
    slave_id: 1
    metrics_files:
      - "meter.yaml"
"#;
    let config_path = write_file(dir.path(), "config.yaml", config_yaml);
    let config = Config::load(&config_path).unwrap();
    // voltage gets defaults
    let v = config.collectors[0]
        .metrics
        .iter()
        .find(|m| m.name == "voltage")
        .unwrap();
    assert_eq!(v.data_type, DataType::F32);
    assert_eq!(v.byte_order, ByteOrder::LittleEndian);
    assert_eq!(v.scale, 0.1);
    // current overrides defaults
    let c = config.collectors[0]
        .metrics
        .iter()
        .find(|m| m.name == "current")
        .unwrap();
    assert_eq!(c.data_type, DataType::U32);
    assert_eq!(c.byte_order, ByteOrder::BigEndian);
    assert_eq!(c.scale, 0.01);
}
