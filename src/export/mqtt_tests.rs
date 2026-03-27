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
    assert_eq!(cfg.qos, 1);
    assert!(!cfg.retain);
    assert_eq!(cfg.interval.as_secs(), 10);
    assert_eq!(cfg.timeout.as_secs(), 5);
    assert!(cfg.auth.is_none());
    assert!(cfg.tls.is_none());
    assert!(cfg.client_id.is_none());
}

#[test]
fn test_parse_endpoint_mqtt() {
    let (host, port, tls) = parse_endpoint("mqtt://broker.example.com:1883");
    assert_eq!(host, "broker.example.com");
    assert_eq!(port, 1883);
    assert!(!tls);
}

#[test]
fn test_parse_endpoint_mqtts() {
    let (host, port, tls) = parse_endpoint("mqtts://broker.example.com:8883");
    assert_eq!(host, "broker.example.com");
    assert_eq!(port, 8883);
    assert!(tls);
}

#[test]
fn test_parse_endpoint_mqtts_default_port() {
    let (host, port, tls) = parse_endpoint("mqtts://broker.example.com");
    assert_eq!(host, "broker.example.com");
    assert_eq!(port, 8883);
    assert!(tls);
}

#[test]
fn test_parse_endpoint_mqtt_default_port() {
    let (host, port, tls) = parse_endpoint("mqtt://broker.example.com");
    assert_eq!(host, "broker.example.com");
    assert_eq!(port, 1883);
    assert!(!tls);
}

#[test]
fn test_parse_endpoint_ipv6_bracketed() {
    let (host, port, tls) = parse_endpoint("mqtt://[::1]:1883");
    assert_eq!(host, "::1");
    assert_eq!(port, 1883);
    assert!(!tls);
}

#[test]
fn test_parse_endpoint_ipv6_bracketed_default_port() {
    let (host, port, tls) = parse_endpoint("mqtt://[::1]");
    assert_eq!(host, "::1");
    assert_eq!(port, 1883);
    assert!(!tls);
}

#[test]
fn test_parse_endpoint_ipv6_mqtts() {
    let (host, port, tls) = parse_endpoint("mqtts://[2001:db8::1]:8883");
    assert_eq!(host, "2001:db8::1");
    assert_eq!(port, 8883);
    assert!(tls);
}

#[test]
fn test_parse_endpoint_no_scheme() {
    let (host, port, tls) = parse_endpoint("localhost:1883");
    assert_eq!(host, "localhost");
    assert_eq!(port, 1883);
    assert!(!tls);
}

#[test]
fn test_parse_endpoint_invalid_scheme_treated_as_plain() {
    // No recognized scheme, treated as plain host
    let (host, port, tls) = parse_endpoint("http://broker:1883");
    assert_eq!(host, "http://broker");
    assert_eq!(port, 1883);
    assert!(!tls);
}
