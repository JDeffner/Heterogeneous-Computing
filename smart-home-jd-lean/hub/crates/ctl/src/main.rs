//! Small operator tool (replaces the old seed / clear-devices scripts).
//!
//!   ctl seed-rooms          create a default set of rooms via the hub
//!   ctl clear               wipe retained broker state + data/*.json
//!
//! Broker via MQTT_URL (default mqtt://localhost:1883), data via DATA_DIR.
use std::time::{Duration, Instant};

use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use shared::{topics, RoomControl};

fn parse_broker(url: &str) -> (String, u16) {
    let s = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("tcp://"))
        .unwrap_or(url);
    let s = s.split('/').next().unwrap_or(s);
    match s.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
        None => (s.to_string(), 1883),
    }
}

#[tokio::main]
async fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    let broker = std::env::var("MQTT_URL").unwrap_or_else(|_| "mqtt://localhost:1883".into());
    let (host, port) = parse_broker(&broker);

    let mut opts = MqttOptions::new(format!("ctl-{}", std::process::id()), host.as_str(), port);
    opts.set_keep_alive(Duration::from_secs(10));
    let (client, mut eventloop) = AsyncClient::new(opts, 64);

    match cmd.as_str() {
        "seed-rooms" => {
            let rooms = [
                ("House A", "Ground floor", "Kitchen"),
                ("House A", "Ground floor", "Living room"),
                ("House A", "Ground floor", "Bathroom"),
                ("House A", "Ground floor", "Bedroom"),
            ];
            for (building, floor, room) in rooms {
                let ctrl = RoomControl::Create {
                    building: building.into(),
                    floor: floor.into(),
                    room: room.into(),
                };
                let bytes = serde_json::to_vec(&ctrl).unwrap();
                let _ = client.publish(topics::room_control(), QoS::AtLeastOnce, false, bytes).await;
                println!("seed: room {room}");
            }
            // Drive the event loop briefly so the publishes actually go out.
            pump(&mut eventloop, Duration::from_millis(800)).await;
            let _ = client.disconnect().await;
            println!("done. (the hub creates + persists the rooms)");
        }
        "clear" => {
            // Collect retained topics (broker resends them on subscribe), then
            // clear each by publishing an empty retained payload.
            client.subscribe("smarthome/#", QoS::AtLeastOnce).await.unwrap();
            let mut retained: Vec<String> = Vec::new();
            let deadline = Instant::now() + Duration::from_millis(1500);
            while Instant::now() < deadline {
                match tokio::time::timeout(Duration::from_millis(300), eventloop.poll()).await {
                    Ok(Ok(Event::Incoming(Packet::Publish(p)))) => {
                        if p.retain && !p.payload.is_empty() && !retained.contains(&p.topic) {
                            retained.push(p.topic.clone());
                        }
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        eprintln!("mqtt error: {e}");
                        break;
                    }
                    Err(_) => {} // poll timeout: keep waiting until deadline
                }
            }
            for topic in &retained {
                let _ = client.publish(topic, QoS::AtLeastOnce, true, Vec::new()).await;
            }
            pump(&mut eventloop, Duration::from_millis(600)).await;
            let _ = client.disconnect().await;
            println!("cleared {} retained topic(s)", retained.len());

            // Wipe persisted state files.
            let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".into());
            for f in ["devices.json", "rooms.json", "sim-devices.json"] {
                let path = std::path::Path::new(&data_dir).join(f);
                if path.exists() {
                    match std::fs::remove_file(&path) {
                        Ok(_) => println!("removed {path:?}"),
                        Err(e) => eprintln!("could not remove {path:?}: {e}"),
                    }
                }
            }
        }
        _ => {
            eprintln!("usage: ctl <seed-rooms|clear>");
            std::process::exit(2);
        }
    }
}

/// Drive the event loop for a fixed duration so queued packets are sent.
async fn pump(eventloop: &mut rumqttc::EventLoop, dur: Duration) {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), eventloop.poll()).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
}
