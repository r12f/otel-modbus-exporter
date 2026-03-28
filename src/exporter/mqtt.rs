use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::config::MqttExporterConfig;
use crate::metrics::MetricStore;

fn qos_from_u8(q: u8) -> QoS {
    match q {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        _ => QoS::ExactlyOnce,
    }
}

pub fn build_topic(prefix: &str, collector: &str, metric: &str) -> String {
    format!("{}/{}/{}", prefix, collector, metric)
}

pub fn format_value(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

/// Parse an mqtt:// or mqtts:// endpoint into (host, port, use_tls).
fn parse_endpoint(endpoint: &str) -> (String, u16, bool) {
    let (rest, tls) = if let Some(r) = endpoint.strip_prefix("mqtts://") {
        (r, true)
    } else if let Some(r) = endpoint.strip_prefix("mqtt://") {
        (r, false)
    } else {
        (endpoint, false)
    };

    let default_port: u16 = if tls { 8883 } else { 1883 };

    // Handle bracketed IPv6: [::1]:port or [::1]
    if let Some(rest_after_bracket) = rest.strip_prefix('[') {
        if let Some((addr, after_bracket)) = rest_after_bracket.split_once(']') {
            let port = after_bracket
                .strip_prefix(':')
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(default_port);
            return (addr.to_string(), port, tls);
        }
    }

    // Regular host:port — but only split on the last colon if the port part is numeric
    if let Some((h, p)) = rest.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.to_string(), port, tls);
        }
    }

    (rest.to_string(), default_port, tls)
}

fn build_tls_config(
    tls_cfg: Option<&crate::config::MqttTlsConfig>,
) -> Result<rumqttc::TlsConfiguration, String> {
    let ca = match tls_cfg.and_then(|t| t.ca_cert.as_ref()) {
        Some(path) => {
            std::fs::read(path).map_err(|e| format!("failed to read ca_cert '{}': {}", path, e))?
        }
        None => Vec::new(),
    };

    let client_auth = match tls_cfg {
        Some(t) if t.client_cert.is_some() && t.client_key.is_some() => {
            let cert = std::fs::read(t.client_cert.as_ref().unwrap())
                .map_err(|e| format!("failed to read client_cert: {}", e))?;
            let key = std::fs::read(t.client_key.as_ref().unwrap())
                .map_err(|e| format!("failed to read client_key: {}", e))?;
            Some((cert, key))
        }
        _ => None,
    };

    Ok(rumqttc::TlsConfiguration::Simple {
        ca,
        alpn: None,
        client_auth,
    })
}

pub async fn run_mqtt_exporter(
    config: MqttExporterConfig,
    store: MetricStore,
    cancel: CancellationToken,
) {
    let endpoint = match &config.endpoint {
        Some(ep) => ep.clone(),
        None => {
            error!("mqtt exporter has no endpoint configured");
            return;
        }
    };

    let (host, port, use_tls) = parse_endpoint(&endpoint);
    let client_id = config
        .client_id
        .clone()
        .unwrap_or_else(|| "bus-exporter".to_string());

    let mut mqttoptions = MqttOptions::new(&client_id, &host, port);
    mqttoptions.set_keep_alive(config.timeout);

    if let Some(auth) = &config.auth {
        mqttoptions.set_credentials(&auth.username, &auth.password);
    }

    if use_tls {
        match build_tls_config(config.tls.as_ref()) {
            Ok(tls_config) => {
                mqttoptions.set_transport(Transport::tls_with_config(tls_config));
            }
            Err(e) => {
                error!(%e, "failed to build TLS config");
                return;
            }
        }
    }

    let status_topic = format!("{}/status", config.topic_prefix);
    let lwt = rumqttc::LastWill::new(&status_topic, "offline", qos_from_u8(config.qos), true);
    mqttoptions.set_last_will(lwt);

    let qos = qos_from_u8(config.qos);
    let retain = config.retain;
    let prefix = config.topic_prefix.clone();
    let interval = config.interval;

    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(60);

    loop {
        let (client, mut eventloop) = AsyncClient::new(mqttoptions.clone(), 64);

        // Channel to detect connection loss from eventloop
        let (connected_tx, mut connected_rx) = watch::channel(true);

        let cancel_inner = cancel.clone();
        let loop_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_inner.cancelled() => break,
                    notification = eventloop.poll() => {
                        match notification {
                            Ok(Event::Incoming(Packet::ConnAck(_))) => {}
                            Ok(_) => {}
                            Err(e) => {
                                warn!(%e, "mqtt eventloop error");
                                let _ = connected_tx.send(false);
                                break;
                            }
                        }
                    }
                }
            }
        });

        // Publish online status
        if let Err(e) = client.publish(&status_topic, qos, true, "online").await {
            warn!(%e, "failed to publish online status");
            let _ = loop_handle.await;
            if cancel.is_cancelled() {
                return;
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(max_backoff);
            continue;
        }

        info!(endpoint = %endpoint, "mqtt exporter connected");
        backoff = Duration::from_secs(1);

        // Main publish loop
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = client.publish(&status_topic, qos, true, "offline").await;
                    let _ = client.disconnect().await;
                    let _ = loop_handle.await;
                    return;
                }
                _ = connected_rx.changed() => {
                    if !*connected_rx.borrow() {
                        warn!("mqtt connection lost, reconnecting...");
                        break;
                    }
                }
                _ = tokio::time::sleep(interval) => {
                    let metrics = store.all_metrics_flat();
                    for m in &metrics {
                        let collector = m.labels.get("collector").map(|s| s.as_str()).unwrap_or("unknown");
                        let topic = build_topic(&prefix, collector, &m.name);
                        let payload = format_value(m.value);
                        if let Err(e) = client.publish(&topic, qos, retain, payload.as_bytes()).await {
                            warn!(%e, topic = %topic, "failed to publish metric");
                        }
                    }
                }
            }
        }

        let _ = loop_handle.await;
        if cancel.is_cancelled() {
            return;
        }

        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(backoff) => {}
        }
        backoff = (backoff * 2).min(max_backoff);
    }
}

#[cfg(test)]
#[path = "mqtt_tests.rs"]
mod tests;
