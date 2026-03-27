use super::*;

#[test]
fn test_log_output_from_str() {
    assert_eq!(LogOutput::from_str("stdout").unwrap(), LogOutput::Stdout);
    assert_eq!(LogOutput::from_str("stderr").unwrap(), LogOutput::Stderr);
    assert_eq!(LogOutput::from_str("syslog").unwrap(), LogOutput::Syslog);
    assert_eq!(LogOutput::from_str("STDOUT").unwrap(), LogOutput::Stdout);
    assert_eq!(LogOutput::from_str("Stderr").unwrap(), LogOutput::Stderr);
    assert!(LogOutput::from_str("invalid").is_err());
}

#[test]
fn test_parse_level_valid() {
    assert_eq!(parse_level("trace").unwrap(), tracing::Level::TRACE);
    assert_eq!(parse_level("debug").unwrap(), tracing::Level::DEBUG);
    assert_eq!(parse_level("info").unwrap(), tracing::Level::INFO);
    assert_eq!(parse_level("warn").unwrap(), tracing::Level::WARN);
    assert_eq!(parse_level("error").unwrap(), tracing::Level::ERROR);
    assert_eq!(parse_level("INFO").unwrap(), tracing::Level::INFO);
}

#[test]
fn test_parse_level_invalid() {
    assert!(parse_level("verbose").is_err());
    assert!(parse_level("").is_err());
}

#[test]
fn test_default_logging_config() {
    let config = LoggingConfig::default();
    assert_eq!(config.level, "info");
    assert_eq!(config.output, LogOutput::Syslog);
    assert_eq!(config.syslog_facility, "daemon");
}

#[test]
fn test_init_logging_invalid_level() {
    let config = LoggingConfig {
        level: "invalid".to_string(),
        output: LogOutput::Stdout,
        syslog_facility: "daemon".to_string(),
    };
    assert!(init_logging(&config).is_err());
}
