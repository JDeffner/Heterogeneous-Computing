//! The central HUB - the heart of the centralized architecture.
//!
//! Every device communicates with the hub via the MQTT broker. The hub owns the
//! device REGISTRY, the ROOM registry, RULE evaluation and ALARM management. It
//! is loosely coupled to everything else (only MQTT + JSON files on disk): if a
//! UI or device is down the hub keeps running; on restart it rebuilds its
//! retained state from disk.
mod alarms;
mod catalog;
mod config;
mod mqtt;
mod registry;
mod rules;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, MqttOptions, Packet};
use shared::util::{new_id, now_iso};
use shared::{
    topics, CommissionRequest, CommissionedMessage, DeviceControl, HubControl, HubStatus, Location,
    PairingAd, Room, RoomControl, SituationEvent, Status, Telemetry,
};

use alarms::AlarmEngine;
use config::Config;
use mqtt::{clear_retained, publish_json};
use registry::Registry;
use rules::RuleEngine;

struct Hub {
    client: AsyncClient,
    config: Config,
    registry: Registry,
    alarms: AlarmEngine,
    rules: RuleEngine,
    rooms: HashMap<String, Room>,
    pairing_ads: HashMap<String, PairingAd>,
    data_dir: PathBuf,
    rooms_file: PathBuf,
}

impl Hub {
    async fn emit(&mut self, events: Vec<SituationEvent>) {
        for ev in events {
            publish_json(&self.client, &topics::event(&ev.event_id), &ev, false).await;
            let cleared = if ev.cleared.unwrap_or(false) { " CLEARED" } else { "" };
            println!("EVENT [{}]{cleared} {}", ev.severity.as_str(), ev.message);
            self.alarms.handle_event(&ev).await;
        }
    }

    async fn handle_publish(&mut self, topic: &str, payload: &[u8]) {
        // Presence is plain text ("online"/"offline").
        if topic.ends_with("/presence") {
            if let Some(id) = topics::device_id_from_topic(topic) {
                if !payload.is_empty() {
                    let online = payload == b"online";
                    let id = id.to_string();
                    self.registry
                        .set_status(&id, if online { Status::Online } else { Status::Offline })
                        .await;
                    let evs = self.rules.handle_presence(&self.registry, &id, online);
                    self.emit(evs).await;
                }
            }
            return;
        }

        // Pairing advertisements are retained; an empty payload clears one.
        if let Some(serial) = topics::serial_from_pairing(topic) {
            if payload.is_empty() {
                self.pairing_ads.remove(serial);
            } else if let Ok(ad) = serde_json::from_slice::<PairingAd>(payload) {
                self.pairing_ads.insert(serial.to_string(), ad);
            }
            return;
        }

        if payload.is_empty() {
            return; // cleared retained message
        }

        if topic.ends_with("/telemetry") {
            match serde_json::from_slice::<Telemetry>(payload) {
                Ok(t) => {
                    let evs = self.rules.handle_telemetry(&self.registry, &t);
                    self.emit(evs).await;
                }
                Err(e) => eprintln!("[hub] invalid telemetry on {topic}: {e}"),
            }
        } else if topic == topics::commission_control().as_str() {
            if let Ok(req) = serde_json::from_slice::<CommissionRequest>(payload) {
                self.handle_commission(req).await;
            }
        } else if topic == topics::room_control().as_str() {
            if let Ok(ctrl) = serde_json::from_slice::<RoomControl>(payload) {
                self.handle_room_control(ctrl).await;
            }
        } else if topic == topics::device_control().as_str() {
            if let Ok(ctrl) = serde_json::from_slice::<DeviceControl>(payload) {
                self.handle_device_control(ctrl).await;
            }
        } else if topic == topics::alarm_control().as_str() {
            if let Ok(ctrl) = serde_json::from_slice(payload) {
                self.alarms.handle_control(&ctrl).await;
            }
        } else if topic == topics::hub_control().as_str() {
            if let Ok(ctrl) = serde_json::from_slice::<HubControl>(payload) {
                self.handle_hub_control(ctrl).await;
            }
        }
    }

    async fn tick(&mut self) {
        let evs = self.rules.evaluate(&self.registry, &self.config);
        self.emit(evs).await;
    }

    // ----- Onboarding ------------------------------------------------------

    async fn handle_commission(&mut self, req: CommissionRequest) {
        let Some(ad) = self.pairing_ads.get(&req.serial).cloned() else {
            eprintln!("[hub] commission for unknown/expired serial {}", req.serial);
            return;
        };
        let location = self.resolve_location(req.room_id.as_deref());
        let record = self
            .registry
            .commission(req.serial.clone(), ad.kind, req.name, location)
            .await;

        let msg = CommissionedMessage {
            serial: req.serial.clone(),
            device_id: record.device_id.clone(),
            name: record.name.clone(),
            location: record.location.clone(),
        };
        publish_json(&self.client, &topics::pairing_commissioned(&req.serial), &msg, false).await;
        clear_retained(&self.client, &topics::pairing(&req.serial)).await;
        self.pairing_ads.remove(&req.serial);
    }

    fn resolve_location(&self, room_id: Option<&str>) -> Option<Location> {
        let room_id = room_id?;
        if room_id.is_empty() {
            return None;
        }
        match self.rooms.get(room_id) {
            Some(r) => Some(Location {
                building: r.building.clone(),
                floor: r.floor.clone(),
                room: r.room.clone(),
                room_id: Some(r.room_id.clone()),
            }),
            None => {
                eprintln!("[hub] unknown room {room_id} (leaving unassigned)");
                None
            }
        }
    }

    async fn handle_hub_control(&mut self, ctrl: HubControl) {
        if let Some(force) = ctrl.force_night {
            self.config.force_night = force;
            println!("[hub] forceNight -> {force}");
            self.publish_hub_status().await;
        }
    }

    async fn publish_hub_status(&self) {
        let status = HubStatus {
            night_start_hour: self.config.night_start_hour,
            night_end_hour: self.config.night_end_hour,
            force_night: self.config.force_night,
        };
        publish_json(&self.client, &topics::hub_status(), &status, true).await;
    }

    // ----- Rooms -----------------------------------------------------------

    async fn handle_room_control(&mut self, ctrl: RoomControl) {
        match ctrl {
            RoomControl::Create { building, floor, room } => {
                let suffix = new_id("r");
                let suffix = suffix.rsplit('-').next().unwrap_or("0");
                let room_id = format!("{}-{}", slug(&room), suffix);
                let r = Room {
                    room_id: room_id.clone(),
                    building,
                    floor,
                    room,
                    created_at: now_iso(),
                };
                publish_json(&self.client, &topics::room(&room_id), &r, true).await;
                println!("[hub] room created: {room_id} ({})", r.room);
                self.rooms.insert(room_id, r);
                self.persist_rooms();
            }
            RoomControl::Delete { room_id } => {
                if self.rooms.remove(&room_id).is_none() {
                    return;
                }
                self.persist_rooms();
                clear_retained(&self.client, &topics::room(&room_id)).await;
                let to_unassign: Vec<String> = self
                    .registry
                    .all()
                    .filter(|d| d.location.as_ref().and_then(|l| l.room_id.as_deref()) == Some(&room_id))
                    .map(|d| d.device_id.clone())
                    .collect();
                for id in to_unassign {
                    self.registry.assign(&id, None).await;
                }
                println!("[hub] room deleted: {room_id}");
            }
        }
    }

    async fn handle_device_control(&mut self, ctrl: DeviceControl) {
        match ctrl {
            DeviceControl::Assign { device_id, room_id } => {
                if let Some(rid) = room_id.as_deref() {
                    if !rid.is_empty() && !self.rooms.contains_key(rid) {
                        eprintln!("[hub] assign to unknown room {rid}");
                        return;
                    }
                }
                let loc = self.resolve_location(room_id.as_deref());
                self.registry.assign(&device_id, loc).await;
            }
            DeviceControl::Remove { device_id } => {
                self.registry.remove(&device_id).await;
            }
        }
    }

    fn persist_rooms(&self) {
        if let Err(e) = std::fs::create_dir_all(&self.data_dir) {
            eprintln!("[hub] could not create {:?}: {e}", self.data_dir);
            return;
        }
        let all: Vec<&Room> = self.rooms.values().collect();
        match serde_json::to_string_pretty(&all) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&self.rooms_file, s) {
                    eprintln!("[hub] could not write {:?}: {e}", self.rooms_file);
                }
            }
            Err(e) => eprintln!("[hub] serialize rooms failed: {e}"),
        }
    }
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "room".to_string()
    } else {
        trimmed
    }
}

fn read_rooms(file: &PathBuf) -> Vec<Room> {
    if file.exists() {
        match std::fs::read_to_string(file) {
            Ok(s) => match serde_json::from_str::<Vec<Room>>(&s) {
                Ok(v) => return v,
                Err(e) => eprintln!("[hub] could not parse {file:?}: {e}"),
            },
            Err(e) => eprintln!("[hub] could not read {file:?}: {e}"),
        }
    }
    Vec::new()
}

/// Parse "mqtt://host:port" into (host, port). Defaults to localhost:1883.
fn parse_broker(url: &str) -> (String, u16) {
    let stripped = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("tcp://"))
        .unwrap_or(url);
    let stripped = stripped.split('/').next().unwrap_or(stripped);
    match stripped.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
        None => (stripped.to_string(), 1883),
    }
}

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let data_dir = if config.data_dir.is_empty() {
        std::env::current_dir().unwrap_or_default().join("data")
    } else {
        PathBuf::from(&config.data_dir)
    };
    let rooms_file = data_dir.join("rooms.json");

    let (host, port) = parse_broker(&config.broker_url);
    let mut opts = MqttOptions::new("hub", host.as_str(), port);
    opts.set_keep_alive(Duration::from_secs(30));
    let (client, mut eventloop) = AsyncClient::new(opts, 128);

    let mut hub = Hub {
        client: client.clone(),
        config: config.clone(),
        registry: Registry::new(client.clone(), data_dir.clone()),
        alarms: AlarmEngine::new(client.clone()),
        rules: RuleEngine::default(),
        rooms: HashMap::new(),
        pairing_ads: HashMap::new(),
        data_dir,
        rooms_file: rooms_file.clone(),
    };

    // Restore rooms (retained) and the device registry from disk.
    for room in read_rooms(&rooms_file) {
        publish_json(&hub.client, &topics::room(&room.room_id), &room, true).await;
        hub.rooms.insert(room.room_id.clone(), room);
    }
    hub.registry.load().await;
    hub.publish_hub_status().await;

    // Subscriptions for discovery and control.
    let subs = [
        shared::topics::sub::all_presence(),
        shared::topics::sub::all_telemetry(),
        shared::topics::sub::all_pairing(),
        topics::commission_control(),
        topics::room_control(),
        topics::device_control(),
        topics::alarm_control(),
        topics::hub_control(),
    ];
    for s in subs {
        if let Err(e) = client.subscribe(s.clone(), rumqttc::QoS::AtLeastOnce).await {
            eprintln!("[hub] subscribe {s} failed: {e}");
        }
    }

    println!(
        "[hub] running on {host}:{port}. {} room(s), {} device(s). night={}. Data dir: {:?}",
        hub.rooms.len(),
        hub.registry.len(),
        if hub.config.force_night {
            "forced".to_string()
        } else {
            format!("{}-{}h", hub.config.night_start_hour, hub.config.night_end_hour)
        },
        hub.rooms_file.parent().unwrap_or(&hub.rooms_file)
    );

    let mut interval = tokio::time::interval(Duration::from_millis(hub.config.tick_ms));

    loop {
        tokio::select! {
            event = eventloop.poll() => match event {
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    hub.handle_publish(&p.topic, &p.payload).await;
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[hub] mqtt connection error: {e}; retrying in 2s");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },
            _ = interval.tick() => hub.tick().await,
            _ = tokio::signal::ctrl_c() => {
                println!("\n[hub] shutting down");
                break;
            }
        }
    }
    let _ = client.disconnect().await;
}
