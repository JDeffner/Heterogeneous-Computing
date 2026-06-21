/**
 * Hub rule engine.
 *
 * Consumes presence + telemetry, tracks per-device/per-room state, and derives
 * "situations" (edge-triggered: emitted once per occurrence, with a matching
 * `cleared` event on recovery). Situations are handed to the caller, which both
 * publishes them for the UI event log and feeds them to the alarm engine.
 *
 * Rules:
 *   1. stove_on_no_motion          - stove on a long time with no motion in room
 *   2. door_open_no_motion         - door open a long time, then no motion
 *   3. bed_left_at_night_no_return - bed left at night without returning
 *   4. sos_pressed                 - panic button pressed (immediate, critical)
 *   5. device_offline              - a registered device became unreachable
 */
import { newId } from "../shared/mqtt.js";
import type { Location, SituationEvent, Telemetry } from "../shared/types.js";
import type { Registry } from "./registry.js";
import { config } from "./config.js";

const now = () => Date.now();
const roomKey = (l: Location) => `${l.building}|${l.floor}|${l.room}`;
const UNKNOWN_LOCATION: Location = {
  building: "Unknown",
  floor: "Unknown",
  room: "Unassigned",
};

type EventDraft = Omit<SituationEvent, "eventId" | "detectedAt" | "cleared">;

export class RuleEngine {
  private lastMotionByRoom = new Map<string, number>();
  private stoveState = new Map<string, { on: boolean; onSince: number }>();
  private doorState = new Map<string, { open: boolean; openedAt: number }>();
  private bedState = new Map<string, { occupied: boolean; leftAt: number }>();
  private sosPrev = new Map<string, boolean>();
  private activeSituations = new Set<string>();

  constructor(
    private readonly registry: Registry,
    private readonly emit: (ev: SituationEvent) => void,
  ) {}

  // ----- Emission helpers --------------------------------------------------

  private publish(draft: EventDraft, cleared = false): void {
    this.emit({
      ...draft,
      cleared,
      eventId: newId("evt"),
      detectedAt: new Date().toISOString(),
    });
  }

  private raise(key: string, draft: EventDraft): void {
    if (this.activeSituations.has(key)) return;
    this.activeSituations.add(key);
    this.publish(draft);
  }

  private clear(key: string, draft: EventDraft): void {
    if (!this.activeSituations.delete(key)) return;
    this.publish(draft, true);
  }

  /** Local-only clear (lets a rule re-fire) without notifying the alarm engine. */
  private silentClear(key: string): void {
    this.activeSituations.delete(key);
  }

  // ----- Inputs ------------------------------------------------------------

  handlePresence(deviceId: string, online: boolean): void {
    const rec = this.registry.get(deviceId);
    if (!rec) return;
    const key = `device_offline:${deviceId}`;
    if (online) {
      this.clear(key, {
        rule: "device_offline",
        severity: "warning",
        message: `${rec.name} (${deviceId}) is reachable again.`,
        location: rec.location ?? UNKNOWN_LOCATION,
        deviceIds: [deviceId],
      });
    } else {
      this.raise(key, {
        rule: "device_offline",
        severity: "warning",
        message: `${rec.name} (${deviceId}) is unreachable - no connection to the hub.`,
        location: rec.location ?? UNKNOWN_LOCATION,
        deviceIds: [deviceId],
      });
    }
  }

  handleTelemetry(t: Telemetry): void {
    const rec = this.registry.get(t.deviceId);
    if (!rec) return;
    const ts = Date.parse(t.ts) || now();

    // SOS does not need a room: a press is always actionable.
    if (t.state.kind === "sos") {
      const prev = this.sosPrev.get(t.deviceId) ?? false;
      this.sosPrev.set(t.deviceId, t.state.pressed);
      // Rising edge only. Releasing the button does NOT clear the alarm - a
      // panic alarm LATCHES until a caregiver resolves it manually. We publish
      // directly (not via the activeSituations latch) so the alarm engine, which
      // dedups while active and removes the alarm on resolve, will raise a fresh
      // alarm if the button is pressed again after a resolve.
      if (t.state.pressed && !prev) {
        this.publish({
          rule: "sos_pressed",
          severity: "critical",
          message: `SOS button pressed${rec.location ? ` in ${rec.location.room}` : ""} - assistance requested.`,
          location: rec.location ?? UNKNOWN_LOCATION,
          deviceIds: [t.deviceId],
        });
      }
      return;
    }

    if (!rec.location) return; // unassigned: no room-based rules
    const rk = roomKey(rec.location);

    switch (t.state.kind) {
      case "motion": {
        if (t.state.motion) this.lastMotionByRoom.set(rk, ts);
        break;
      }
      case "stove": {
        const prev = this.stoveState.get(t.deviceId);
        if (t.state.on && (!prev || !prev.on)) {
          this.stoveState.set(t.deviceId, { on: true, onSince: ts });
        } else if (!t.state.on) {
          this.stoveState.set(t.deviceId, { on: false, onSince: 0 });
          this.silentClear(`stove_on_no_motion:${t.deviceId}`);
        }
        break;
      }
      case "door": {
        const prev = this.doorState.get(t.deviceId);
        if (t.state.open && (!prev || !prev.open)) {
          this.doorState.set(t.deviceId, { open: true, openedAt: ts });
        } else if (!t.state.open) {
          this.doorState.set(t.deviceId, { open: false, openedAt: 0 });
          this.silentClear(`door_open_no_motion:${t.deviceId}`);
        }
        break;
      }
      case "bed": {
        const prev = this.bedState.get(t.deviceId);
        if (!t.state.occupied && (!prev || prev.occupied)) {
          this.bedState.set(t.deviceId, { occupied: false, leftAt: ts });
        } else if (t.state.occupied) {
          this.bedState.set(t.deviceId, { occupied: true, leftAt: 0 });
          this.silentClear(`bed_left_at_night_no_return:${t.deviceId}`);
        }
        break;
      }
    }
  }

  // ----- Periodic evaluation ----------------------------------------------

  evaluate(): void {
    // Rule 1: stove on for a long time, but no motion in the room.
    for (const [id, st] of this.stoveState) {
      if (!st.on) continue;
      const rec = this.registry.get(id);
      if (!rec?.location) continue;
      const onSeconds = (now() - st.onSince) / 1000;
      const rk = roomKey(rec.location);
      const noMotionSeconds = this.secondsSinceMotion(rk);
      if (onSeconds >= config.stoveOnSeconds && noMotionSeconds >= config.stoveNoMotionSeconds) {
        this.raise(`stove_on_no_motion:${id}`, {
          rule: "stove_on_no_motion",
          severity: "critical",
          message:
            `Stove in ${rec.location.room} has been on for ${Math.round(onSeconds)}s, ` +
            `but ${this.fmtSinceMotion(noMotionSeconds)}.`,
          location: rec.location,
          deviceIds: [id, ...this.motionSensorsInRoom(rk)],
        });
      }
    }

    // Rule 2: door opened, then unusually long no motion.
    for (const [id, st] of this.doorState) {
      if (!st.open) continue;
      const rec = this.registry.get(id);
      if (!rec?.location) continue;
      const openSeconds = (now() - st.openedAt) / 1000;
      const rk = roomKey(rec.location);
      const noMotionSeconds = this.secondsSinceMotion(rk);
      if (openSeconds >= config.doorOpenSeconds && noMotionSeconds >= config.doorNoMotionSeconds) {
        this.raise(`door_open_no_motion:${id}`, {
          rule: "door_open_no_motion",
          severity: "warning",
          message:
            `Door in ${rec.location.room} has been open for ${Math.round(openSeconds)}s, ` +
            `then ${this.fmtSinceMotion(noMotionSeconds)}.`,
          location: rec.location,
          deviceIds: [id, ...this.motionSensorsInRoom(rk)],
        });
      }
    }

    // Rule 3: bed left at night and no return for a long time.
    for (const [id, st] of this.bedState) {
      if (st.occupied) continue;
      const rec = this.registry.get(id);
      if (!rec?.location) continue;
      const absenceSeconds = (now() - st.leftAt) / 1000;
      if (this.isNight() && absenceSeconds >= config.bedAbsenceSeconds) {
        this.raise(`bed_left_at_night_no_return:${id}`, {
          rule: "bed_left_at_night_no_return",
          severity: "critical",
          message:
            `Bed in ${rec.location.room} was left at night and no return detected ` +
            `for ${Math.round(absenceSeconds)}s.`,
          location: rec.location,
          deviceIds: [id],
        });
      }
    }
  }

  // ----- Helpers -----------------------------------------------------------

  private motionSensorsInRoom(rk: string): string[] {
    return this.registry
      .all()
      .filter((d) => d.type === "motion" && d.location && roomKey(d.location) === rk)
      .map((d) => d.deviceId);
  }
  private secondsSinceMotion(rk: string): number {
    const last = this.lastMotionByRoom.get(rk);
    return last === undefined ? Number.POSITIVE_INFINITY : (now() - last) / 1000;
  }
  private fmtSinceMotion(seconds: number): string {
    return Number.isFinite(seconds)
      ? `no motion in the room for ${Math.round(seconds)}s`
      : `no active motion sensor in the room`;
  }
  private isNight(): boolean {
    if (config.forceNight) return true;
    const h = new Date().getHours();
    const { nightStartHour: s, nightEndHour: e } = config;
    return s > e ? h >= s || h < e : h >= s && h < e;
  }
}
