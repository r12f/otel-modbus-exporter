use super::*;

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
    assert_eq!(c.logging.level, "debug");
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
    assert_eq!(c.logging.level, "info");
    assert_eq!(c.logging.output, "syslog");
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
