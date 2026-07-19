//! Hub alarm engine. Turns situation events into standalone alarm objects,
//! published RETAINED so a UI connecting later sees current alarms immediately.
//! Identical, still-active alarms are deduplicated. Resolution (manual or via a
//! `cleared` event) removes the retained alarm.
use std::collections::HashMap;

use chrono::Utc;
use rumqttc::AsyncClient;
use shared::util::now_iso;
use shared::{topics, Alarm, AlarmControl, AlarmStatus, SituationEvent};

use crate::mqtt::{clear_retained, publish_json};

pub struct AlarmEngine {
    active: HashMap<String, Alarm>,
    counter: u64,
    client: AsyncClient,
}

impl AlarmEngine {
    pub fn new(client: AsyncClient) -> Self {
        AlarmEngine {
            active: HashMap::new(),
            counter: 0,
            client,
        }
    }

    pub async fn handle_event(&mut self, ev: &SituationEvent) {
        let key = dedup_key(ev);

        if ev.cleared.unwrap_or(false) {
            self.resolve(&key, "situation cleared").await;
            return;
        }

        let ts = now_iso();
        if let Some(existing) = self.active.get_mut(&key) {
            if existing.status != AlarmStatus::Resolved {
                existing.updated_at = ts;
                existing.message = ev.message.clone();
                let existing = existing.clone();
                publish_json(&self.client, &topics::alarm(&existing.alarm_id), &existing, true).await;
                return;
            }
        }

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
        self.active.insert(key, alarm);
    }

    pub async fn handle_control(&mut self, ctrl: &AlarmControl) {
        if ctrl.action != "resolve" {
            return;
        }
        let key = self
            .active
            .iter()
            .find(|(_, a)| a.alarm_id == ctrl.alarm_id)
            .map(|(k, _)| k.clone());
        match key {
            Some(k) => self.resolve(&k, "manual resolve").await,
            None => eprintln!("[hub] resolve requested for unknown alarm {}", ctrl.alarm_id),
        }
    }

    async fn resolve(&mut self, key: &str, reason: &str) {
        if let Some(alarm) = self.active.remove(key) {
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
