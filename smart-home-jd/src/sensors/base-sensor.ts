/**
 * Base class for all simulated sensors.
 *
 * A sensor process EMBODIES one device whose static descriptor (the
 * {@link DeviceRecord}) it received from the hub - either freshly minted when a
 * new device was registered, or read from the retained registry when "taking
 * over" a previously installed device. The sensor then owns the device's live
 * half:
 *
 *   1. presence (retained + last-will): "online" while the process runs,
 *      "offline" when it exits or crashes. THIS is what makes "closing the
 *      console window takes the device offline" work, with no extra plumbing.
 *   2. telemetry: periodic samples of state + battery + signal strength.
 *
 * It reacts to commands on its command topic (manual control, link simulation)
 * and watches its own registry topic so descriptor changes made by the hub
 * (e.g. a room assignment) are reflected live.
 *
 * The class is an EventEmitter so an interactive console can render a live
 * dashboard: it emits "update" (state/metrics changed) and "log" (human-readable
 * line) events. Headless callers (smoke test, wake launcher) simply ignore them.
 */
import { EventEmitter } from "node:events";
import type { MqttClient } from "mqtt";
import { connect, publishJson } from "../shared/mqtt.js";
import { topics } from "../shared/topics.js";
import type {
  DeviceRecord,
  SensorCommand,
  SensorMode,
  Telemetry,
} from "../shared/types.js";
import { DEVICE_CATALOG } from "./device-catalog.js";

export interface SensorSnapshot {
  record: DeviceRecord;
  mode: SensorMode;
  /** Simulated network link (false = pretending to be unreachable). */
  linkUp: boolean;
  battery: number;
  rssi: number;
  state: Telemetry["state"];
  telemetryCount: number;
  lastPublish?: string;
}

export abstract class BaseSensor extends EventEmitter {
  protected client!: MqttClient;
  protected record: DeviceRecord;
  protected mode: SensorMode;

  private battery: number;
  private rssi: number;
  private linkUp = true;
  private telemetryCount = 0;
  private lastPublish?: string;
  private timer?: NodeJS.Timeout;
  private readonly reportIntervalMs: number;

  constructor(record: DeviceRecord, mode: SensorMode = "auto") {
    super();
    this.record = record;
    this.mode = mode;
    const cat = DEVICE_CATALOG[record.type];
    this.reportIntervalMs = cat.reportIntervalMs;
    // Battery devices start partly used; mains devices report a constant 100%.
    this.battery =
      record.powerSource === "battery" ? 70 + Math.floor(Math.random() * 30) : 100;
    this.rssi = cat.nominalRssi;
  }

  // ----- Lifecycle ---------------------------------------------------------

  async start(): Promise<void> {
    const presenceTopic = topics.presence(this.record.deviceId);

    // Last-will: if this process dies, the broker publishes "offline" for us.
    this.client = await connect({
      clientId: `sensor-${this.record.deviceId}`,
      will: { topic: presenceTopic, payload: "offline", retain: true },
    });

    // Announce we are alive (retained, so the hub/UI see it even if they connect
    // later).
    this.client.publish(presenceTopic, "online", { retain: true, qos: 1 });

    // Listen for commands addressed to this device, and for descriptor changes
    // the hub makes in the registry (e.g. room assignment).
    const commandTopic = topics.command(this.record.deviceId);
    const registryTopic = topics.registry(this.record.deviceId);
    this.client.subscribe([commandTopic, registryTopic]);
    this.client.on("message", (topic, buf) => {
      const raw = buf.toString();
      if (raw.length === 0) return;
      if (topic === commandTopic) {
        try {
          this.handleCommand(JSON.parse(raw) as SensorCommand);
        } catch (err) {
          this.log(`invalid command: ${String(err)}`);
        }
      } else if (topic === registryTopic) {
        try {
          this.applyRegistryUpdate(JSON.parse(raw) as DeviceRecord);
        } catch {
          /* ignore */
        }
      }
    });

    this.log(`online as ${this.record.type} (${this.record.deviceId})`);

    // First report immediately, then periodically.
    this.publishTelemetry();
    this.timer = setInterval(() => this.tick(), this.reportIntervalMs);
    this.emit("update", this.snapshot());
  }

  /**
   * Gracefully go offline: publish a retained "offline" presence, then close the
   * connection. Does not exit the process - the owner decides.
   */
  async stop(): Promise<void> {
    if (this.timer) clearInterval(this.timer);
    if (!this.client) return;
    const presenceTopic = topics.presence(this.record.deviceId);
    await new Promise<void>((resolve) => {
      this.client.publish(presenceTopic, "offline", { retain: true, qos: 1 }, () => {
        this.client.end(false, undefined, () => resolve());
      });
      setTimeout(resolve, 1500); // safety net
    });
  }

  // ----- Simulation loop ---------------------------------------------------

  private tick(): void {
    if (!this.linkUp) return; // unreachable: stay silent
    if (this.mode === "auto") this.simulateStep();
    this.updateMetrics();
    this.publishTelemetry();
    this.emit("update", this.snapshot());
  }

  /** Slowly drain the battery (battery devices) and jitter the signal. */
  private updateMetrics(): void {
    if (this.record.powerSource === "battery") {
      this.battery = Math.max(1, this.battery - Math.random() * 0.4);
    }
    const nominal = DEVICE_CATALOG[this.record.type].nominalRssi;
    // Random walk that drifts back toward the nominal value.
    this.rssi = Math.round(
      Math.max(-95, Math.min(-35, this.rssi + (Math.random() - 0.5) * 6 + (nominal - this.rssi) * 0.2)),
    );
  }

  private publishTelemetry(): void {
    const msg: Telemetry = {
      deviceId: this.record.deviceId,
      type: this.record.type,
      state: this.currentState(),
      mode: this.mode,
      battery: Math.round(this.battery),
      rssi: this.rssi,
      ts: new Date().toISOString(),
    };
    publishJson(this.client, topics.telemetry(this.record.deviceId), msg, { qos: 1 });
    this.telemetryCount += 1;
    this.lastPublish = msg.ts;
  }

  // ----- Commands ----------------------------------------------------------

  private handleCommand(cmd: SensorCommand): void {
    if (cmd.mode) {
      this.mode = cmd.mode;
      this.log(`mode -> ${this.mode}`);
    }

    if (typeof cmd.online === "boolean") {
      this.setLink(cmd.online);
      if (!this.linkUp) {
        this.emit("update", this.snapshot());
        return; // gone offline: emit nothing further
      }
    }

    this.applyCommand(cmd);
    this.publishTelemetry();
    this.emit("update", this.snapshot());
  }

  /** Bring the simulated network link up or down (retained presence update). */
  private setLink(up: boolean): void {
    if (up === this.linkUp) return;
    this.linkUp = up;
    this.client.publish(
      topics.presence(this.record.deviceId),
      up ? "online" : "offline",
      { retain: true, qos: 1 },
    );
    this.log(`simulated link ${up ? "UP (online)" : "DOWN (offline)"}`);
  }

  /** Pick up descriptor changes the hub made (e.g. a new room assignment). */
  private applyRegistryUpdate(rec: DeviceRecord): void {
    const locChanged =
      JSON.stringify(rec.location ?? null) !== JSON.stringify(this.record.location ?? null);
    const nameChanged = rec.name !== this.record.name;
    this.record = { ...this.record, location: rec.location, name: rec.name };
    if (locChanged) {
      this.log(
        `assigned to ${rec.location ? rec.location.room : "(unassigned)"} by hub`,
      );
    }
    if (locChanged || nameChanged) this.emit("update", this.snapshot());
  }

  // ----- Console support ---------------------------------------------------

  snapshot(): SensorSnapshot {
    return {
      record: this.record,
      mode: this.mode,
      linkUp: this.linkUp,
      battery: Math.round(this.battery),
      rssi: this.rssi,
      state: this.currentState(),
      telemetryCount: this.telemetryCount,
      lastPublish: this.lastPublish,
    };
  }

  /**
   * Manually flip the device's primary state (console spacebar). Switches to
   * manual mode so the change "sticks" instead of being overwritten next tick.
   */
  toggle(): void {
    this.mode = "manual";
    this.flipPrimary();
    this.publishTelemetry();
    this.log(`manual toggle -> ${this.describeState()}`);
    this.emit("update", this.snapshot());
  }

  /** Switch between auto and manual (console 'm' key). */
  setMode(mode: SensorMode): void {
    if (mode === this.mode) return;
    this.mode = mode;
    this.publishTelemetry();
    this.log(`mode -> ${mode}`);
    this.emit("update", this.snapshot());
  }

  /** Simulate the network link from the console (console 'o' key). */
  setLinkFromConsole(up: boolean): void {
    this.setLink(up);
    this.emit("update", this.snapshot());
  }

  isLinkUp(): boolean {
    return this.linkUp;
  }

  /** Human-readable one-line state for the console/log. */
  abstract describeState(): string;

  protected log(line: string): void {
    this.emit("log", line);
  }

  // ----- Type-specific behaviour (subclasses) ------------------------------

  protected abstract simulateStep(): void;
  protected abstract currentState(): Telemetry["state"];
  protected abstract applyCommand(cmd: SensorCommand): void;
  /** Flip the primary boolean of this device type (for the manual toggle). */
  protected abstract flipPrimary(): void;
}
