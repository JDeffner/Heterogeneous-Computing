/**
 * `pnpm run clear-devices` — reset the demo to a clean slate.
 *
 * Clears every retained message (registry, presence, rooms, alarms) on the
 * broker and deletes the hub's persisted data files. Stop the hub first
 * (otherwise it keeps its in-memory state and may re-publish it).
 */
import { rmSync } from "node:fs";
import { join } from "node:path";
import { connect } from "../src/shared/mqtt.js";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function main(): Promise<void> {
  const client = await connect({ clientId: "clear-devices" });
  const retained = new Set<string>();

  client.on("message", (topic, buf) => {
    // Only retained messages with a payload need clearing.
    if (buf.length > 0) retained.add(topic);
  });
  client.subscribe([
    "smarthome/registry/+",
    "smarthome/devices/+/presence",
    "smarthome/rooms/+",
    "smarthome/alarms/+",
  ]);

  await sleep(1000);

  for (const topic of retained) {
    client.publish(topic, "", { retain: true, qos: 1 });
  }
  await sleep(500);
  console.log(`Cleared ${retained.size} retained message(s) on the broker.`);

  // Wipe persisted state so a hub restart does not resurrect the devices.
  const dataDir = process.env.DATA_DIR ?? join(process.cwd(), "data");
  for (const file of ["devices.json", "rooms.json"]) {
    try {
      rmSync(join(dataDir, file));
      console.log(`Deleted ${file}.`);
    } catch {
      /* not present - fine */
    }
  }

  client.end(true, () => process.exit(0));
  setTimeout(() => process.exit(0), 1000);
}

main().catch((err) => {
  console.error("clear-devices failed:", err instanceof Error ? err.message : err);
  process.exit(1);
});
