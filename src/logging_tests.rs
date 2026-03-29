use super::*;
use std::str::FromStr;

#[test]
fn test_log_output_from_str() {
    assert_eq!(LogOutput::from_str("stdout").unwrap(), LogOutput::Stdout);
    assert_eq!(LogOutput::from_str("stderr").unwrap(), LogOutput::Stderr);
    assert_eq!(LogOutput::from_str("json").unwrap(), LogOutput::Json);
    assert_eq!(LogOutput::from_str("syslog").unwrap(), LogOutput::Syslog);
    assert_eq!(LogOutput::from_str("STDOUT").unwrap(), LogOutput::Stdout);
    assert_eq!(LogOutput::from_str("Stderr").unwrap(), LogOutput::Stderr);
    assert_eq!(LogOutput::from_str("SYSLOG").unwrap(), LogOutput::Syslog);
    assert!(LogOutput::from_str("invalid").is_err());
}

#[test]
fn test_parse_level_via_std() {
    assert_eq!(
        "trace".parse::<tracing::Level>().unwrap(),
        tracing::Level::TRACE
    );
    assert_eq!(
        "debug".parse::<tracing::Level>().unwrap(),
        tracing::Level::DEBUG
    );
    assert_eq!(
        "info".parse::<tracing::Level>().unwrap(),
        tracing::Level::INFO
    );
    assert_eq!(
        "warn".parse::<tracing::Level>().unwrap(),
        tracing::Level::WARN
    );
    assert_eq!(
        "error".parse::<tracing::Level>().unwrap(),
        tracing::Level::ERROR
    );
    assert_eq!(
        "INFO".parse::<tracing::Level>().unwrap(),
        tracing::Level::INFO
    );
}

#[test]
fn test_parse_level_invalid() {
    assert!("verbose".parse::<tracing::Level>().is_err());
    assert!("".parse::<tracing::Level>().is_err());
}

#[test]
fn test_default_logging_config() {
    let config = LoggingConfig::default();
    assert_eq!(config.level, "info");
    assert_eq!(config.output, LogOutput::Syslog);
    assert_eq!(config.syslog_facility, config::SyslogFacility::Daemon);
}

#[test]
fn test_init_logging_invalid_level() {
    let config = LoggingConfig {
        level: "invalid".to_string(),
        output: LogOutput::Stdout,
        syslog_facility: config::SyslogFacility::Daemon,
    };
    assert!(init_logging(&config).is_err());
}

#[test]
fn test_init_logging_stdout() {
    let config = LoggingConfig {
        level: "info".to_string(),
        output: LogOutput::Stdout,
        syslog_facility: config::SyslogFacility::Daemon,
    };
    let _ = init_logging(&config);
}

#[test]
fn test_init_logging_stderr() {
    let config = LoggingConfig {
        level: "debug".to_string(),
        output: LogOutput::Stderr,
        syslog_facility: config::SyslogFacility::Daemon,
    };
    let _ = init_logging(&config);
}

#[test]
fn test_init_logging_json() {
    let config = LoggingConfig {
        level: "warn".to_string(),
        output: LogOutput::Json,
        syslog_facility: config::SyslogFacility::Daemon,
    };
    let _ = init_logging(&config);
}

#[test]
fn test_logging_config_deserialize() {
    let yaml = r#"
level: debug
output: json
"#;
    let config: LoggingConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.level, "debug");
    assert_eq!(config.output, LogOutput::Json);
    // syslog_facility should default to Daemon when not specified
    assert_eq!(config.syslog_facility, config::SyslogFacility::Daemon);
}

#[test]
fn test_logging_config_deserialize_syslog() {
    let yaml = r#"
level: info
output: syslog
syslog_facility: local3
"#;
    let config: LoggingConfig = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.output, LogOutput::Syslog);
    assert_eq!(config.syslog_facility, config::SyslogFacility::Local3);
}

#[test]
fn test_map_syslog_facility_all_variants() {
    use config::SyslogFacility::*;

    // syslog::Facility doesn't impl PartialEq, so use matches!()
    assert!(matches!(
        map_syslog_facility(Daemon),
        syslog::Facility::LOG_DAEMON
    ));
    assert!(matches!(
        map_syslog_facility(Local0),
        syslog::Facility::LOG_LOCAL0
    ));
    assert!(matches!(
        map_syslog_facility(Local1),
        syslog::Facility::LOG_LOCAL1
    ));
    assert!(matches!(
        map_syslog_facility(Local2),
        syslog::Facility::LOG_LOCAL2
    ));
    assert!(matches!(
        map_syslog_facility(Local3),
        syslog::Facility::LOG_LOCAL3
    ));
    assert!(matches!(
        map_syslog_facility(Local4),
        syslog::Facility::LOG_LOCAL4
    ));
    assert!(matches!(
        map_syslog_facility(Local5),
        syslog::Facility::LOG_LOCAL5
    ));
    assert!(matches!(
        map_syslog_facility(Local6),
        syslog::Facility::LOG_LOCAL6
    ));
    assert!(matches!(
        map_syslog_facility(Local7),
        syslog::Facility::LOG_LOCAL7
    ));
}
