//! Device catalog: the "spec sheet" of every supported device type, mirrored
//! from the firmware catalog so the registry records stay faithful.
use rand::Rng;
use shared::{PowerSource, Protocol, SensorType};

pub struct CatalogEntry {
    pub manufacturer: &'static str,
    pub model: &'static str,
    pub protocol: Protocol,
    pub power_source: PowerSource,
    pub firmware: &'static str,
    pub capabilities: &'static [&'static str],
    pub nominal_rssi: i64,
    pub report_interval_ms: u64,
}

pub fn catalog(t: SensorType) -> CatalogEntry {
    match t {
        SensorType::Door => CatalogEntry {
            manufacturer: "Aqara",
            model: "DW-S100",
            protocol: Protocol::Zigbee,
            power_source: PowerSource::Battery,
            firmware: "esp-2.1.0",
            capabilities: &["door.open"],
            nominal_rssi: -62,
            report_interval_ms: 2500,
        },
        SensorType::Motion => CatalogEntry {
            manufacturer: "Philips Hue",
            model: "SML-002",
            protocol: Protocol::Zigbee,
            power_source: PowerSource::Battery,
            firmware: "esp-2.1.0",
            capabilities: &["motion.detected", "motion.lux"],
            nominal_rssi: -58,
            report_interval_ms: 2500,
        },
        SensorType::Bed => CatalogEntry {
            manufacturer: "Emfit",
            model: "QS-Care",
            protocol: Protocol::WiFi,
            power_source: PowerSource::Mains,
            firmware: "esp-3.0.4",
            capabilities: &["bed.occupied", "bed.heartRate"],
            nominal_rssi: -47,
            report_interval_ms: 2500,
        },
        SensorType::Stove => CatalogEntry {
            manufacturer: "Inirv",
            model: "Guard-Z",
            protocol: Protocol::ZWave,
            power_source: PowerSource::Mains,
            firmware: "esp-1.4.2",
            capabilities: &["stove.on", "stove.temp"],
            nominal_rssi: -51,
            report_interval_ms: 2500,
        },
        SensorType::Sos => CatalogEntry {
            manufacturer: "CareTech",
            model: "SOS-Pendant",
            protocol: Protocol::Ble,
            power_source: PowerSource::Battery,
            firmware: "esp-1.0.1",
            capabilities: &["sos.pressed"],
            nominal_rssi: -70,
            report_interval_ms: 2500,
        },
    }
}

/// Factory serial, e.g. "SN-DWS100-3f9ac2" (stable per physical device).
pub fn make_serial(t: SensorType) -> String {
    let model: String = catalog(t)
        .model
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let hex: u32 = rand::thread_rng().gen_range(0..=0xff_ffff);
    format!("SN-{model}-{hex:06x}")
}
