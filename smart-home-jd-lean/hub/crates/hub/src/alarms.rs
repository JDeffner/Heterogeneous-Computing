//! Hub alarm engine. Turns situation events into standalone alarm objects,
//! published RETAINED so a UI connecting later sees current alarms immediately.
//! Identical, still-active alarms are deduplicated. Resolution (manual or via a
//! `cleared` event) removes the retained alarm.
//!
//! This engine also owns the RESPONSE side of an alarm:
//!  - recommended actions: concrete ordered steps for the caregiver,
//!  - a prepared emergency CALL SHEET (who to dial, what to say), built from
//!    the resident profile so 112/110 gets the correct information,
//!  - escalation: a critical alarm nobody acknowledges within the ack timeout
//!    is flagged `escalated` and an event urges placing the call NOW.
use std::collections::HashMap;
use std::time::Instant;

use chrono::{Datelike, Utc};
use rumqttc::AsyncClient;
use shared::util::{new_id, now_iso};
use shared::{
    topics, Alarm, AlarmControl, AlarmStatus, CallSheet, Contact, Resident, RuleKey, Severity,
    SituationEvent,
};

use crate::config::Config;
use crate::mqtt::{clear_retained, publish_json};

pub struct AlarmEngine {
    active: HashMap<String, (Alarm, Instant)>,
    counter: u64,
    client: AsyncClient,
    resident: Option<Resident>,
}

impl AlarmEngine {
    pub fn new(client: AsyncClient) -> Self {
        AlarmEngine {
            active: HashMap::new(),
            counter: 0,
            client,
            resident: None,
        }
    }

    pub fn set_resident(&mut self, resident: Resident) {
        self.resident = Some(resident);
    }

    pub async fn handle_event(&mut self, ev: &SituationEvent) {
        let key = dedup_key(ev);

        if ev.cleared.unwrap_or(false) {
            self.resolve(&key, "situation cleared").await;
            return;
        }

        let ts = now_iso();
        if let Some((existing, _)) = self.active.get_mut(&key) {
            if existing.status != AlarmStatus::Resolved {
                existing.updated_at = ts;
                existing.message = ev.message.clone();
                if !ev.evidence.is_empty() {
                    existing.evidence = ev.evidence.clone();
                }
                let existing = existing.clone();
                publish_json(&self.client, &topics::alarm(&existing.alarm_id), &existing, true).await;
                return;
            }
        }

        let guidance = Guidance::for_rule(ev, self.resident.as_ref());
        let alarm = Alarm {
            alarm_id: self.new_alarm_id(),
            rule: ev.rule,
            severity: ev.severity,
            message: ev.message.clone(),
            location: ev.location.clone(),
            device_ids: ev.device_ids.clone(),
            status: AlarmStatus::Active,
            raised_at: ts.clone(),
            updated_at: ts,
            recommended_actions: guidance.actions,
            evidence: ev.evidence.clone(),
            call_sheet: guidance.call_sheet,
            escalated: false,
            call_logged_at: None,
        };
        publish_json(&self.client, &topics::alarm(&alarm.alarm_id), &alarm, true).await;
        let l = &alarm.location;
        println!(
            ">>> ALARM [{}] {} / {} / {}: {} (alarmId={})",
            alarm.severity.as_str().to_uppercase(),
            l.building,
            l.floor,
            l.room,
            alarm.message,
            alarm.alarm_id
        );
        self.active.insert(key, (alarm, Instant::now()));
    }

    /// Escalate critical alarms that stayed unacknowledged past the timeout.
    /// Returns info events for the activity feed (published by the caller,
    /// NOT fed back into `handle_event`).
    pub async fn tick(&mut self, config: &Config) -> Vec<SituationEvent> {
        let mut out = Vec::new();
        let timeout = config.ack_timeout_seconds;
        for (alarm, raised) in self.active.values_mut() {
            if alarm.severity == Severity::Critical
                && alarm.status == AlarmStatus::Active
                && !alarm.escalated
                && raised.elapsed().as_secs() >= timeout
            {
                alarm.escalated = true;
                alarm.updated_at = now_iso();
                publish_json(&self.client, &topics::alarm(&alarm.alarm_id), alarm, true).await;
                let call_hint = match &alarm.call_sheet {
                    Some(cs) => format!("Place the {} call now - the call sheet is ready.", cs.number),
                    None => "Act on the recommended steps now.".to_string(),
                };
                println!(
                    ">>> ESCALATED alarm {} - unacknowledged for {timeout}s",
                    alarm.alarm_id
                );
                out.push(SituationEvent {
                    event_id: new_id("evt"),
                    rule: RuleKey::AlarmEscalated,
                    severity: Severity::Critical,
                    message: format!(
                        "Critical alarm in {} unacknowledged for {timeout}s. {call_hint}",
                        alarm.location.room
                    ),
                    location: alarm.location.clone(),
                    device_ids: alarm.device_ids.clone(),
                    cleared: None,
                    detected_at: now_iso(),
                    evidence: Vec::new(),
                });
            }
        }
        out
    }

    /// Returns an info event for the activity feed where useful.
    pub async fn handle_control(&mut self, ctrl: &AlarmControl) -> Option<SituationEvent> {
        let key = self
            .active
            .iter()
            .find(|(_, (a, _))| a.alarm_id == ctrl.alarm_id)
            .map(|(k, _)| k.clone());
        let Some(key) = key else {
            eprintln!(
                "[hub] {} requested for unknown alarm {}",
                ctrl.action, ctrl.alarm_id
            );
            return None;
        };

        match ctrl.action.as_str() {
            "resolve" => {
                self.resolve(&key, "manual resolve").await;
                None
            }
            "ack" => {
                let (alarm, _) = self.active.get_mut(&key)?;
                alarm.status = AlarmStatus::Acknowledged;
                alarm.updated_at = now_iso();
                publish_json(&self.client, &topics::alarm(&alarm.alarm_id), alarm, true).await;
                println!("ACK alarm {}", alarm.alarm_id);
                Some(feed_event(alarm, "Alarm acknowledged - a caregiver is on it."))
            }
            "call_logged" => {
                let (alarm, _) = self.active.get_mut(&key)?;
                alarm.call_logged_at = Some(now_iso());
                if alarm.status == AlarmStatus::Active {
                    alarm.status = AlarmStatus::Acknowledged;
                }
                alarm.updated_at = now_iso();
                publish_json(&self.client, &topics::alarm(&alarm.alarm_id), alarm, true).await;
                let service = alarm
                    .call_sheet
                    .as_ref()
                    .map(|cs| format!("{} ({})", cs.number, cs.service))
                    .unwrap_or_else(|| "emergency".into());
                println!("CALL LOGGED for alarm {} -> {service}", alarm.alarm_id);
                Some(feed_event(
                    alarm,
                    &format!("Emergency call to {service} marked as placed."),
                ))
            }
            other => {
                eprintln!("[hub] unknown alarm action {other}");
                None
            }
        }
    }

    async fn resolve(&mut self, key: &str, reason: &str) {
        if let Some((alarm, _)) = self.active.remove(key) {
            clear_retained(&self.client, &topics::alarm(&alarm.alarm_id)).await;
            println!(
                "RESOLVED alarm {} ({:?}) - {reason}",
                alarm.alarm_id, alarm.rule
            );
        }
    }

    fn new_alarm_id(&mut self) -> String {
        self.counter += 1;
        let millis = Utc::now().timestamp_millis().max(0) as u128;
        format!("alm-{}-{}", base36(millis), self.counter)
    }
}

fn feed_event(alarm: &Alarm, message: &str) -> SituationEvent {
    SituationEvent {
        event_id: new_id("evt"),
        rule: alarm.rule,
        severity: Severity::Info,
        message: message.to_string(),
        location: alarm.location.clone(),
        device_ids: alarm.device_ids.clone(),
        cleared: None,
        detected_at: now_iso(),
        evidence: Vec::new(),
    }
}

// ----- Guidance: what to do, whom to call, what to say ----------------------

struct Guidance {
    actions: Vec<String>,
    call_sheet: Option<CallSheet>,
}

impl Guidance {
    fn for_rule(ev: &SituationEvent, resident: Option<&Resident>) -> Guidance {
        let room = ev.location.room.as_str();
        let name = resident.map(|r| r.name.as_str()).unwrap_or("the resident");
        let key_holder = resident
            .and_then(|r| r.contacts.iter().find(|c| c.role.to_lowercase().contains("key")))
            .or_else(|| resident.and_then(|r| r.contacts.first()));
        let key_holder_line = key_holder
            .map(|c| format!("Send {} ({}, {}) to check in person.", c.name, c.role, c.phone))
            .unwrap_or_else(|| "Send someone with a key to check in person.".to_string());
        let dementia = resident
            .map(|r| r.conditions.iter().any(|c| c.to_lowercase().contains("dement")))
            .unwrap_or(false);

        let (actions, service): (Vec<String>, Option<EmergencyService>) = match ev.rule {
            RuleKey::SosPressed => (
                vec![
                    format!("Speak to {name} immediately via intercom or phone."),
                    "No response within 1 minute: call 112 with the prepared call sheet.".into(),
                    key_holder_line,
                ],
                Some(EmergencyService::Ambulance),
            ),
            RuleKey::PossibleFall => (
                vec![
                    format!("Call {name} and listen via the intercom in {room}."),
                    key_holder_line,
                    "Unresponsive or injured: call 112 (ambulance) - suspected fall.".into(),
                ],
                Some(EmergencyService::Ambulance),
            ),
            RuleKey::StoveOnNoMotion => (
                vec![
                    format!("Call {name} - the stove may simply be forgotten."),
                    key_holder_line,
                    "Smoke, or nobody can get there: call 112 (fire brigade).".into(),
                ],
                Some(EmergencyService::Fire),
            ),
            RuleKey::BedLeftAtNightNoReturn => (
                vec![
                    format!("Call {name} - they may be in the bathroom or kitchen."),
                    key_holder_line,
                    "Unresponsive: call 112 (ambulance) - possible collapse on the way back to bed.".into(),
                ],
                Some(EmergencyService::Ambulance),
            ),
            RuleKey::DoorOpenAtNight => {
                let mut a = vec![format!(
                    "Check whether {name} left the building (door camera, hallway)."
                )];
                if dementia {
                    a.push(format!(
                        "{name} has dementia (see profile): high disorientation risk, search the surroundings immediately."
                    ));
                }
                a.push(key_holder_line);
                a.push("Not found nearby: call 110 (police) - missing vulnerable person.".into());
                (a, Some(EmergencyService::Police))
            }
            RuleKey::DoorOpenNoMotion => (
                vec![
                    format!("Check the door in {room} - possibly a visitor left it open."),
                    format!("Give {name} a call if anything seems off."),
                ],
                None,
            ),
            RuleKey::Inactivity => (
                vec![
                    format!("Give {name} a check-in call."),
                    "No answer: treat as a possible fall and follow that procedure.".into(),
                ],
                None,
            ),
            RuleKey::DeviceOffline => (
                vec![
                    "Check the device's battery and power supply.".into(),
                    "Check Wi-Fi/radio coverage in the room.".into(),
                ],
                None,
            ),
            RuleKey::AlarmEscalated => (Vec::new(), None),
        };

        Guidance {
            actions,
            call_sheet: service.map(|s| build_call_sheet(s, ev, resident)),
        }
    }
}

#[derive(Clone, Copy)]
enum EmergencyService {
    Ambulance,
    Fire,
    Police,
}

/// Compose the exact call: correct number and service, and the answers to the
/// standard dispatcher questions (who / what / where / access / call-back),
/// filled from the resident profile and the alarm's sensor evidence.
fn build_call_sheet(
    service: EmergencyService,
    ev: &SituationEvent,
    resident: Option<&Resident>,
) -> CallSheet {
    let (number, service_name) = match service {
        EmergencyService::Ambulance => ("112", "Rettungsdienst (ambulance)"),
        EmergencyService::Fire => ("112", "Feuerwehr (fire brigade)"),
        EmergencyService::Police => ("110", "Polizei (missing person)"),
    };

    let reason = match ev.rule {
        RuleKey::SosPressed => "Resident pressed the emergency call button and may need medical help.",
        RuleKey::PossibleFall => "Suspected fall: movement stopped abruptly, resident unresponsive.",
        RuleKey::StoveOnNoMotion => "Stove is on and unattended, resident not responding - fire risk.",
        RuleKey::BedLeftAtNightNoReturn => "Resident left bed at night and has not returned - possible collapse.",
        RuleKey::DoorOpenAtNight => "Vulnerable resident presumably left home at night and is missing.",
        _ => "Assisted-living monitoring alarm.",
    };

    let mut script: Vec<String> = Vec::new();
    script.push(format!(
        "\"I am calling from an assisted-living monitoring service about {}.\"",
        resident.map(|r| r.name.as_str()).unwrap_or("a monitored resident")
    ));
    script.push(format!("\"What happened: {reason}\""));
    let mut evidence_line = format!("Detected at {} in {}.", ev.detected_at, ev.location.room);
    if let Some(last) = ev.evidence.last() {
        evidence_line.push_str(&format!(" Last observation: {} ({}).", last.text, last.ts));
    }
    script.push(format!("\"{evidence_line}\""));

    if let Some(r) = resident {
        let age = Utc::now().year() - r.year_of_birth;
        let mut who = format!("Patient: {}, born {}, about {} years old.", r.name, r.year_of_birth, age);
        if !r.conditions.is_empty() {
            who.push_str(&format!(" Known conditions: {}.", r.conditions.join(", ")));
        }
        if !r.medications.is_empty() {
            who.push_str(&format!(" Medication: {}.", r.medications.join(", ")));
        }
        script.push(format!("\"{who}\""));
        if !r.notes.is_empty() {
            script.push(format!("\"Note: {}\"", r.notes));
        }
        script.push(format!(
            "\"Address: {}. Room: {} ({}).\"",
            r.address, ev.location.room, ev.location.floor
        ));
        if !r.access_info.is_empty() {
            script.push(format!("\"Access: {}\"", r.access_info));
        }
        if let Some(c) = first_contact(r) {
            script.push(format!(
                "\"Call-back contact: {} ({}), {}.\"",
                c.name, c.role, c.phone
            ));
        }
    } else {
        script.push(format!(
            "\"Location: {} / {} / {}. (No resident profile on file.)\"",
            ev.location.building, ev.location.floor, ev.location.room
        ));
    }

    CallSheet {
        number: number.to_string(),
        service: service_name.to_string(),
        reason: reason.to_string(),
        script,
    }
}

fn first_contact(r: &Resident) -> Option<&Contact> {
    r.contacts.first()
}

fn dedup_key(ev: &SituationEvent) -> String {
    let mut ids = ev.device_ids.clone();
    ids.sort();
    format!("{:?}:{}", ev.rule, ids.join(","))
}

fn base36(mut n: u128) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut o = Vec::new();
    while n > 0 {
        o.push(D[(n % 36) as usize]);
        n /= 36;
    }
    o.reverse();
    String::from_utf8(o).unwrap()
}
