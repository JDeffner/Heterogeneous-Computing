/**
 * Operator web console. A small Express server that mirrors the MQTT bus to the
 * browser via Server-Sent Events. It holds no own truth - if it goes down, the
 * hub and the sensors keep running (loose coupling).
 *
 * It reads the hub's retained registry (the device list + online/offline state),
 * live telemetry, rooms, events and alarms, and offers endpoints to create rooms,
 * assign a device to a room (the "QR scan"), resolve an alarm, send a manual
 * command to a device, or remove a device - each by publishing the corresponding
 * control message that the hub acts upon.
 */
import express from "express";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import type { MqttClient } from "mqtt";
import { connect, publishJson } from "../shared/mqtt.js";
import {
  subscriptions,
  topics,
  deviceIdFromTopic,
  deviceIdFromRegistry,
  roomIdFromTopic,
  serialFromPairing,
} from "../shared/topics.js";
import type {
  Alarm,
  AlarmControl,
  CommissionRequest,
  DeviceControl,
  DeviceRecord,
  HubControl,
  HubStatus,
  PairingAd,
  Room,
  RoomControl,
  SensorCommand,
  SensorMode,
  SituationEvent,
  Telemetry,
} from "../shared/types.js";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PORT = Number(process.env.WEB_PORT ?? 3000);

interface DeviceView {
  /** Hub registry descriptor (named `announce` for the existing UI markup). */
  announce: DeviceRecord;
  online: boolean;
  lastState?: Telemetry["state"];
  mode?: SensorMode;
  battery?: number;
  rssi?: number;
  lastSeen?: string;
}

const devices = new Map<string, DeviceView>();
const rooms = new Map<string, Room>();
const alarms = new Map<string, Alarm>();
const pairing = new Map<string, PairingAd>();
const events: SituationEvent[] = [];
let hubStatus: HubStatus | null = null;
let mqttClient: MqttClient | null = null;

function roomNameOf(d: DeviceView): string {
  return d.announce.location?.room ?? "";
}

function snapshot() {
  return {
    devices: [...devices.values()].sort((a, b) => roomNameOf(a).localeCompare(roomNameOf(b))),
    rooms: [...rooms.values()].sort((a, b) => a.room.localeCompare(b.room)),
    alarms: [...alarms.values()].sort((a, b) => b.raisedAt.localeCompare(a.raisedAt)),
    pairing: [...pairing.values()].sort((a, b) => a.advertisedAt.localeCompare(b.advertisedAt)),
    hubStatus,
    events: events.slice(-25).reverse(),
    ts: new Date().toISOString(),
  };
}

// ----- SSE clients ---------------------------------------------------------

const sseClients = new Set<express.Response>();
function broadcast(): void {
  const data = `data: ${JSON.stringify(snapshot())}\n\n`;
  for (const res of sseClients) res.write(data);
}

// ----- MQTT ----------------------------------------------------------------

async function startMqtt(): Promise<void> {
  const client = await connect({ clientId: "web-ui" });
  mqttClient = client;
  client.subscribe([
    subscriptions.allRegistry,
    subscriptions.allPresence,
    subscriptions.allTelemetry,
    subscriptions.allPairing,
    subscriptions.allEvents,
    subscriptions.allAlarms,
    subscriptions.allRooms,
    subscriptions.hubStatus,
  ]);

  client.on("message", (topic, buf) => {
    const raw = buf.toString();

    // Presence is plain text.
    if (topic.endsWith("/presence")) {
      const id = deviceIdFromTopic(topic);
      if (id && raw.length > 0) {
        const d = devices.get(id);
        if (d) d.online = raw === "online";
      }
      broadcast();
      return;
    }

    // Pairing advertisements (retained; empty payload clears one).
    const serial = serialFromPairing(topic);
    if (serial) {
      if (raw.length === 0) pairing.delete(serial);
      else {
        try {
          pairing.set(serial, JSON.parse(raw) as PairingAd);
        } catch {
          /* ignore */
        }
      }
      broadcast();
      return;
    }

    // Empty payload = a retained message was cleared.
    if (raw.length === 0) {
      const regId = deviceIdFromRegistry(topic);
      if (regId) {
        devices.delete(regId);
      } else if (topic.startsWith("smarthome/alarms/")) {
        const alarmId = topic.split("/").pop();
        if (alarmId) alarms.delete(alarmId);
      } else if (topic.startsWith("smarthome/rooms/")) {
        const roomId = roomIdFromTopic(topic);
        if (roomId) rooms.delete(roomId);
      }
      broadcast();
      return;
    }

    let payload: unknown;
    try {
      payload = JSON.parse(raw);
    } catch {
      return;
    }

    const regId = deviceIdFromRegistry(topic);
    if (regId) {
      const rec = payload as DeviceRecord;
      const existing = devices.get(rec.deviceId);
      devices.set(rec.deviceId, {
        announce: rec,
        online: existing?.online ?? rec.status === "online",
        lastState: existing?.lastState,
        mode: existing?.mode,
        battery: existing?.battery,
        rssi: existing?.rssi,
        lastSeen: existing?.lastSeen,
      });
    } else if (topic.endsWith("/telemetry")) {
      const t = payload as Telemetry;
      const d = devices.get(t.deviceId);
      if (d) {
        d.lastState = t.state;
        d.mode = t.mode;
        d.battery = t.battery;
        d.rssi = t.rssi;
        d.lastSeen = t.ts;
      }
    } else if (topic.startsWith("smarthome/rooms/")) {
      const r = payload as Room;
      rooms.set(r.roomId, r);
    } else if (topic.startsWith("smarthome/events/")) {
      events.push(payload as SituationEvent);
      if (events.length > 200) events.shift();
    } else if (topic.startsWith("smarthome/alarms/")) {
      const al = payload as Alarm;
      alarms.set(al.alarmId, al);
    } else if (topic === topics.hubStatus()) {
      hubStatus = payload as HubStatus;
    }

    broadcast();
  });
}

// ----- HTTP ----------------------------------------------------------------

const app = express();
app.use(express.json());
app.use(express.static(join(__dirname, "public")));

function requireMqtt(res: express.Response): MqttClient | null {
  if (!mqttClient) {
    res.status(503).json({ error: "MQTT not connected" });
    return null;
  }
  return mqttClient;
}

app.get("/api/state", (_req, res) => res.json(snapshot()));

app.get("/api/stream", (req, res) => {
  res.set({
    "Content-Type": "text/event-stream",
    "Cache-Control": "no-cache",
    Connection: "keep-alive",
  });
  res.flushHeaders();
  res.write(`data: ${JSON.stringify(snapshot())}\n\n`);
  sseClients.add(res);
  req.on("close", () => sseClients.delete(res));
});

// Resolve an alarm.
app.post("/api/alarms/:id/resolve", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  const ctrl: AlarmControl = { alarmId: req.params.id, action: "resolve" };
  publishJson(client, topics.alarmControl(), ctrl, { qos: 1 });
  res.json({ ok: true });
});

// Create a room (the hub assigns the id, persists and publishes it).
app.post("/api/rooms", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  const { building, floor, room } = req.body as Partial<Room>;
  if (!building || !floor || !room) {
    res.status(400).json({ error: "building, floor and room are required" });
    return;
  }
  const ctrl: RoomControl = { action: "create", building, floor, room };
  publishJson(client, topics.roomControl(), ctrl, { qos: 1 });
  res.json({ ok: true });
});

// Delete a room.
app.delete("/api/rooms/:roomId", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  const ctrl: RoomControl = { action: "delete", roomId: req.params.roomId };
  publishJson(client, topics.roomControl(), ctrl, { qos: 1 });
  res.json({ ok: true });
});

// Assign a device to a room (or unassign with roomId null) - the "QR scan".
// The hub owns the registry, so this goes through device control.
app.post("/api/devices/:id/assign", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  const { roomId } = req.body as { roomId?: string | null };
  const ctrl: DeviceControl = { action: "assign", deviceId: req.params.id, roomId: roomId ?? null };
  publishJson(client, topics.deviceControl(), ctrl, { qos: 1 });
  res.json({ ok: true });
});

// Send a manual command to a live device (e.g. simulate offline from the UI).
app.post("/api/devices/:id/command", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  publishJson(client, topics.command(req.params.id), req.body as SensorCommand, { qos: 1 });
  res.json({ ok: true });
});

// Remove a device from the registry entirely.
app.delete("/api/devices/:id", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  const ctrl: DeviceControl = { action: "remove", deviceId: req.params.id };
  publishJson(client, topics.deviceControl(), ctrl, { qos: 1 });
  res.json({ ok: true });
});

// Commission a device that is advertising in pairing mode.
app.post("/api/commission", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  const { serial, name, roomId } = req.body as Partial<CommissionRequest>;
  if (!serial) {
    res.status(400).json({ error: "serial is required" });
    return;
  }
  const ctrl: CommissionRequest = { serial, name, roomId: roomId ?? null };
  publishJson(client, topics.commissionControl(), ctrl, { qos: 1 });
  res.json({ ok: true });
});

// Toggle the hub's "force night" demo override (the Simulate night button).
app.post("/api/hub/force-night", (req, res) => {
  const client = requireMqtt(res);
  if (!client) return;
  const { on } = req.body as { on?: boolean };
  const ctrl: HubControl = { forceNight: !!on };
  publishJson(client, topics.hubControl(), ctrl, { qos: 1 });
  res.json({ ok: true });
});

app.listen(PORT, () => console.log(`Operator console at http://localhost:${PORT}`));

startMqtt().catch((err) => {
  console.error("Web UI MQTT start failed:", err);
  process.exit(1);
});
