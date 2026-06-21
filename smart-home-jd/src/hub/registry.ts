/**
 * The hub's persisted DEVICE REGISTRY - the single owner of every device's
 * static descriptor.
 *
 * Each record is published RETAINED on `smarthome/registry/<id>` so that:
 *   - the operator console / UIs always see the whole fleet, and
 *   - a sensor console can list and "take over" devices that are OFFLINE.
 * The records also persist to data/devices.json so the fleet survives restarts
 * (and a broker restart, which wipes retained messages).
 */
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import type { MqttClient } from "mqtt";
import { publishJson, newId } from "../shared/mqtt.js";
import { topics } from "../shared/topics.js";
import {
  SENSOR_TYPE_LABELS,
  type DeviceRecord,
  type Location,
  type PairingAd,
  type SensorType,
} from "../shared/types.js";
import { DEVICE_CATALOG, makeSerial } from "../sensors/device-catalog.js";

function deriveName(type: SensorType, location?: Location): string {
  const label = SENSOR_TYPE_LABELS[type];
  return location ? `${label} - ${location.room}` : `${label} (unassigned)`;
}

export class Registry {
  private records = new Map<string, DeviceRecord>();
  private readonly file: string;

  constructor(
    private readonly client: MqttClient,
    private readonly dataDir: string,
  ) {
    this.file = join(dataDir, "devices.json");
  }

  /** Load persisted records, mark them offline (no live process yet), republish. */
  load(): void {
    let stored: DeviceRecord[] = [];
    try {
      if (existsSync(this.file)) {
        stored = JSON.parse(readFileSync(this.file, "utf8")) as DeviceRecord[];
      }
    } catch (err) {
      console.warn(`[hub] could not read ${this.file}: ${String(err)}`);
    }
    for (const rec of stored) {
      rec.status = "offline"; // a retained presence message will flip it back
      this.records.set(rec.deviceId, rec);
      this.publish(rec);
    }
    console.log(`[hub] registry loaded: ${this.records.size} device(s)`);
  }

  all(): DeviceRecord[] {
    return [...this.records.values()];
  }
  get(id: string): DeviceRecord | undefined {
    return this.records.get(id);
  }
  has(id: string): boolean {
    return this.records.has(id);
  }

  /**
   * Build and store a brand-new device. Used by both onboarding paths:
   *  - the direct register/ack shortcut (scripts, CLI `--type`), and
   *  - commissioning a device that was advertising in pairing mode.
   */
  private create(
    type: SensorType,
    opts: { deviceId?: string; serial?: string; location?: Location; name?: string } = {},
  ): DeviceRecord {
    const cat = DEVICE_CATALOG[type];
    let deviceId = opts.deviceId?.trim() || newId(type);
    if (this.records.has(deviceId)) deviceId = newId(type); // avoid collisions
    const location = opts.location;
    const record: DeviceRecord = {
      deviceId,
      type,
      name: opts.name?.trim() || deriveName(type, location),
      location,
      serialNumber: opts.serial ?? makeSerial(type),
      manufacturer: cat.manufacturer,
      model: cat.model,
      protocol: cat.protocol,
      powerSource: cat.powerSource,
      firmware: cat.firmware,
      capabilities: cat.capabilities,
      status: "offline",
      registeredAt: new Date().toISOString(),
    };
    this.records.set(deviceId, record);
    this.persist();
    this.publish(record);
    return record;
  }

  /** Direct registration (auto-commissioned) - scripts and CLI `--type`. */
  register(type: SensorType, desiredId?: string, location?: Location): DeviceRecord {
    const record = this.create(type, { deviceId: desiredId, location });
    console.log(`[hub] registered ${type} ${record.deviceId}`);
    return record;
  }

  /** Commission a device that advertised itself in pairing mode. */
  commission(ad: PairingAd, name?: string, location?: Location): DeviceRecord {
    const record = this.create(ad.type, { serial: ad.serial, name, location });
    console.log(`[hub] commissioned ${ad.type} ${record.deviceId} (serial ${ad.serial})`);
    return record;
  }

  /** Update liveness. Returns true if the status actually changed. */
  setStatus(id: string, status: "online" | "offline"): boolean {
    const rec = this.records.get(id);
    if (!rec) return false;
    const changed = rec.status !== status;
    rec.status = status;
    rec.lastSeen = new Date().toISOString();
    if (changed) {
      this.persist();
      this.publish(rec);
    }
    return changed;
  }

  /** Assign (or, with null, clear) a device's room. */
  assign(id: string, location: Location | null): void {
    const rec = this.records.get(id);
    if (!rec) return;
    rec.location = location ?? undefined;
    rec.name = deriveName(rec.type, rec.location);
    this.persist();
    this.publish(rec);
    console.log(
      `[hub] ${id} assigned -> ${rec.location ? rec.location.room : "(unassigned)"}`,
    );
  }

  remove(id: string): void {
    if (!this.records.delete(id)) return;
    this.persist();
    // Clear retained registry + presence so the device vanishes everywhere.
    this.client.publish(topics.registry(id), "", { retain: true, qos: 1 });
    this.client.publish(topics.presence(id), "", { retain: true, qos: 1 });
    console.log(`[hub] removed ${id}`);
  }

  private publish(rec: DeviceRecord): void {
    publishJson(this.client, topics.registry(rec.deviceId), rec, { retain: true });
  }

  private persist(): void {
    try {
      mkdirSync(this.dataDir, { recursive: true });
      writeFileSync(this.file, JSON.stringify(this.all(), null, 2));
    } catch (err) {
      console.warn(`[hub] could not write ${this.file}: ${String(err)}`);
    }
  }
}
