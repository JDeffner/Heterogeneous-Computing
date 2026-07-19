//! Hub rule engine. Consumes presence + telemetry, tracks per-device/per-room
//! state, and derives "situations" (edge-triggered: emitted once per occurrence,
//! with a matching `cleared` event on recovery). Each input method returns the
//! situations it produced; the caller publishes them and feeds the alarm engine.
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use shared::util::{new_id, now_iso};
use shared::{Location, RuleKey, SensorState, SensorType, Severity, SituationEvent, Telemetry};

use crate::config::Config;
use crate::registry::Registry;

fn room_key(l: &Location) -> String {
    format!("{}|{}|{}", l.building, l.floor, l.room)
}
fn unknown_location() -> Location {
    Location {
        building: "Unknown".into(),
        floor: "Unknown".into(),
        room: "Unassigned".into(),
        room_id: None,
    }
}

struct Draft {
    rule: RuleKey,
    severity: Severity,
    message: String,
    location: Location,
    device_ids: Vec<String>,
}

#[derive(Default)]
pub struct RuleEngine {
    last_motion_by_room: HashMap<String, Instant>,
    stove_state: HashMap<String, (bool, Instant)>, // (on, since)
    door_state: HashMap<String, (bool, Instant)>,  // (open, opened_at)
    bed_state: HashMap<String, (bool, Instant)>,   // (occupied, left_at)
    sos_prev: HashMap<String, bool>,
    active: HashSet<String>,
}

impl RuleEngine {
    fn make_event(draft: Draft, cleared: bool) -> SituationEvent {
        SituationEvent {
            event_id: new_id("evt"),
            rule: draft.rule,
            severity: draft.severity,
            message: draft.message,
            location: draft.location,
            device_ids: draft.device_ids,
            cleared: Some(cleared),
            detected_at: now_iso(),
        }
    }

    fn raise(&mut self, key: String, draft: Draft, out: &mut Vec<SituationEvent>) {
        if self.active.contains(&key) {
            return;
        }
        self.active.insert(key);
        out.push(Self::make_event(draft, false));
    }

    fn clear(&mut self, key: &str, draft: Draft, out: &mut Vec<SituationEvent>) {
        if self.active.remove(key) {
            out.push(Self::make_event(draft, true));
        }
    }

    fn silent_clear(&mut self, key: &str) {
        self.active.remove(key);
    }

    // ----- Inputs ----------------------------------------------------------

    pub fn handle_presence(
        &mut self,
        registry: &Registry,
        device_id: &str,
        online: bool,
    ) -> Vec<SituationEvent> {
        let mut out = Vec::new();
        let Some(rec) = registry.get(device_id) else {
            return out;
        };
        let key = format!("device_offline:{device_id}");
        let loc = rec.location.clone().unwrap_or_else(unknown_location);
        if online {
            self.clear(
                &key,
                Draft {
                    rule: RuleKey::DeviceOffline,
                    severity: Severity::Warning,
                    message: format!("{} ({device_id}) is reachable again.", rec.name),
                    location: loc,
                    device_ids: vec![device_id.to_string()],
                },
                &mut out,
            );
        } else {
            self.raise(
                key,
                Draft {
                    rule: RuleKey::DeviceOffline,
                    severity: Severity::Warning,
                    message: format!(
                        "{} ({device_id}) is unreachable - no connection to the hub.",
                        rec.name
                    ),
                    location: loc,
                    device_ids: vec![device_id.to_string()],
                },
                &mut out,
            );
        }
        out
    }

    pub fn handle_telemetry(&mut self, registry: &Registry, t: &Telemetry) -> Vec<SituationEvent> {
        let mut out = Vec::new();
        let Some(rec) = registry.get(&t.device_id) else {
            return out;
        };
        let now = Instant::now();

        // SOS needs no room: a press is always actionable (rising edge, latches).
        if let SensorState::Sos { pressed } = &t.state {
            let pressed = *pressed;
            let prev = *self.sos_prev.get(&t.device_id).unwrap_or(&false);
            self.sos_prev.insert(t.device_id.clone(), pressed);
            if pressed && !prev {
                let room_suffix = rec
                    .location
                    .as_ref()
                    .map(|l| format!(" in {}", l.room))
                    .unwrap_or_default();
                out.push(Self::make_event(
                    Draft {
                        rule: RuleKey::SosPressed,
                        severity: Severity::Critical,
                        message: format!("SOS button pressed{room_suffix} - assistance requested."),
                        location: rec.location.clone().unwrap_or_else(unknown_location),
                        device_ids: vec![t.device_id.clone()],
                    },
                    false,
                ));
            }
            return out;
        }

        let Some(loc) = rec.location.as_ref() else {
            return out; // unassigned: no room-based rules
        };
        let rk = room_key(loc);

        match &t.state {
            SensorState::Motion { motion, .. } => {
                if *motion {
                    self.last_motion_by_room.insert(rk, now);
                }
            }
            SensorState::Stove { on, .. } => {
                let prev = self.stove_state.get(&t.device_id).map(|(o, _)| *o).unwrap_or(false);
                if *on && !prev {
                    self.stove_state.insert(t.device_id.clone(), (true, now));
                } else if !*on {
                    self.stove_state.insert(t.device_id.clone(), (false, now));
                    self.silent_clear(&format!("stove_on_no_motion:{}", t.device_id));
                }
            }
            SensorState::Door { open } => {
                let prev = self.door_state.get(&t.device_id).map(|(o, _)| *o).unwrap_or(false);
                if *open && !prev {
                    self.door_state.insert(t.device_id.clone(), (true, now));
                } else if !*open {
                    self.door_state.insert(t.device_id.clone(), (false, now));
                    self.silent_clear(&format!("door_open_no_motion:{}", t.device_id));
                }
            }
            SensorState::Bed { occupied, .. } => {
                let prev = self.bed_state.get(&t.device_id).map(|(o, _)| *o).unwrap_or(true);
                if !*occupied && prev {
                    self.bed_state.insert(t.device_id.clone(), (false, now));
                } else if *occupied {
                    self.bed_state.insert(t.device_id.clone(), (true, now));
                    self.silent_clear(&format!("bed_left_at_night_no_return:{}", t.device_id));
                }
            }
            SensorState::Sos { .. } => {}
        }
        out
    }

    // ----- Periodic evaluation ---------------------------------------------

    pub fn evaluate(&mut self, registry: &Registry, config: &Config) -> Vec<SituationEvent> {
        let mut out = Vec::new();
        let now = Instant::now();

        // Rule 1: stove on a long time, but no motion in the room.
        let stoves: Vec<(String, Instant)> = self
            .stove_state
            .iter()
            .filter(|(_, (on, _))| *on)
            .map(|(id, (_, since))| (id.clone(), *since))
            .collect();
        for (id, since) in stoves {
            let Some(rec) = registry.get(&id) else { continue };
            let Some(loc) = rec.location.clone() else { continue };
            let on_seconds = now.duration_since(since).as_secs_f64();
            let rk = room_key(&loc);
            let no_motion = self.seconds_since_motion(&rk, now);
            if on_seconds >= config.stove_on_seconds as f64
                && no_motion >= config.stove_no_motion_seconds as f64
            {
                let mut ids = vec![id.clone()];
                ids.extend(self.motion_sensors_in_room(registry, &rk));
                self.raise(
                    format!("stove_on_no_motion:{id}"),
                    Draft {
                        rule: RuleKey::StoveOnNoMotion,
                        severity: Severity::Critical,
                        message: format!(
                            "Stove in {} has been on for {}s, but {}.",
                            loc.room,
                            on_seconds.round() as i64,
                            fmt_since_motion(no_motion)
                        ),
                        location: loc,
                        device_ids: ids,
                    },
                    &mut out,
                );
            }
        }

        // Rule 2: door opened, then unusually long no motion.
        let doors: Vec<(String, Instant)> = self
            .door_state
            .iter()
            .filter(|(_, (open, _))| *open)
            .map(|(id, (_, at))| (id.clone(), *at))
            .collect();
        for (id, at) in doors {
            let Some(rec) = registry.get(&id) else { continue };
            let Some(loc) = rec.location.clone() else { continue };
            let open_seconds = now.duration_since(at).as_secs_f64();
            let rk = room_key(&loc);
            let no_motion = self.seconds_since_motion(&rk, now);
            if open_seconds >= config.door_open_seconds as f64
                && no_motion >= config.door_no_motion_seconds as f64
            {
                let mut ids = vec![id.clone()];
                ids.extend(self.motion_sensors_in_room(registry, &rk));
                self.raise(
                    format!("door_open_no_motion:{id}"),
                    Draft {
                        rule: RuleKey::DoorOpenNoMotion,
                        severity: Severity::Warning,
                        message: format!(
                            "Door in {} has been open for {}s, then {}.",
                            loc.room,
                            open_seconds.round() as i64,
                            fmt_since_motion(no_motion)
                        ),
                        location: loc,
                        device_ids: ids,
                    },
                    &mut out,
                );
            }
        }

        // Rule 3: bed left at night and no return for a long time.
        let beds: Vec<(String, Instant)> = self
            .bed_state
            .iter()
            .filter(|(_, (occ, _))| !*occ)
            .map(|(id, (_, at))| (id.clone(), *at))
            .collect();
        for (id, at) in beds {
            let Some(rec) = registry.get(&id) else { continue };
            let Some(loc) = rec.location.clone() else { continue };
            let absence = now.duration_since(at).as_secs_f64();
            if is_night(config) && absence >= config.bed_absence_seconds as f64 {
                self.raise(
                    format!("bed_left_at_night_no_return:{id}"),
                    Draft {
                        rule: RuleKey::BedLeftAtNightNoReturn,
                        severity: Severity::Critical,
                        message: format!(
                            "Bed in {} was left at night and no return detected for {}s.",
                            loc.room,
                            absence.round() as i64
                        ),
                        location: loc,
                        device_ids: vec![id.clone()],
                    },
                    &mut out,
                );
            }
        }

        out
    }

    fn motion_sensors_in_room(&self, registry: &Registry, rk: &str) -> Vec<String> {
        registry
            .all()
            .filter(|d| {
                d.kind == SensorType::Motion
                    && d.location.as_ref().map(|l| room_key(l) == rk).unwrap_or(false)
            })
            .map(|d| d.device_id.clone())
            .collect()
    }
    fn seconds_since_motion(&self, rk: &str, now: Instant) -> f64 {
        match self.last_motion_by_room.get(rk) {
            Some(&last) => now.duration_since(last).as_secs_f64(),
            None => f64::INFINITY,
        }
    }
}

fn fmt_since_motion(seconds: f64) -> String {
    if seconds.is_finite() {
        format!("no motion in the room for {}s", seconds.round() as i64)
    } else {
        "no active motion sensor in the room".to_string()
    }
}

fn is_night(config: &Config) -> bool {
    if config.force_night {
        return true;
    }
    use chrono::Timelike;
    let h = chrono::Local::now().hour();
    let (s, e) = (config.night_start_hour, config.night_end_hour);
    if s > e {
        h >= s || h < e
    } else {
        h >= s && h < e
    }
}
