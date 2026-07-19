//! Hub rule engine. Consumes presence + telemetry, tracks per-device/per-room
//! state, and derives "situations" (edge-triggered: emitted once per occurrence,
//! with a matching `cleared` event on recovery). Each input method returns the
//! situations it produced; the caller publishes them and feeds the alarm engine.
//!
//! Besides the per-sensor rules, the engine keeps a rolling EVIDENCE LOG of
//! human-readable sensor observations. Every situation carries the recent
//! evidence, so an alarm can show *why* it fired, and the emergency call sheet
//! can quote concrete observations to the dispatcher.
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use shared::util::{new_id, now_iso};
use shared::{
    EvidenceItem, Location, RuleKey, SensorState, SensorType, Severity, SituationEvent, Telemetry,
};

use crate::config::Config;
use crate::registry::Registry;

const EVIDENCE_KEEP: usize = 60;
const EVIDENCE_ATTACH: usize = 8;

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
    evidence: Vec<EvidenceItem>,
}

/// The last thing the resident visibly did (any sensor edge caused by a person).
#[derive(Clone)]
struct Activity {
    at: Instant,
    room: String,
    location: Location,
}

#[derive(Default)]
pub struct RuleEngine {
    last_motion_by_room: HashMap<String, Instant>,
    motion_prev: HashMap<String, bool>,
    stove_state: HashMap<String, (bool, Instant)>, // (on, since)
    stove_temp: HashMap<String, i64>,
    door_state: HashMap<String, (bool, Instant)>, // (open, opened_at)
    bed_state: HashMap<String, (bool, Instant)>,  // (occupied, left_at)
    sos_prev: HashMap<String, bool>,
    active: HashSet<String>,
    evidence: VecDeque<(Instant, EvidenceItem)>,
    last_activity: Option<Activity>,
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
            evidence: draft.evidence,
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

    // ----- Evidence log ----------------------------------------------------

    fn note(&mut self, text: String) {
        self.evidence.push_back((
            Instant::now(),
            EvidenceItem { ts: now_iso(), text },
        ));
        while self.evidence.len() > EVIDENCE_KEEP {
            self.evidence.pop_front();
        }
    }

    /// The most recent observations, oldest first.
    pub fn recent_evidence(&self) -> Vec<EvidenceItem> {
        self.evidence
            .iter()
            .rev()
            .take(EVIDENCE_ATTACH)
            .map(|(_, e)| e.clone())
            .rev()
            .collect()
    }

    fn mark_activity(&mut self, loc: &Location) {
        self.last_activity = Some(Activity {
            at: Instant::now(),
            room: loc.room.clone(),
            location: loc.clone(),
        });
    }

    /// "last seen: motion in Kitchen, 40s ago" for messages and call sheets.
    pub fn last_activity_summary(&self) -> Option<String> {
        let a = self.last_activity.as_ref()?;
        Some(format!(
            "last activity in {} {}s ago",
            a.room,
            a.at.elapsed().as_secs()
        ))
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
        let name = rec.name.clone();
        let loc = rec.location.clone().unwrap_or_else(unknown_location);
        self.note(format!(
            "{name} went {}",
            if online { "online" } else { "offline" }
        ));
        let key = format!("device_offline:{device_id}");
        if online {
            self.clear(
                &key,
                Draft {
                    rule: RuleKey::DeviceOffline,
                    severity: Severity::Warning,
                    message: format!("{name} ({device_id}) is reachable again."),
                    location: loc,
                    device_ids: vec![device_id.to_string()],
                    evidence: Vec::new(),
                },
                &mut out,
            );
        } else {
            let evidence = self.recent_evidence();
            self.raise(
                key,
                Draft {
                    rule: RuleKey::DeviceOffline,
                    severity: Severity::Warning,
                    message: format!(
                        "{name} ({device_id}) is unreachable - no connection to the hub."
                    ),
                    location: loc,
                    device_ids: vec![device_id.to_string()],
                    evidence,
                },
                &mut out,
            );
        }
        out
    }

    pub fn handle_telemetry(
        &mut self,
        registry: &Registry,
        config: &Config,
        t: &Telemetry,
    ) -> Vec<SituationEvent> {
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
                let loc = rec.location.clone().unwrap_or_else(unknown_location);
                self.note(format!("SOS button pressed ({})", loc.room));
                self.mark_activity(&loc);
                let evidence = self.recent_evidence();
                out.push(Self::make_event(
                    Draft {
                        rule: RuleKey::SosPressed,
                        severity: Severity::Critical,
                        message: format!(
                            "SOS button pressed in {} - the resident is actively calling for help.",
                            loc.room
                        ),
                        location: loc,
                        device_ids: vec![t.device_id.clone()],
                        evidence,
                    },
                    false,
                ));
            } else if !pressed && prev {
                self.note("SOS button released".into());
            }
            return out;
        }

        let Some(loc) = rec.location.clone() else {
            return out; // unassigned: no room-based rules
        };
        let rk = room_key(&loc);

        match &t.state {
            SensorState::Motion { motion, .. } => {
                let prev = *self.motion_prev.get(&t.device_id).unwrap_or(&false);
                self.motion_prev.insert(t.device_id.clone(), *motion);
                if *motion {
                    self.last_motion_by_room.insert(rk, now);
                    if !prev {
                        self.note(format!("Motion detected in {}", loc.room));
                    }
                    self.mark_activity(&loc);
                    // Any motion means the resident is up and moving.
                    self.clear_silence_alarms(&loc, &mut out);
                } else if prev {
                    self.note(format!("Motion in {} ended", loc.room));
                }
            }
            SensorState::Stove { on, temperature_c } => {
                let prev = self.stove_state.get(&t.device_id).map(|(o, _)| *o).unwrap_or(false);
                self.stove_temp.insert(t.device_id.clone(), *temperature_c);
                if *on && !prev {
                    self.stove_state.insert(t.device_id.clone(), (true, now));
                    self.note(format!("Stove in {} turned on", loc.room));
                    self.mark_activity(&loc);
                } else if !*on && prev {
                    self.stove_state.insert(t.device_id.clone(), (false, now));
                    self.note(format!("Stove in {} turned off", loc.room));
                    self.mark_activity(&loc);
                    self.silent_clear(&format!("stove_on_no_motion:{}", t.device_id));
                } else if !*on {
                    self.stove_state.insert(t.device_id.clone(), (false, now));
                }
            }
            SensorState::Door { open } => {
                let prev = self.door_state.get(&t.device_id).map(|(o, _)| *o).unwrap_or(false);
                if *open && !prev {
                    self.door_state.insert(t.device_id.clone(), (true, now));
                    self.note(format!("Door in {} opened", loc.room));
                    self.mark_activity(&loc);
                    // A door opening at night while the bed is empty is an
                    // immediate wandering indicator, not a slow timeout.
                    if is_night(config) && !self.any_bed_occupied() {
                        let evidence = self.recent_evidence();
                        self.raise(
                            format!("door_open_at_night:{}", t.device_id),
                            Draft {
                                rule: RuleKey::DoorOpenAtNight,
                                severity: Severity::Critical,
                                message: format!(
                                    "Door in {} opened at night while the bed is empty - the resident may be leaving the home.",
                                    loc.room
                                ),
                                location: loc.clone(),
                                device_ids: vec![t.device_id.clone()],
                                evidence,
                            },
                            &mut out,
                        );
                    }
                } else if !*open && prev {
                    self.door_state.insert(t.device_id.clone(), (false, now));
                    self.note(format!("Door in {} closed", loc.room));
                    self.mark_activity(&loc);
                    self.silent_clear(&format!("door_open_no_motion:{}", t.device_id));
                }
            }
            SensorState::Bed { occupied, .. } => {
                let prev = self.bed_state.get(&t.device_id).map(|(o, _)| *o).unwrap_or(true);
                if !*occupied && prev {
                    self.bed_state.insert(t.device_id.clone(), (false, now));
                    self.note(format!("Bed in {} was left", loc.room));
                    self.mark_activity(&loc);
                } else if *occupied && !prev {
                    self.bed_state.insert(t.device_id.clone(), (true, now));
                    self.note(format!("Bed in {} is occupied again", loc.room));
                    self.mark_activity(&loc);
                    self.silent_clear(&format!("bed_left_at_night_no_return:{}", t.device_id));
                    // Back in bed: night-wandering and silence concerns are over.
                    self.clear_night_alarms(&loc, &mut out);
                    self.clear_silence_alarms(&loc, &mut out);
                } else {
                    self.bed_state.insert(t.device_id.clone(), (*occupied, now));
                }
            }
            SensorState::Sos { .. } => {}
        }
        out
    }

    /// Clear possible-fall / inactivity when the resident is visibly fine again.
    fn clear_silence_alarms(&mut self, loc: &Location, out: &mut Vec<SituationEvent>) {
        for (key, rule, what) in [
            ("possible_fall", RuleKey::PossibleFall, "possible fall"),
            ("inactivity", RuleKey::Inactivity, "inactivity"),
        ] {
            self.clear(
                key,
                Draft {
                    rule,
                    severity: Severity::Info,
                    message: format!(
                        "Activity in {} - the {} concern is cleared.",
                        loc.room, what
                    ),
                    location: loc.clone(),
                    device_ids: Vec::new(),
                    evidence: Vec::new(),
                },
                out,
            );
        }
    }

    fn clear_night_alarms(&mut self, loc: &Location, out: &mut Vec<SituationEvent>) {
        let keys: Vec<String> = self
            .active
            .iter()
            .filter(|k| k.starts_with("door_open_at_night:"))
            .cloned()
            .collect();
        for key in keys {
            self.clear(
                &key,
                Draft {
                    rule: RuleKey::DoorOpenAtNight,
                    severity: Severity::Info,
                    message: format!(
                        "Bed in {} is occupied again - the night-wandering concern is cleared.",
                        loc.room
                    ),
                    location: loc.clone(),
                    device_ids: Vec::new(),
                    evidence: Vec::new(),
                },
                out,
            );
        }
    }

    fn any_bed_occupied(&self) -> bool {
        self.bed_state.values().any(|(occupied, _)| *occupied)
    }

    fn any_motion_sensor_reported(&self) -> bool {
        !self.motion_prev.is_empty()
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
                let temp = self.stove_temp.get(&id).copied();
                let mut ids = vec![id.clone()];
                ids.extend(self.motion_sensors_in_room(registry, &rk));
                let last_seen = self
                    .last_activity_summary()
                    .map(|s| format!(" Resident: {s}."))
                    .unwrap_or_default();
                let temp_part = temp.map(|t| format!(" at {t}\u{00b0}C")).unwrap_or_default();
                let evidence = self.recent_evidence();
                self.raise(
                    format!("stove_on_no_motion:{id}"),
                    Draft {
                        rule: RuleKey::StoveOnNoMotion,
                        severity: Severity::Critical,
                        message: format!(
                            "Stove in {} has been on{temp_part} for {}s and {}.{last_seen}",
                            loc.room,
                            on_seconds.round() as i64,
                            fmt_since_motion(no_motion)
                        ),
                        location: loc,
                        device_ids: ids,
                        evidence,
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
                let evidence = self.recent_evidence();
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
                        evidence,
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
                let last_seen = self
                    .last_activity_summary()
                    .map(|s| format!(" Since then: {s}."))
                    .unwrap_or_default();
                let evidence = self.recent_evidence();
                self.raise(
                    format!("bed_left_at_night_no_return:{id}"),
                    Draft {
                        rule: RuleKey::BedLeftAtNightNoReturn,
                        severity: Severity::Critical,
                        message: format!(
                            "Bed in {} was left {}s ago at night and the resident has not returned.{last_seen}",
                            loc.room,
                            absence.round() as i64
                        ),
                        location: loc,
                        device_ids: vec![id.clone()],
                        evidence,
                    },
                    &mut out,
                );
            }
        }

        // Rule 4 (inference): possible fall. The resident was up and moving,
        // then EVERY sensor went quiet - motion ended and nothing else happened
        // anywhere, while the bed is empty. Silence in bed is sleep; silence
        // right after movement is a fall risk.
        if self.any_motion_sensor_reported() && !self.any_bed_occupied() {
            if let Some(act) = self.last_activity.clone() {
                let silence = now.duration_since(act.at).as_secs_f64();
                if silence >= config.fall_silence_seconds as f64
                    && silence
                        < (config.fall_silence_seconds + config.fall_window_seconds) as f64
                {
                    let evidence = self.recent_evidence();
                    self.raise(
                        "possible_fall".into(),
                        Draft {
                            rule: RuleKey::PossibleFall,
                            severity: Severity::Critical,
                            message: format!(
                                "Activity in {} stopped abruptly {}s ago and no sensor has seen the resident since, while the bed is empty. This pattern matches a fall.",
                                act.room,
                                silence.round() as i64
                            ),
                            location: act.location.clone(),
                            device_ids: Vec::new(),
                            evidence,
                        },
                        &mut out,
                    );
                }

                // Rule 5: long daytime inactivity (the slow-burn variant).
                if !is_night(config)
                    && !self.active.contains("possible_fall")
                    && silence >= config.inactivity_seconds as f64
                {
                    let evidence = self.recent_evidence();
                    self.raise(
                        "inactivity".into(),
                        Draft {
                            rule: RuleKey::Inactivity,
                            severity: Severity::Warning,
                            message: format!(
                                "No activity anywhere in the home for {}s during the day (last seen in {}). A check-in call is advisable.",
                                silence.round() as i64,
                                act.room
                            ),
                            location: act.location,
                            device_ids: Vec::new(),
                            evidence,
                        },
                        &mut out,
                    );
                }
            }
        }

        // Leaving night mode clears the night-only concerns.
        if !is_night(config) {
            let keys: Vec<String> = self
                .active
                .iter()
                .filter(|k| k.starts_with("door_open_at_night:"))
                .cloned()
                .collect();
            for key in keys {
                self.clear(
                    &key,
                    Draft {
                        rule: RuleKey::DoorOpenAtNight,
                        severity: Severity::Info,
                        message: "Night mode ended - the night-wandering concern is cleared.".into(),
                        location: unknown_location(),
                        device_ids: Vec::new(),
                        evidence: Vec::new(),
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
