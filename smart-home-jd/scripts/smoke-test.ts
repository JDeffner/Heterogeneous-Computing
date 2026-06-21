/**
 * End-to-end smoke test without external dependencies.
 *
 * Starts an embedded MQTT broker (aedes), loads the HUB in-process against a
 * throwaway data dir, and exercises the real chain over MQTT:
 *
 *   1) Onboard a stove (register request -> hub ack -> retained registry).
 *   2) Stove stays on while no motion is reported -> "stove_on_no_motion" alarm
 *      (presence + telemetry -> rule -> alarm).
 *   3) Device goes offline (presence "offline") -> "device_offline" alarm; coming
 *      back online auto-resolves it (retained alarm cleared).
 */
import { createServer } from "node:net";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Aedes from "aedes";
import mqtt from "mqtt";
import { topics } from "../src/shared/topics.js";
import type { Location, Telemetry } from "../src/shared/types.js";

const PORT = 1884;
const URL = `mqtt://localhost:${PORT}`;

// Configure BEFORE importing the hub: broker url, fast thresholds, temp data dir.
process.env.MQTT_URL = URL;
process.env.EVAL_TICK_MS = "400";
process.env.STOVE_ON_SECONDS = "2";
process.env.STOVE_NO_MOTION_SECONDS = "2";
process.env.DATA_DIR = mkdtempSync(join(tmpdir(), "alsh-smoke-"));

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function waitFor(label: string, cond: () => boolean, ms = 10_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    if (cond()) return;
    await sleep(150);
  }
  throw new Error(`timeout waiting for: ${label}`);
}

function telemetry(deviceId: string): Telemetry {
  return {
    deviceId,
    type: "stove",
    state: { kind: "stove", on: true, temperatureC: 180 },
    mode: "manual",
    battery: 100,
    rssi: -50,
    ts: new Date().toISOString(),
  };
}

async function main(): Promise<void> {
  // 1) Embedded broker
  const aedes = new Aedes();
  const server = createServer(aedes.handle);
  await new Promise<void>((resolve) => server.listen(PORT, resolve));
  console.log(`[smoke] broker running at ${URL}`);

  // 2) Hub in-process (connects automatically)
  const { HubClient } = await import("../src/sensors/hub-client.js");
  await import("../src/hub/index.js");
  await sleep(900);

  // 3) Observer client (track alarms by rule + cleared ids)
  const client = mqtt.connect(URL, { clientId: "smoke-test" });
  const activeByRule = new Map<string, string>();
  const clearedIds = new Set<string>();
  await new Promise<void>((resolve) => client.once("connect", () => resolve()));
  client.subscribe("smarthome/alarms/+");
  client.on("message", (topic, buf) => {
    const raw = buf.toString();
    const alarmId = topic.split("/").pop() ?? "";
    if (!raw) {
      clearedIds.add(alarmId);
      return;
    }
    try {
      const a = JSON.parse(raw);
      if (a?.rule) activeByRule.set(a.rule, a.alarmId);
    } catch {
      /* ignore */
    }
  });

  // 4) Onboard a stove through the hub (the real registration path).
  const hub = await HubClient.open("smoke-hubclient");
  const location: Location = { building: "Test house", floor: "Ground floor", room: "Test kitchen" };
  const record = await hub.register("stove", "stove-smoke", location);
  const deviceId = record.deviceId;
  console.log(`[smoke] OK: hub registered ${deviceId}.`);

  // Act as the live sensor process.
  client.publish(topics.presence(deviceId), "online", { retain: true, qos: 1 });
  await sleep(300);

  // --- Scenario 1: stove on, no motion -> alarm -------------------------
  client.publish(topics.telemetry(deviceId), JSON.stringify(telemetry(deviceId)), { qos: 1 });
  await waitFor("stove_on_no_motion alarm", () => activeByRule.has("stove_on_no_motion"));
  console.log("[smoke] OK: 'stove_on_no_motion' alarm created.");

  // --- Scenario 2: offline -> alarm, reconnect -> auto-resolved ---------
  client.publish(topics.presence(deviceId), "offline", { retain: true, qos: 1 });
  await waitFor("device_offline alarm", () => activeByRule.has("device_offline"));
  const offlineAlarmId = activeByRule.get("device_offline")!;
  console.log(`[smoke] OK: 'device_offline' alarm created (${offlineAlarmId}).`);

  client.publish(topics.presence(deviceId), "online", { retain: true, qos: 1 });
  await waitFor("device_offline alarm auto-resolved", () => clearedIds.has(offlineAlarmId));
  console.log("[smoke] OK: 'device_offline' alarm auto-resolved on reconnect.");

  console.log("[smoke] PASS: full chain works (onboarding, rules, offline fault, auto-resolve).");
  process.exit(0);
}

main().catch((err) => {
  console.error(`[smoke] FAIL: ${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
});
