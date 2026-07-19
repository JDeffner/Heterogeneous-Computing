//! Hub rule thresholds, overridable via environment variables. Values are small
//! (seconds) so the demo reacts quickly; a real install would use minutes.
use std::env;

fn num(name: &str, def: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(def)
}
fn boolean(name: &str, def: bool) -> bool {
    match env::var(name) {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        Err(_) => def,
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub tick_ms: u64,
    pub stove_on_seconds: u64,
    pub stove_no_motion_seconds: u64,
    pub door_open_seconds: u64,
    pub door_no_motion_seconds: u64,
    pub bed_absence_seconds: u64,
    /// Abrupt end of movement: active less than this long ago, then silence.
    pub fall_window_seconds: u64,
    pub fall_silence_seconds: u64,
    /// Daytime "nothing at all is happening" watchdog.
    pub inactivity_seconds: u64,
    /// Critical alarm unacknowledged this long -> escalate to "call now".
    pub ack_timeout_seconds: u64,
    pub night_start_hour: u32,
    pub night_end_hour: u32,
    /// Demo override: forces "night" so the bed rule is demonstrable any time.
    pub force_night: bool,
    pub data_dir: String,
    pub broker_url: String,
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            tick_ms: num("EVAL_TICK_MS", 2000),
            stove_on_seconds: num("STOVE_ON_SECONDS", 12),
            stove_no_motion_seconds: num("STOVE_NO_MOTION_SECONDS", 12),
            door_open_seconds: num("DOOR_OPEN_SECONDS", 10),
            door_no_motion_seconds: num("DOOR_NO_MOTION_SECONDS", 12),
            bed_absence_seconds: num("BED_ABSENCE_SECONDS", 15),
            fall_window_seconds: num("FALL_WINDOW_SECONDS", 30),
            fall_silence_seconds: num("FALL_SILENCE_SECONDS", 20),
            inactivity_seconds: num("INACTIVITY_SECONDS", 90),
            ack_timeout_seconds: num("ACK_TIMEOUT_SECONDS", 25),
            night_start_hour: num("NIGHT_START", 22) as u32,
            night_end_hour: num("NIGHT_END", 7) as u32,
            force_night: boolean("FORCE_NIGHT", false),
            data_dir: env::var("DATA_DIR").unwrap_or_default(),
            broker_url: env::var("MQTT_URL").unwrap_or_else(|_| "mqtt://localhost:1883".into()),
        }
    }
}
