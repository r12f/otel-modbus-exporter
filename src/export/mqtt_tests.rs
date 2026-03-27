use super::*;

#[test]
fn test_build_topic() {
    assert_eq!(
        build_topic("modbus/metrics", "inverter", "voltage"),
        "modbus/metrics/inverter/voltage"
    );
    assert_eq!(
        build_topic("plant/data", "meter", "power"),
        "plant/data/meter/power"
    );
}

#[test]
fn test_format_value_f64() {
    assert_eq!(format_value(23.456), "23.456");
    assert_eq!(format_value(0.0), "0");
    assert_eq!(format_value(-1.5), "-1.5");
}

#[test]
fn test_format_value_integer() {
    assert_eq!(format_value(42.0), "42");
    assert_eq!(format_value(-100.0), "-100");
}

#[test]
fn test_format_value_bool_as_f64() {
    // Bools are stored as f64: 1.0 or 0.0
    assert_eq!(format_value(1.0), "1");
    assert_eq!(format_value(0.0), "0");
}

#[test]
fn test_config_defaults() {
    let yaml = r#"
endpoint: "mqtt://localhost:1883"
"#;
    let cfg: crate::config::MqttExporter = serde_yaml::from_str(yaml).unwrap();
    assert!(!cfg.enabled);
    assert_eq!(cfg.topic_prefix, "modbus/metrics");
    assert_eq!(cfg.qos, 0);
    assert!(!cfg.retain);
    assert_eq!(cfg.interval.as_secs(), 10);
    assert_eq!(cfg.timeout.as_secs(), 10);
    assert!(cfg.auth.is_none());
    assert!(cfg.tls.is_none());
    assert!(cfg.client_id.is_none());
}
