/**
 * The central HUB - the heart of the centralized architecture.
 *
 * Every sensor communicates with the hub (via the MQTT broker). The hub is the
 * single owner of:
 *   - the device REGISTRY (onboarding + persistence + online/offline tracking),
 *   - the ROOM registry ("QR codes" for location assignment),
 *   - RULE evaluation (deriving situations from telemetry/presence), and
 *   - ALARM management.
 *
 * It is still loosely coupled to everything else: it only talks MQTT + a JSON
 * file on disk. If a UI or a sensor is down, the hub keeps running; if the hub
 * restarts, it rebuilds its retained state from disk.
 */
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import type { MqttClient } from "mqtt";
import { connect, publishJson, newId } from "../shared/mqtt.js";
import {
  subscriptions,
  topics,
  deviceIdFromTopic,
  serialFromPairing,
} from "../shared/topics.js";
import type {
  AlarmControl,
  CommissionRequest,
  CommissionedMessage,
  DeviceControl,
  HubControl,
  HubStatus,
  Location,
  PairingAd,
  RegisterAck,
  RegisterRequest,
  Room,
  RoomControl,
  Telemetry,
} from "../shared/types.js";
import { config } from "./config.js";
import { Registry } from "./registry.js";
import { RuleEngine } from "./rules.js";
import { AlarmEngine } from "./alarms.js";

const DATA_DIR = config.dataDir || join(process.cwd(), "data");
const ROOMS_FILE = join(DATA_DIR, "rooms.json");

const rooms = new Map<string, Room>();
/** Devices currently advertising in pairing mode, keyed by serial. */
const pairingAds = new Map<string, PairingAd>();

function persistRooms(): void {
  try {
    mkdirSync(DATA_DIR, { recursive: true });
    writeFileSync(ROOMS_FILE, JSON.stringify([...rooms.values()], null, 2));
  } catch (err) {
    console.warn(`[hub] could not write ${ROOMS_FILE}: ${String(err)}`);
  }
}

function slug(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, "") || "room";
}

async function main(): Promise<void> {
  const client: MqttClient = await connect({ clientId: "hub" });
  const registry = new Registry(client, DATA_DIR);
  const alarms = new AlarmEngine(client);
  const rules = new RuleEngine(registry, (ev) => {
    // Publish the situation for the UI event log, then drive the alarm engine.
    publishJson(client, topics.event(ev.eventId), ev, { qos: 1 });
    console.log(`EVENT [${ev.severity}]${ev.cleared ? " CLEARED" : ""} ${ev.message}`);
    alarms.handleEvent(ev);
  });

  // Restore rooms (retained) and the device registry from disk.
  for (const room of readRooms()) {
    rooms.set(room.roomId, room);
    publishJson(client, topics.room(room.roomId), room, { retain: true });
  }
  registry.load();
  publishHubStatus(client);

  client.subscribe([
    subscriptions.allPresence,
    subscriptions.allTelemetry,
    subscriptions.register,
    subscriptions.allPairing,
    subscriptions.commissionControl,
    subscriptions.roomControl,
    subscriptions.deviceControl,
    subscriptions.alarmControl,
    subscriptions.hubControl,
  ]);

  client.on("message", (topic, buf) => {
    const raw = buf.toString();

    // Presence is plain text ("online"/"offline").
    if (topic.endsWith("/presence")) {
      const id = deviceIdFromTopic(topic);
      if (id && raw.length > 0) {
        const online = raw === "online";
        registry.setStatus(id, online ? "online" : "offline");
        rules.handlePresence(id, online);
      }
      return;
    }

    // Pairing advertisements are retained; an empty payload clears one.
    const serial = serialFromPairing(topic);
    if (serial) {
      if (raw.length === 0) {
        pairingAds.delete(serial);
      } else {
        try {
          pairingAds.set(serial, JSON.parse(raw) as PairingAd);
        } catch {
          /* ignore malformed ad */
        }
      }
      return;
    }

    if (raw.length === 0) return; // cleared retained message
    try {
      if (topic.endsWith("/telemetry")) {
        rules.handleTelemetry(JSON.parse(raw) as Telemetry);
      } else if (topic === topics.register()) {
        handleRegister(client, registry, JSON.parse(raw) as RegisterRequest);
      } else if (topic === topics.commissionControl()) {
        handleCommission(client, registry, JSON.parse(raw) as CommissionRequest);
      } else if (topic === topics.roomControl()) {
        handleRoomControl(client, registry, JSON.parse(raw) as RoomControl);
      } else if (topic === topics.deviceControl()) {
        handleDeviceControl(registry, JSON.parse(raw) as DeviceControl);
      } else if (topic === topics.alarmControl()) {
        alarms.handleControl(JSON.parse(raw) as AlarmControl);
      } else if (topic === topics.hubControl()) {
        handleHubControl(client, JSON.parse(raw) as HubControl);
      }
    } catch (err) {
      console.warn(`[hub] invalid message on ${topic}: ${String(err)}`);
    }
  });

  setInterval(() => rules.evaluate(), config.tickMs);

  console.log(
    `[hub] running. ${rooms.size} room(s), ${registry.all().length} device(s). ` +
      `night=${config.forceNight ? "forced" : `${config.nightStartHour}-${config.nightEndHour}h`}. ` +
      `Data dir: ${DATA_DIR}`,
  );

  const shutdown = () => client.end(true, () => process.exit(0));
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

// ----- Onboarding ----------------------------------------------------------

function handleRegister(client: MqttClient, registry: Registry, req: RegisterRequest): void {
  let ack: RegisterAck;
  try {
    const record = registry.register(req.type, req.desiredId, req.location);
    ack = { reqId: req.reqId, ok: true, record };
  } catch (err) {
    ack = { reqId: req.reqId, ok: false, error: String(err) };
  }
  publishJson(client, topics.registerAck(req.reqId), ack, { qos: 1 });
}

/** Commission a device that is advertising in pairing mode (the faithful path). */
function handleCommission(client: MqttClient, registry: Registry, req: CommissionRequest): void {
  const ad = pairingAds.get(req.serial);
  if (!ad) {
    console.warn(`[hub] commission for unknown/expired serial ${req.serial}`);
    return;
  }
  const location = resolveLocation(req.roomId);
  const record = registry.commission(ad, req.name, location);

  // Tell the device its operational identity, then clear the retained pairing ad.
  const msg: CommissionedMessage = {
    serial: req.serial,
    deviceId: record.deviceId,
    name: record.name,
    location: record.location,
  };
  publishJson(client, topics.pairingCommissioned(req.serial), msg, { qos: 1 });
  client.publish(topics.pairing(req.serial), "", { retain: true, qos: 1 });
  pairingAds.delete(req.serial);
}

/** Resolve a roomId to a full Location (null/undefined -> unassigned). */
function resolveLocation(roomId: string | null | undefined): Location | undefined {
  if (!roomId) return undefined;
  const room = rooms.get(roomId);
  if (!room) {
    console.warn(`[hub] unknown room ${roomId} (leaving unassigned)`);
    return undefined;
  }
  return { building: room.building, floor: room.floor, room: room.room, roomId: room.roomId };
}

/** Runtime hub tweaks from the UI (the "Simulate night" button). */
function handleHubControl(client: MqttClient, ctrl: HubControl): void {
  if (typeof ctrl.forceNight === "boolean") {
    config.forceNight = ctrl.forceNight;
    console.log(`[hub] forceNight -> ${config.forceNight}`);
    publishHubStatus(client);
  }
}

function publishHubStatus(client: MqttClient): void {
  const status: HubStatus = {
    nightStartHour: config.nightStartHour,
    nightEndHour: config.nightEndHour,
    forceNight: config.forceNight,
  };
  publishJson(client, topics.hubStatus(), status, { retain: true });
}

// ----- Rooms ---------------------------------------------------------------

function handleRoomControl(client: MqttClient, registry: Registry, ctrl: RoomControl): void {
  if (ctrl.action === "create") {
    const roomId = `${slug(ctrl.room)}-${newId("r").split("-").pop()}`;
    const room: Room = {
      roomId,
      building: ctrl.building,
      floor: ctrl.floor,
      room: ctrl.room,
      createdAt: new Date().toISOString(),
    };
    rooms.set(roomId, room);
    persistRooms();
    publishJson(client, topics.room(roomId), room, { retain: true });
    console.log(`[hub] room created: ${roomId} (${room.room})`);
    return;
  }

  if (ctrl.action === "delete") {
    if (!rooms.delete(ctrl.roomId)) return;
    persistRooms();
    client.publish(topics.room(ctrl.roomId), "", { retain: true, qos: 1 });
    // Unassign any device placed in this room.
    for (const d of registry.all()) {
      if (d.location?.roomId === ctrl.roomId) registry.assign(d.deviceId, null);
    }
    console.log(`[hub] room deleted: ${ctrl.roomId}`);
  }
}

// ----- Device control ------------------------------------------------------

function handleDeviceControl(registry: Registry, ctrl: DeviceControl): void {
  if (ctrl.action === "assign") {
    if (ctrl.roomId && !rooms.has(ctrl.roomId)) {
      console.warn(`[hub] assign to unknown room ${ctrl.roomId}`);
      return;
    }
    registry.assign(ctrl.deviceId, resolveLocation(ctrl.roomId) ?? null);
    return;
  }
  if (ctrl.action === "remove") {
    registry.remove(ctrl.deviceId);
  }
}

// ----- Persistence read ----------------------------------------------------

function readRooms(): Room[] {
  try {
    if (existsSync(ROOMS_FILE)) return JSON.parse(readFileSync(ROOMS_FILE, "utf8")) as Room[];
  } catch (err) {
    console.warn(`[hub] could not read ${ROOMS_FILE}: ${String(err)}`);
  }
  return [];
}

main().catch((err) => {
  console.error("Hub start failed:", err);
  process.exit(1);
});
