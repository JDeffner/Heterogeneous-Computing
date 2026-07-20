//! Simulated devices: runs the five ESP32 roles as plain MQTT clients so the
//! whole system can be demoed without hardware. Mirrors the firmware behaviour
//! exactly on the wire: retained pairing ad with a clearing last-will, waits to
//! be commissioned, then retained presence + telemetry + command handling
//! (`firmware/main/device_runtime.c` / `provisioning.c` are the reference).
//!
//!   sim                 run all five roles
//!   sim door,stove      run a subset
//!
//! Broker via MQTT_URL (default mqtt://localhost:1883). Identities persist in
//! DATA_DIR/sim-devices.json (the firmware's NVS equivalent), so a restarted
//! sim reconnects as the same devices instead of re-advertising fresh ones.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use shared::{
    topics, util, CommissionedMessage, PairingAd, PowerSource, Protocol, SensorCommand,
    SensorMode, SensorState, SensorType, Telemetry,
};
use tokio::sync::Mutex;

const ROLES: [SensorType; 5] = [
    SensorType::Door,
    SensorType::Motion,
    SensorType::Bed,
    SensorType::Stove,
    SensorType::Sos,
];

// ----- Catalog (mirror of firmware/main/sensors/catalog.c) ------------------

struct CatalogEntry {
    manufacturer: &'static str,
    model: &'static str,
    protocol: Protocol,
    power_source: PowerSource,
    firmware: &'static str,
    nominal_rssi: i64,
    report_interval_ms: u64,
}

fn catalog(kind: SensorType) -> CatalogEntry {
    match kind {
        SensorType::Door => CatalogEntry {
            manufacturer: "Aqara",
            model: "DW-S100",
            protocol: Protocol::Zigbee,
            power_source: PowerSource::Battery,
            firmware: "esp-2.1.0",
            nominal_rssi: -62,
            report_interval_ms: 2500,
        },
        SensorType::Motion => CatalogEntry {
            manufacturer: "Philips Hue",
            model: "SML-002",
            protocol: Protocol::Zigbee,
            power_source: PowerSource::Battery,
            firmware: "esp-2.1.0",
            nominal_rssi: -58,
            report_interval_ms: 2500,
        },
        SensorType::Bed => CatalogEntry {
            manufacturer: "Emfit",
            model: "QS-Care",
            protocol: Protocol::WiFi,
            power_source: PowerSource::Mains,
            firmware: "esp-3.0.4",
            nominal_rssi: -47,
            report_interval_ms: 2500,
        },
        SensorType::Stove => CatalogEntry {
            manufacturer: "Inirv",
            model: "Guard-Z",
            protocol: Protocol::ZWave,
            power_source: PowerSource::Mains,
            firmware: "esp-1.4.2",
            nominal_rssi: -51,
            report_interval_ms: 2500,
        },
        SensorType::Sos => CatalogEntry {
            manufacturer: "CareTech",
            model: "SOS-Pendant",
            protocol: Protocol::Ble,
            power_source: PowerSource::Battery,
            firmware: "esp-1.0.1",
            nominal_rssi: -70,
            report_interval_ms: 2500,
        },
    }
}

/// `SN-<Model without punctuation>-<6 hex>` (mirror of catalog_make_serial).
fn make_serial(kind: SensorType) -> String {
    let model: String = catalog(kind).model.chars().filter(|c| c.is_alphanumeric()).collect();
    format!("SN-{model}-{:06x}", rand::random::<u32>() & 0xffffff)
}

fn frand() -> f64 {
    rand::random::<f64>()
}

// ----- Sensor model (mirror of firmware/main/sensors/sensor.c) --------------

struct Sensor {
    kind: SensorType,
    door_open: bool,
    motion: bool,
    lux: f64,
    bed_occupied: bool,
    heart_rate: i64,
    stove_on: bool,
    stove_temp_c: f64,
    sos_pressed: bool,
}

impl Sensor {
    fn new(kind: SensorType) -> Self {
        Sensor {
            kind,
            door_open: false,
            motion: false,
            lux: if kind == SensorType::Motion { 120.0 } else { 0.0 },
            bed_occupied: kind == SensorType::Bed,
            heart_rate: if kind == SensorType::Bed { 62 } else { 0 },
            stove_on: false,
            stove_temp_c: 20.0,
            sos_pressed: false,
        }
    }

    fn simulate_step(&mut self) {
        match self.kind {
            SensorType::Door => {
                if self.door_open {
                    if frand() < 0.5 {
                        self.door_open = false;
                    }
                } else if frand() < 0.3 {
                    self.door_open = true;
                }
            }
            SensorType::Motion => {
                self.motion = frand() < 0.22;
                let target = if self.motion { 240.0 } else { 90.0 };
                self.lux = (self.lux + (target - self.lux) * 0.3 + (frand() - 0.5) * 20.0).max(0.0);
            }
            SensorType::Bed => {
                if self.bed_occupied {
                    if frand() < 0.15 {
                        self.bed_occupied = false;
                    }
                } else if frand() < 0.4 {
                    self.bed_occupied = true;
                }
                self.heart_rate =
                    if self.bed_occupied { (58.0 + frand() * 12.0).round() as i64 } else { 0 };
            }
            SensorType::Stove => {
                if self.stove_on {
                    if frand() < 0.25 {
                        self.stove_on = false;
                    }
                } else if frand() < 0.25 {
                    self.stove_on = true;
                }
                if self.stove_on {
                    self.stove_temp_c = (self.stove_temp_c + 18.0 + frand() * 10.0).min(230.0);
                } else {
                    self.stove_temp_c = (self.stove_temp_c - 12.0).max(20.0);
                }
            }
            SensorType::Sos => {
                if self.sos_pressed {
                    if frand() < 0.6 {
                        self.sos_pressed = false;
                    }
                } else if frand() < 0.03 {
                    self.sos_pressed = true;
                }
            }
        }
    }

    fn apply_command(&mut self, cmd: &SensorCommand) {
        match self.kind {
            SensorType::Door => {
                if let Some(open) = cmd.open {
                    self.door_open = open;
                }
            }
            SensorType::Motion => {
                if let Some(motion) = cmd.motion {
                    self.motion = motion;
                }
            }
            SensorType::Bed => {
                if let Some(occupied) = cmd.occupied {
                    self.bed_occupied = occupied;
                    self.heart_rate = if occupied { 62 } else { 0 };
                }
            }
            SensorType::Stove => {
                if let Some(on) = cmd.on {
                    self.stove_on = on;
                }
                if let Some(t) = cmd.temperature_c {
                    self.stove_temp_c = t as f64;
                }
            }
            SensorType::Sos => {
                if let Some(pressed) = cmd.pressed {
                    self.sos_pressed = pressed;
                }
            }
        }
    }

    fn state(&self) -> SensorState {
        match self.kind {
            SensorType::Door => SensorState::Door { open: self.door_open },
            SensorType::Motion => SensorState::Motion {
                motion: self.motion,
                lux: self.lux.round() as i64,
            },
            SensorType::Bed => SensorState::Bed {
                occupied: self.bed_occupied,
                heart_rate: self.heart_rate,
            },
            SensorType::Stove => SensorState::Stove {
                on: self.stove_on,
                temperature_c: self.stove_temp_c.round() as i64,
            },
            SensorType::Sos => SensorState::Sos { pressed: self.sos_pressed },
        }
    }
}

// ----- Persisted identities (the firmware's NVS equivalent) -----------------

#[derive(Clone, Serialize, Deserialize)]
struct SavedIdentity {
    serial: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    device_id: Option<String>,
}

struct IdentityStore {
    path: std::path::PathBuf,
    map: HashMap<String, SavedIdentity>,
}

impl IdentityStore {
    fn load(data_dir: &str) -> Self {
        let path = std::path::Path::new(data_dir).join("sim-devices.json");
        let map = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        IdentityStore { path, map }
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_vec_pretty(&self.map) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&self.path, bytes) {
                    eprintln!("[sim] could not write {:?}: {e}", self.path);
                }
            }
            Err(e) => eprintln!("[sim] serialize identities: {e}"),
        }
    }
}

// ----- MQTT plumbing --------------------------------------------------------

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

async fn publish_json<T: Serialize>(client: &AsyncClient, topic: &str, payload: &T, retain: bool) {
    match serde_json::to_vec(payload) {
        Ok(bytes) => {
            if let Err(e) = client.publish(topic, QoS::AtLeastOnce, retain, bytes).await {
                eprintln!("[sim] publish failed on {topic}: {e}");
            }
        }
        Err(e) => eprintln!("[sim] serialize error on {topic}: {e}"),
    }
}

/// Advertise on the pairing topic (retained, with a clearing last-will) and
/// block until the hub commissions us. Returns the minted device id.
async fn pair(host: &str, port: u16, kind: SensorType, serial: &str) -> String {
    let cat = catalog(kind);
    let ad_topic = topics::pairing(serial);
    let ack_topic = topics::pairing_commissioned(serial);

    let mut opts = MqttOptions::new(format!("pairing-{serial}"), host, port);
    opts.set_keep_alive(Duration::from_secs(10));
    opts.set_last_will(LastWill::new(&ad_topic, "", QoS::AtLeastOnce, true));
    let (client, mut eventloop) = AsyncClient::new(opts, 64);

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                let _ = client.subscribe(&ack_topic, QoS::AtLeastOnce).await;
                let ad = PairingAd {
                    serial: serial.to_string(),
                    kind,
                    manufacturer: cat.manufacturer.to_string(),
                    model: cat.model.to_string(),
                    protocol: cat.protocol,
                    power_source: cat.power_source,
                    firmware: cat.firmware.to_string(),
                    pairing_pin: format!("{:04}", 1000 + rand::random::<u32>() % 9000),
                    advertised_at: util::now_iso(),
                };
                publish_json(&client, &ad_topic, &ad, true).await;
                println!("[sim] {} advertising (serial {serial}), waiting to be commissioned ...", kind.as_str());
            }
            Ok(Event::Incoming(Packet::Publish(p))) if p.topic == ack_topic => {
                if let Ok(msg) = serde_json::from_slice::<CommissionedMessage>(&p.payload) {
                    println!("[sim] {} commissioned as {} ({})", kind.as_str(), msg.device_id, msg.name);
                    let _ = client.disconnect().await;
                    return msg.device_id;
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("[sim] {} pairing mqtt error: {e}", kind.as_str());
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// Normal operation: retained presence with offline last-will, periodic
/// telemetry, command handling (mirror of device_runtime.c).
async fn run_provisioned(host: &str, port: u16, kind: SensorType, device_id: &str) {
    let cat = catalog(kind);
    let t_presence = topics::presence(device_id);
    let t_telemetry = topics::telemetry(device_id);
    let t_command = topics::command(device_id);

    let mut opts = MqttOptions::new(format!("sensor-{device_id}"), host, port);
    opts.set_keep_alive(Duration::from_secs(10));
    opts.set_last_will(LastWill::new(&t_presence, "offline", QoS::AtLeastOnce, true));
    let (client, mut eventloop) = AsyncClient::new(opts, 64);

    let mut sensor = Sensor::new(kind);
    let mut manual = false;
    let mut link_up = true;
    let mut battery: f64 = match cat.power_source {
        PowerSource::Battery => 70.0 + (rand::random::<u32>() % 30) as f64,
        PowerSource::Mains => 100.0,
    };
    let mut rssi = cat.nominal_rssi as f64;

    let telemetry = |sensor: &Sensor, manual: bool, battery: f64, rssi: f64| Telemetry {
        device_id: device_id.to_string(),
        kind,
        state: sensor.state(),
        mode: if manual { SensorMode::Manual } else { SensorMode::Auto },
        battery: battery.round() as i64,
        rssi: rssi.round() as i64,
        ts: util::now_iso(),
    };

    let mut tick = tokio::time::interval(Duration::from_millis(cat.report_interval_ms));
    loop {
        tokio::select! {
            ev = eventloop.poll() => match ev {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    if link_up {
                        let _ = client.publish(&t_presence, QoS::AtLeastOnce, true, "online").await;
                    }
                    let _ = client.subscribe(&t_command, QoS::AtLeastOnce).await;
                    println!("[sim] online as {} ({device_id})", kind.as_str());
                }
                Ok(Event::Incoming(Packet::Publish(p))) if p.topic == t_command => {
                    if let Ok(cmd) = serde_json::from_slice::<SensorCommand>(&p.payload) {
                        if let Some(mode) = cmd.mode {
                            manual = mode == SensorMode::Manual;
                        }
                        if let Some(online) = cmd.online {
                            if online != link_up {
                                link_up = online;
                                let msg = if link_up { "online" } else { "offline" };
                                let _ = client.publish(&t_presence, QoS::AtLeastOnce, true, msg).await;
                                println!("[sim] {} simulated link {}", kind.as_str(), if link_up { "UP" } else { "DOWN" });
                            }
                            if !link_up {
                                continue; // unreachable: ignore the rest, stay silent
                            }
                        }
                        sensor.apply_command(&cmd);
                        publish_json(&client, &t_telemetry, &telemetry(&sensor, manual, battery, rssi), false).await;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[sim] {} mqtt error: {e}", kind.as_str());
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            },
            _ = tick.tick() => {
                if !link_up {
                    continue;
                }
                if !manual {
                    sensor.simulate_step();
                }
                if cat.power_source == PowerSource::Battery {
                    battery = (battery - frand() * 0.4).max(1.0);
                }
                rssi = ((rssi + (frand() - 0.5) * 6.0 + (cat.nominal_rssi as f64 - rssi) * 0.2)
                    .max(-95.0))
                    .min(-35.0);
                publish_json(&client, &t_telemetry, &telemetry(&sensor, manual, battery, rssi), false).await;
            }
        }
    }
}

async fn run_role(host: String, port: u16, kind: SensorType, store: Arc<Mutex<IdentityStore>>) {
    let key = kind.as_str().to_string();
    let (serial, device_id) = {
        let mut store = store.lock().await;
        let entry = store.map.entry(key.clone()).or_insert_with(|| SavedIdentity {
            serial: make_serial(kind),
            device_id: None,
        });
        let out = (entry.serial.clone(), entry.device_id.clone());
        store.save();
        out
    };

    let device_id = match device_id {
        Some(id) => id,
        None => {
            let id = pair(&host, port, kind, &serial).await;
            let mut store = store.lock().await;
            if let Some(entry) = store.map.get_mut(&key) {
                entry.device_id = Some(id.clone());
            }
            store.save();
            id
        }
    };

    run_provisioned(&host, port, kind, &device_id).await;
}

#[tokio::main]
async fn main() {
    let roles: Vec<SensorType> = match std::env::args().nth(1) {
        None => ROLES.to_vec(),
        Some(arg) => {
            let mut out = Vec::new();
            for name in arg.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                match ROLES.iter().find(|r| r.as_str() == name) {
                    Some(r) => out.push(*r),
                    None => {
                        eprintln!("usage: sim [door,motion,bed,stove,sos]");
                        std::process::exit(2);
                    }
                }
            }
            out
        }
    };

    let broker = std::env::var("MQTT_URL").unwrap_or_else(|_| "mqtt://localhost:1883".into());
    let (host, port) = parse_broker(&broker);
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "data".into());
    let store = Arc::new(Mutex::new(IdentityStore::load(&data_dir)));

    println!(
        "[sim] {} simulated device(s) -> {host}:{port} (identities in {data_dir}/sim-devices.json)",
        roles.len()
    );
    let tasks: Vec<_> = roles
        .into_iter()
        .map(|kind| tokio::spawn(run_role(host.clone(), port, kind, store.clone())))
        .collect();
    for t in tasks {
        let _ = t.await;
    }
}
