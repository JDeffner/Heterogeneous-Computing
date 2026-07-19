//! The hub's persisted DEVICE REGISTRY: the single owner of every device's
//! static descriptor. Each record is published RETAINED on
//! `smarthome/registry/<id>` and persisted to `<dataDir>/devices.json`.
use std::collections::HashMap;
use std::path::PathBuf;

use rumqttc::AsyncClient;
use shared::util::{new_id, now_iso};
use shared::{topics, DeviceRecord, Location, SensorType, Status};

use crate::catalog::{catalog, make_serial};
use crate::mqtt::{clear_retained, publish_json};

fn derive_name(t: SensorType, location: Option<&Location>) -> String {
    match location {
        Some(l) => format!("{} - {}", t.label(), l.room),
        None => format!("{} (unassigned)", t.label()),
    }
}

pub struct Registry {
    records: HashMap<String, DeviceRecord>,
    client: AsyncClient,
    file: PathBuf,
    data_dir: PathBuf,
}

#[derive(Default)]
pub struct CreateOpts {
    pub device_id: Option<String>,
    pub serial: Option<String>,
    pub location: Option<Location>,
    pub name: Option<String>,
}

impl Registry {
    pub fn new(client: AsyncClient, data_dir: PathBuf) -> Self {
        let file = data_dir.join("devices.json");
        Registry {
            records: HashMap::new(),
            client,
            file,
            data_dir,
        }
    }

    /// Load persisted records, mark them offline, republish (retained).
    pub async fn load(&mut self) {
        let mut stored: Vec<DeviceRecord> = Vec::new();
        if self.file.exists() {
            match std::fs::read_to_string(&self.file) {
                Ok(s) => match serde_json::from_str::<Vec<DeviceRecord>>(&s) {
                    Ok(v) => stored = v,
                    Err(e) => eprintln!("[hub] could not parse {:?}: {e}", self.file),
                },
                Err(e) => eprintln!("[hub] could not read {:?}: {e}", self.file),
            }
        }
        for mut rec in stored {
            rec.status = Status::Offline; // a retained presence message flips it back
            let id = rec.device_id.clone();
            self.publish(&rec).await;
            self.records.insert(id, rec);
        }
        println!("[hub] registry loaded: {} device(s)", self.records.len());
    }

    pub fn all(&self) -> impl Iterator<Item = &DeviceRecord> {
        self.records.values()
    }
    pub fn get(&self, id: &str) -> Option<&DeviceRecord> {
        self.records.get(id)
    }
    pub fn len(&self) -> usize {
        self.records.len()
    }

    async fn create(&mut self, t: SensorType, opts: CreateOpts) -> DeviceRecord {
        let cat = catalog(t);
        let mut device_id = opts
            .device_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| new_id(t.as_str()));
        if self.records.contains_key(&device_id) {
            device_id = new_id(t.as_str());
        }
        let name = opts
            .name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| derive_name(t, opts.location.as_ref()));
        let record = DeviceRecord {
            device_id: device_id.clone(),
            kind: t,
            name,
            location: opts.location,
            serial_number: opts.serial.unwrap_or_else(|| make_serial(t)),
            manufacturer: cat.manufacturer.to_string(),
            model: cat.model.to_string(),
            protocol: cat.protocol,
            power_source: cat.power_source,
            firmware: cat.firmware.to_string(),
            capabilities: cat.capabilities.iter().map(|s| s.to_string()).collect(),
            status: Status::Offline,
            registered_at: now_iso(),
            last_seen: None,
        };
        self.records.insert(device_id, record.clone());
        self.persist();
        self.publish(&record).await;
        record
    }

    /// Direct registration (auto-commissioned) - used by `ctl` / scripts.
    pub async fn register(
        &mut self,
        t: SensorType,
        desired_id: Option<String>,
        location: Option<Location>,
    ) -> DeviceRecord {
        let rec = self
            .create(
                t,
                CreateOpts {
                    device_id: desired_id,
                    location,
                    ..Default::default()
                },
            )
            .await;
        println!("[hub] registered {} {}", t.as_str(), rec.device_id);
        rec
    }

    /// Commission a device that advertised itself in pairing mode.
    pub async fn commission(
        &mut self,
        serial: String,
        kind: SensorType,
        name: Option<String>,
        location: Option<Location>,
    ) -> DeviceRecord {
        let rec = self
            .create(
                kind,
                CreateOpts {
                    serial: Some(serial.clone()),
                    name,
                    location,
                    ..Default::default()
                },
            )
            .await;
        println!(
            "[hub] commissioned {} {} (serial {})",
            kind.as_str(),
            rec.device_id,
            serial
        );
        rec
    }

    /// Update liveness. Returns true if the status actually changed.
    pub async fn set_status(&mut self, id: &str, status: Status) -> bool {
        let Some(rec) = self.records.get_mut(id) else {
            return false;
        };
        let changed = rec.status != status;
        rec.status = status;
        rec.last_seen = Some(now_iso());
        if changed {
            let rec = rec.clone();
            self.persist();
            self.publish(&rec).await;
        }
        changed
    }

    /// Assign (or, with None, clear) a device's room.
    pub async fn assign(&mut self, id: &str, location: Option<Location>) {
        let Some(rec) = self.records.get_mut(id) else {
            return;
        };
        rec.location = location;
        rec.name = derive_name(rec.kind, rec.location.as_ref());
        let rec = rec.clone();
        self.persist();
        self.publish(&rec).await;
        println!(
            "[hub] {id} assigned -> {}",
            rec.location.as_ref().map(|l| l.room.as_str()).unwrap_or("(unassigned)")
        );
    }

    pub async fn remove(&mut self, id: &str) {
        if self.records.remove(id).is_none() {
            return;
        }
        self.persist();
        clear_retained(&self.client, &topics::registry(id)).await;
        clear_retained(&self.client, &topics::presence(id)).await;
        println!("[hub] removed {id}");
    }

    async fn publish(&self, rec: &DeviceRecord) {
        publish_json(&self.client, &topics::registry(&rec.device_id), rec, true).await;
    }

    fn persist(&self) {
        if let Err(e) = std::fs::create_dir_all(&self.data_dir) {
            eprintln!("[hub] could not create {:?}: {e}", self.data_dir);
            return;
        }
        let all: Vec<&DeviceRecord> = self.records.values().collect();
        match serde_json::to_string_pretty(&all) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&self.file, s) {
                    eprintln!("[hub] could not write {:?}: {e}", self.file);
                }
            }
            Err(e) => eprintln!("[hub] serialize registry failed: {e}"),
        }
    }
}
