/**
 * Hub alarm engine.
 *
 * Turns situation events into standalone alarm objects, published RETAINED so a
 * UI connecting later sees the current alarm state immediately. Identical,
 * still-active alarms are deduplicated. Resolution happens either manually (an
 * operator "resolve" control message) or automatically (a `cleared` event when
 * the underlying situation ends); in both cases the retained alarm is removed.
 *
 * In a real installation this is where care staff would be notified (push, SMS,
 * nurse call). Here the published alarm object and a console line stand in.
 */
import type { MqttClient } from "mqtt";
import { publishJson } from "../shared/mqtt.js";
import { topics } from "../shared/topics.js";
import type { Alarm, AlarmControl, SituationEvent } from "../shared/types.js";

export class AlarmEngine {
  /** Active alarms, key = rule + sorted devices (for deduplication). */
  private active = new Map<string, Alarm>();
  private counter = 0;

  constructor(private readonly client: MqttClient) {}

  handleEvent(ev: SituationEvent): void {
    const key = this.dedupKey(ev);

    if (ev.cleared) {
      this.resolve(key, "situation cleared");
      return;
    }

    const ts = new Date().toISOString();
    const existing = this.active.get(key);
    if (existing && existing.status !== "resolved") {
      existing.updatedAt = ts;
      existing.message = ev.message;
      publishJson(this.client, topics.alarm(existing.alarmId), existing, { retain: true });
      return;
    }

    const alarm: Alarm = {
      alarmId: this.newAlarmId(),
      rule: ev.rule,
      severity: ev.severity,
      message: ev.message,
      location: ev.location,
      deviceIds: ev.deviceIds,
      status: "active",
      raisedAt: ts,
      updatedAt: ts,
    };
    this.active.set(key, alarm);
    publishJson(this.client, topics.alarm(alarm.alarmId), alarm, { retain: true });

    const l = alarm.location;
    console.log(
      `>>> ALARM [${alarm.severity.toUpperCase()}] ${l.building} / ${l.floor} / ${l.room}: ` +
        `${alarm.message} (alarmId=${alarm.alarmId})`,
    );
  }

  handleControl(ctrl: AlarmControl): void {
    if (ctrl.action !== "resolve") return;
    for (const [key, alarm] of this.active) {
      if (alarm.alarmId === ctrl.alarmId) {
        this.resolve(key, "manual resolve");
        return;
      }
    }
    console.warn(`[hub] resolve requested for unknown alarm ${ctrl.alarmId}`);
  }

  private resolve(key: string, reason: string): void {
    const alarm = this.active.get(key);
    if (!alarm) return;
    this.active.delete(key);
    this.client.publish(topics.alarm(alarm.alarmId), "", { retain: true, qos: 1 });
    console.log(`RESOLVED alarm ${alarm.alarmId} (${alarm.rule}) - ${reason}`);
  }

  private dedupKey(ev: SituationEvent): string {
    return `${ev.rule}:${[...ev.deviceIds].sort().join(",")}`;
  }
  private newAlarmId(): string {
    this.counter += 1;
    return `alm-${Date.now().toString(36)}-${this.counter}`;
  }
}
