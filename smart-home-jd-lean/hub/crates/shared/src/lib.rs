//! Shared contracts between the loosely coupled components.
//!
//! Every component (hub, dashboard, device firmware) communicates only over MQTT
//! with JSON payloads. These types are the single source of truth for the wire
//! format; the JSON shape is identical to what the ESP32 firmware emits and what
//! the browser dashboard consumes, so the field names use camelCase to match.
pub mod topics;
pub mod types;
pub mod util;

pub use types::*;
