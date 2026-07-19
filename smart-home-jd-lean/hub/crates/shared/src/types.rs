//! Data types that travel over MQTT as JSON. Field names are camelCase to match
//! the established wire contract (browser dashboard + ESP32 firmware).
use serde::{Deserialize, Serialize};

/// The five supported device kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SensorType {
    Door,
    Motion,
    Bed,
    Stove,
    Sos,
}

impl SensorType {
    /// Human-readable label (single source of truth).
    pub fn label(self) -> &'static str {
        match self {
            SensorType::Door => "Door / contact sensor",
            SensorType::Motion => "Motion sensor",
            SensorType::Bed => "Bed occupancy sensor",
            SensorType::Stove => "Stove guard",
            SensorType::Sos => "SOS panic button",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            SensorType::Door => "door",
            SensorType::Motion => "motion",
            SensorType::Bed => "bed",
            SensorType::Stove => "stove",
            SensorType::Sos => "sos",
        }
    }
}

/// Whether a device evolves on its own (auto) or is driven manually.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SensorMode {
    Auto,
    Manual,
}

/// Radio / network transport a device uses to reach the hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Zigbee,
    #[serde(rename = "Z-Wave")]
    ZWave,
    #[serde(rename = "Wi-Fi")]
    WiFi,
    Thread,
    #[serde(rename = "BLE")]
    Ble,
}

/// Where a device draws power from (affects the battery model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerSource {
    Battery,
    Mains,
}

/// Physical location of a device, assigned by "scanning" a room's QR code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    pub building: String,
    pub floor: String,
    pub room: String,
    /// Id of the registered room this location came from (the room's QR code).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub room_id: Option<String>,
}

/// A room in the building; its `roomId` is also the content of its QR code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub room_id: String,
    pub building: String,
    pub floor: String,
    pub room: String,
    pub created_at: String,
}

/// Liveness of a device record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Online,
    Offline,
}

/// The hub's persisted descriptor for a device. Published RETAINED on
/// `smarthome/registry/<deviceId>` so every UI and device sees the whole fleet,
/// including offline devices.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub device_id: String,
    #[serde(rename = "type")]
    pub kind: SensorType,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<Location>,
    pub serial_number: String,
    pub manufacturer: String,
    pub model: String,
    pub protocol: Protocol,
    pub power_source: PowerSource,
    pub firmware: String,
    pub capabilities: Vec<String>,
    pub status: Status,
    pub registered_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_seen: Option<String>,
}

/// Sensor-specific state, tagged by `kind` (matches the TS discriminated union).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SensorState {
    Door {
        open: bool,
    },
    Motion {
        motion: bool,
        lux: i64,
    },
    Bed {
        occupied: bool,
        #[serde(rename = "heartRate")]
        heart_rate: i64,
    },
    Stove {
        on: bool,
        #[serde(rename = "temperatureC")]
        temperature_c: i64,
    },
    Sos {
        pressed: bool,
    },
}

/// One telemetry sample produced by a live device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Telemetry {
    pub device_id: String,
    #[serde(rename = "type")]
    pub kind: SensorType,
    pub state: SensorState,
    pub mode: SensorMode,
    pub battery: i64,
    pub rssi: i64,
    pub ts: String,
}

/// Command sent to a single live device. Unset fields are ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorCommand {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mode: Option<SensorMode>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub online: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub motion: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub occupied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pressed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub temperature_c: Option<i64>,
}

// ----- Onboarding ----------------------------------------------------------

/// Pairing advertisement: a factory-fresh, unprovisioned device announces only
/// its hardware identity (retained on `smarthome/pairing/<serial>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingAd {
    pub serial: String,
    #[serde(rename = "type")]
    pub kind: SensorType,
    pub manufacturer: String,
    pub model: String,
    pub protocol: Protocol,
    pub power_source: PowerSource,
    pub firmware: String,
    pub pairing_pin: String,
    pub advertised_at: String,
}

/// App/console -> hub: commission a waiting device (assign id/name/room).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommissionRequest {
    pub serial: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    #[serde(default)]
    pub room_id: Option<String>,
}

/// Hub -> device: you are commissioned; here is your operational identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommissionedMessage {
    pub serial: String,
    pub device_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub location: Option<Location>,
}

// ----- Hub status / control ------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubStatus {
    pub night_start_hour: u32,
    pub night_end_hour: u32,
    pub force_night: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubControl {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub force_night: Option<bool>,
}

// ----- Rooms / device control ----------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum RoomControl {
    Create {
        building: String,
        floor: String,
        room: String,
    },
    Delete {
        #[serde(rename = "roomId")]
        room_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum DeviceControl {
    Assign {
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(rename = "roomId")]
        room_id: Option<String>,
    },
    Remove {
        #[serde(rename = "deviceId")]
        device_id: String,
    },
}

// ----- Resident ------------------------------------------------------------

/// One person in the resident's escalation chain (ordered by priority).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub name: String,
    pub role: String,
    pub phone: String,
}

/// Profile of the person living in the home. Owned by the hub (retained on
/// `smarthome/resident`, persisted to disk); the emergency call sheets are
/// generated from it, so it holds exactly what a 112 dispatcher asks for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resident {
    pub name: String,
    pub year_of_birth: i32,
    pub conditions: Vec<String>,
    pub medications: Vec<String>,
    pub notes: String,
    pub address: String,
    pub access_info: String,
    pub contacts: Vec<Contact>,
    pub updated_at: String,
}

// ----- Situations / alarms -------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKey {
    StoveOnNoMotion,
    BedLeftAtNightNoReturn,
    DoorOpenNoMotion,
    DoorOpenAtNight,
    PossibleFall,
    Inactivity,
    SosPressed,
    DeviceOffline,
    AlarmEscalated,
}

/// One entry of the sensor-evidence timeline attached to situations/alarms.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceItem {
    pub ts: String,
    pub text: String,
}

/// A prepared emergency call: which service to dial and exactly what to say.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSheet {
    pub number: String,
    pub service: String,
    pub reason: String,
    pub script: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SituationEvent {
    pub event_id: String,
    pub rule: RuleKey,
    pub severity: Severity,
    pub message: String,
    pub location: Location,
    pub device_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cleared: Option<bool>,
    pub detected_at: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub evidence: Vec<EvidenceItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlarmStatus {
    Active,
    Acknowledged,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alarm {
    pub alarm_id: String,
    pub rule: RuleKey,
    pub severity: Severity,
    pub message: String,
    pub location: Location,
    pub device_ids: Vec<String>,
    pub status: AlarmStatus,
    pub raised_at: String,
    pub updated_at: String,
    /// What the caregiver should do, step by step, most urgent first.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub recommended_actions: Vec<String>,
    /// Sensor observations that led to this alarm (oldest first).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub evidence: Vec<EvidenceItem>,
    /// Prepared emergency call, present on alarms where calling 112 may be needed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub call_sheet: Option<CallSheet>,
    /// Set by the hub when the alarm stayed unacknowledged past the ack timeout.
    #[serde(default)]
    pub escalated: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub call_logged_at: Option<String>,
}

/// UI -> hub: `action` is one of "resolve", "ack", "call_logged".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmControl {
    pub alarm_id: String,
    pub action: String,
}
