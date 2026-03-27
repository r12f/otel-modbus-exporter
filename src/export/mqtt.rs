use std::sync::Arc;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS, Transport};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::config::MqttExporter;
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
    // If it looks like an integer, print without decimal
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
        // Shouldn't happen after validation, fallback
        (endpoint, false)
    };

    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(1883)),
        None => (rest.to_string(), if tls { 8883 } else { 1883 }),
    };

    (host, port, tls)
}

pub async fn run_mqtt_exporter(
    config: MqttExporter,
    store: Arc<MetricStore>,
    cancel: CancellationToken,
) {
    let endpoint = match &config.endpoint {
        Some(ep) => ep.clone(),
        None => {
            warn!("mqtt exporter has no endpoint configured");
            return;
        }
    };

    let (host, port, use_tls) = parse_endpoint(&endpoint);
    let client_id = config
        .client_id
        .clone()
        .unwrap_or_else(|| "modbus-exporter".to_string());

    let mut mqttoptions = MqttOptions::new(&client_id, &host, port);
    mqttoptions.set_keep_alive(Duration::from_secs(30));

    if let Some(auth) = &config.auth {
        mqttoptions.set_credentials(&auth.username, &auth.password);
    }

    if use_tls {
        // Use rustls with default config; custom certs would need more setup
        let tls_config = rumqttc::TlsConfiguration::default();
        mqttoptions.set_transport(Transport::tls_with_config(tls_config));
    }

    // Set Last Will and Testament
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

        // Drive the event loop in background
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
        // Reset backoff on successful connection
        #[allow(unused_assignments)]
        {
            backoff = Duration::from_secs(1);
        }

        // Main publish loop
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    // Publish offline before exit
                    let _ = client.publish(&status_topic, qos, true, "offline").await;
                    let _ = client.disconnect().await;
                    let _ = loop_handle.await;
                    return;
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
    }
}

#[cfg(test)]
#[path = "mqtt_tests.rs"]
mod tests;
