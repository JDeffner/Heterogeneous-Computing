/**
 * `pnpm run seed` — populate the hub with a few rooms and devices so there is
 * something to demonstrate immediately.
 *
 * It creates rooms, then registers a handful of devices (some placed in rooms,
 * some left unassigned). The devices are registered but NOT started, so they all
 * show up as OFFLINE in the registry. Run `pnpm run wake` afterwards to open a
 * console window for each of them, or `pnpm run sensor` to take one over by hand.
 *
 * Requires the hub to be running (`pnpm run hub`).
 */
import { HubClient } from "../src/sensors/hub-client.js";
import type { Location, SensorType } from "../src/shared/types.js";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const ROOMS = [
  ["House A", "Ground floor", "Kitchen"],
  ["House A", "Ground floor", "Living room"],
  ["House A", "Ground floor", "Bedroom"],
  ["House A", "Ground floor", "Bathroom"],
] as const;

// type, roomName (or null = unassigned)
const DEVICES: Array<[SensorType, string | null]> = [
  ["stove", "Kitchen"],
  ["motion", "Kitchen"],
  ["door", "Living room"],
  ["motion", "Living room"],
  ["bed", "Bedroom"],
  ["motion", "Bathroom"],
  ["sos", null], // wearable pendant: belongs to the person, not a room
];

async function main(): Promise<void> {
  const hub = await HubClient.open(`seed-${Date.now().toString(36)}`);

  // 1) Create rooms, then re-snapshot to learn their generated ids.
  for (const [building, floor, room] of ROOMS) hub.createRoom(building, floor, room);
  await sleep(1200);
  const { rooms, records } = await hub.snapshot();

  if (records.size > 0) {
    console.log(`Registry already has ${records.size} device(s); seeding more on top.`);
  }

  const roomByName = new Map<string, Location>();
  for (const r of rooms.values()) {
    roomByName.set(r.room, { building: r.building, floor: r.floor, room: r.room, roomId: r.roomId });
  }

  // 2) Register the devices.
  let n = 0;
  for (const [type, roomName] of DEVICES) {
    const location = roomName ? roomByName.get(roomName) : undefined;
    const rec = await hub.register(type, undefined, location);
    console.log(`  + ${rec.type.padEnd(6)} ${rec.deviceId}  ${roomName ? "-> " + roomName : "(unassigned)"}`);
    n += 1;
  }

  await hub.close();
  console.log(`\nSeeded ${ROOMS.length} room(s) and ${n} device(s), all OFFLINE.`);
  console.log("Next: `pnpm run wake` to bring them online (one console window each).");
  process.exit(0);
}

main().catch((err) => {
  console.error("seed failed:", err instanceof Error ? err.message : err);
  console.error("Is the hub running? Start it with `pnpm run hub`.");
  process.exit(1);
});
