//! Thin helpers over rumqttc's AsyncClient: publish JSON / strings, clear a
//! retained topic. QoS 1 everywhere (matches the rest of the system).
use rumqttc::{AsyncClient, QoS};
use serde::Serialize;

pub async fn publish_json<T: Serialize>(client: &AsyncClient, topic: &str, payload: &T, retain: bool) {
    match serde_json::to_vec(payload) {
        Ok(bytes) => {
            if let Err(e) = client.publish(topic, QoS::AtLeastOnce, retain, bytes).await {
                eprintln!("[hub] publish failed on {topic}: {e}");
            }
        }
        Err(e) => eprintln!("[hub] serialize error on {topic}: {e}"),
    }
}

#[allow(dead_code)]
pub async fn publish_str(client: &AsyncClient, topic: &str, payload: &str, retain: bool) {
    if let Err(e) = client
        .publish(topic, QoS::AtLeastOnce, retain, payload.as_bytes().to_vec())
        .await
    {
        eprintln!("[hub] publish failed on {topic}: {e}");
    }
}

/// Delete a retained message by publishing an empty retained payload.
pub async fn clear_retained(client: &AsyncClient, topic: &str) {
    if let Err(e) = client.publish(topic, QoS::AtLeastOnce, true, Vec::new()).await {
        eprintln!("[hub] clear retained failed on {topic}: {e}");
    }
}
