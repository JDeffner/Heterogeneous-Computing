//! Central topic schema (single source of truth) for the centralized model.
//!
//!   smarthome/registry/<deviceId>            (retained, HUB)   -> DeviceRecord
//!   smarthome/devices/<deviceId>/presence    (retained, LWT)   -> "online"/"offline"
//!   smarthome/devices/<deviceId>/telemetry                     -> Telemetry
//!   smarthome/devices/<deviceId>/command                       -> SensorCommand
//!   smarthome/pairing/<serial>               (retained, LWT)   -> PairingAd
//!   smarthome/pairing/<serial>/commissioned                    -> CommissionedMessage
//!   smarthome/control/commission                               -> CommissionRequest
//!   smarthome/events/<eventId>                                 -> SituationEvent
//!   smarthome/alarms/<alarmId>               (retained, HUB)   -> Alarm
//!   smarthome/control/alarms                                   -> AlarmControl
//!   smarthome/control/rooms                                    -> RoomControl
//!   smarthome/control/devices                                  -> DeviceControl
//!   smarthome/rooms/<roomId>                 (retained, HUB)   -> Room
//!   smarthome/hub/status                     (retained, HUB)   -> HubStatus
//!   smarthome/control/hub                                      -> HubControl

pub const ROOT: &str = "smarthome";

pub fn registry(device_id: &str) -> String {
    format!("{ROOT}/registry/{device_id}")
}
pub fn presence(device_id: &str) -> String {
    format!("{ROOT}/devices/{device_id}/presence")
}
pub fn telemetry(device_id: &str) -> String {
    format!("{ROOT}/devices/{device_id}/telemetry")
}
pub fn command(device_id: &str) -> String {
    format!("{ROOT}/devices/{device_id}/command")
}
pub fn pairing(serial: &str) -> String {
    format!("{ROOT}/pairing/{serial}")
}
pub fn pairing_commissioned(serial: &str) -> String {
    format!("{ROOT}/pairing/{serial}/commissioned")
}
pub fn commission_control() -> String {
    format!("{ROOT}/control/commission")
}
pub fn event(event_id: &str) -> String {
    format!("{ROOT}/events/{event_id}")
}
pub fn alarm(alarm_id: &str) -> String {
    format!("{ROOT}/alarms/{alarm_id}")
}
pub fn alarm_control() -> String {
    format!("{ROOT}/control/alarms")
}
pub fn room(room_id: &str) -> String {
    format!("{ROOT}/rooms/{room_id}")
}
pub fn room_control() -> String {
    format!("{ROOT}/control/rooms")
}
pub fn device_control() -> String {
    format!("{ROOT}/control/devices")
}
pub fn hub_status() -> String {
    format!("{ROOT}/hub/status")
}
pub fn hub_control() -> String {
    format!("{ROOT}/control/hub")
}

/// Wildcard subscriptions for discovery and processing.
pub mod sub {
    use super::ROOT;
    pub fn all_registry() -> String {
        format!("{ROOT}/registry/+")
    }
    pub fn all_presence() -> String {
        format!("{ROOT}/devices/+/presence")
    }
    pub fn all_telemetry() -> String {
        format!("{ROOT}/devices/+/telemetry")
    }
    pub fn all_pairing() -> String {
        // Only the ads, not the .../commissioned replies (single level wildcard).
        format!("{ROOT}/pairing/+")
    }
    pub fn all_events() -> String {
        format!("{ROOT}/events/+")
    }
    pub fn all_alarms() -> String {
        format!("{ROOT}/alarms/+")
    }
    pub fn all_rooms() -> String {
        format!("{ROOT}/rooms/+")
    }
}

/// Extract `<id>` from `smarthome/registry/<id>`.
pub fn device_id_from_registry(topic: &str) -> Option<&str> {
    topic.strip_prefix("smarthome/registry/").filter(|s| !s.contains('/'))
}

/// Extract `<id>` from `smarthome/devices/<id>/...`.
pub fn device_id_from_topic(topic: &str) -> Option<&str> {
    let rest = topic.strip_prefix("smarthome/devices/")?;
    rest.split('/').next().filter(|s| !s.is_empty())
}

/// Extract `<serial>` from `smarthome/pairing/<serial>` (NOT the commissioned reply).
pub fn serial_from_pairing(topic: &str) -> Option<&str> {
    topic.strip_prefix("smarthome/pairing/").filter(|s| !s.contains('/'))
}

/// Extract `<roomId>` from `smarthome/rooms/<roomId>`.
pub fn room_id_from_topic(topic: &str) -> Option<&str> {
    topic.strip_prefix("smarthome/rooms/").filter(|s| !s.contains('/'))
}

/// Extract `<alarmId>` from `smarthome/alarms/<alarmId>`.
pub fn alarm_id_from_topic(topic: &str) -> Option<&str> {
    topic.strip_prefix("smarthome/alarms/").filter(|s| !s.contains('/'))
}
