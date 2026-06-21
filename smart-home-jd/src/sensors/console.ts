/**
 * Interactive sensor console.
 *
 * Running `pnpm run sensor` opens this console. You either REGISTER a brand-new
 * device or TAKE OVER the role of a previously installed device that is currently
 * OFFLINE. From then on this process *is* that device: it shows a live dashboard
 * (identity, battery, signal, state, telemetry/command log) and you can drive it
 * by keyboard. Closing the window (or Ctrl+C) takes the device offline.
 *
 * Non-interactive shortcuts (used by `pnpm run wake` and the smoke test):
 *   --id <deviceId>            take over an existing device by id
 *   --type <t> [--room <name>] register and embody a new device
 *   --id <id> --type <t>       register a new device with a specific id
 */
import readline from "node:readline";
import { HubClient, advertisePairing, type HubSnapshot } from "./hub-client.js";
import { makeSensor } from "./factory.js";
import type { BaseSensor, SensorSnapshot } from "./base-sensor.js";
import { DEVICE_CATALOG, makeSerial } from "./device-catalog.js";
import {
  SENSOR_TYPE_LABELS,
  SENSOR_TYPES,
  type CommissionedMessage,
  type DeviceRecord,
  type Location,
  type PairingAd,
  type Room,
  type SensorType,
} from "../shared/types.js";

// ----- Tiny ANSI helpers ---------------------------------------------------

const C = {
  reset: "\x1b[0m",
  dim: "\x1b[2m",
  bold: "\x1b[1m",
  green: "\x1b[32m",
  red: "\x1b[31m",
  yellow: "\x1b[33m",
  cyan: "\x1b[36m",
  gray: "\x1b[90m",
};
const clearScreen = () => process.stdout.write("\x1b[2J\x1b[3J\x1b[H");
const time = () => new Date().toLocaleTimeString("en-GB");
const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
const rule = `  ${C.gray}${"-".repeat(58)}${C.reset}`;
const heavy = `${C.cyan}${"=".repeat(62)}${C.reset}`;

function signalBars(rssi: number): string {
  // -35 (great) .. -95 (poor) -> 0..4 bars
  const level = Math.max(0, Math.min(4, Math.round(((rssi + 95) / 60) * 4)));
  const glyphs = ["▁", "▂", "▄", "▆", "█"];
  return glyphs.slice(0, Math.max(1, level)).join("") + C.gray + glyphs.slice(Math.max(1, level)).join("") + C.reset;
}

// ----- Interactive selection ----------------------------------------------

function ask(rl: readline.Interface, q: string): Promise<string> {
  return new Promise((res) => rl.question(q, (a) => res(a.trim())));
}

function locationFromRoom(room: Room): Location {
  return { building: room.building, floor: room.floor, room: room.room, roomId: room.roomId };
}

/** Free-form location for CLI --room values that don't match a registered room. */
function looseLocation(name: string, snap: HubSnapshot): Location {
  for (const room of snap.rooms.values()) {
    if (room.room.toLowerCase() === name.toLowerCase()) return locationFromRoom(room);
  }
  return { building: "House A", floor: "Ground floor", room: name };
}

function printFleet(snap: HubSnapshot): void {
  const records = [...snap.records.values()].sort((a, b) => a.deviceId.localeCompare(b.deviceId));
  console.log(`\n${C.bold}Known devices in the hub registry:${C.reset}`);
  if (records.length === 0) {
    console.log(`  ${C.dim}(none yet — register the first one)${C.reset}`);
    return;
  }
  for (const r of records) {
    const dot = r.status === "online" ? `${C.green}● online ${C.reset}` : `${C.gray}○ offline${C.reset}`;
    const where = r.location ? r.location.room : "unassigned";
    console.log(`  ${dot} ${r.deviceId.padEnd(20)} ${SENSOR_TYPE_LABELS[r.type].padEnd(24)} ${C.dim}${where}${C.reset}`);
  }
}

async function interactiveSelect(hub: HubClient, snap: HubSnapshot): Promise<DeviceRecord> {
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  try {
    clearScreen();
    console.log(heavy);
    console.log(`  ${C.bold}SMART-HOME DEVICE CONSOLE${C.reset}  ${C.dim}— take the role of a device${C.reset}`);
    console.log(heavy);
    printFleet(snap);

    const offline = [...snap.records.values()]
      .filter((r) => r.status === "offline")
      .sort((a, b) => a.deviceId.localeCompare(b.deviceId));

    console.log(`\n${C.bold}What do you want to do?${C.reset}`);
    offline.forEach((r, i) => {
      const where = r.location ? r.location.room : "unassigned";
      console.log(`  ${C.cyan}${i + 1}${C.reset}) take over  ${r.deviceId}  ${C.dim}(${SENSOR_TYPE_LABELS[r.type]}, ${where})${C.reset}`);
    });
    console.log(`  ${C.cyan}n${C.reset}) register a NEW device`);
    console.log(`  ${C.cyan}q${C.reset}) quit`);
    if (offline.length === 0) console.log(`  ${C.dim}(online devices can't be taken over — they're already running)${C.reset}`);

    const choice = (await ask(rl, "\n> ")).toLowerCase();
    if (choice === "q" || choice === "") {
      rl.close();
      process.exit(0);
    }
    if (choice === "n") return await registerFlow(rl, hub, snap);

    const idx = Number(choice) - 1;
    if (Number.isInteger(idx) && idx >= 0 && idx < offline.length) return offline[idx];

    console.log(`${C.red}Invalid choice.${C.reset}`);
    rl.close();
    return interactiveSelect(hub, snap); // re-open and retry
  } finally {
    // rl may already be closed in the exit paths above.
  }
}

function makeAd(type: SensorType): PairingAd {
  const cat = DEVICE_CATALOG[type];
  return {
    serial: makeSerial(type),
    type,
    manufacturer: cat.manufacturer,
    model: cat.model,
    protocol: cat.protocol,
    powerSource: cat.powerSource,
    firmware: cat.firmware,
    pairingPin: String(Math.floor(1000 + Math.random() * 9000)),
    advertisedAt: new Date().toISOString(),
  };
}

/** Build the operational record the console will embody after commissioning. */
function recordFromCommissioned(msg: CommissionedMessage, type: SensorType, ad: PairingAd): DeviceRecord {
  const cat = DEVICE_CATALOG[type];
  return {
    deviceId: msg.deviceId,
    type,
    name: msg.name,
    location: msg.location,
    serialNumber: ad.serial,
    manufacturer: cat.manufacturer,
    model: cat.model,
    protocol: cat.protocol,
    powerSource: cat.powerSource,
    firmware: cat.firmware,
    capabilities: cat.capabilities,
    status: "offline",
    registeredAt: new Date().toISOString(),
  };
}

function renderPairingScreen(ad: PairingAd): void {
  clearScreen();
  console.log(heavy);
  console.log(`  ${C.bold}PAIRING MODE${C.reset}  ${C.dim}— this device is unprovisioned${C.reset}`);
  console.log(heavy);
  console.log(`  ${SENSOR_TYPE_LABELS[ad.type]} ${C.dim}(${ad.manufacturer} ${ad.model} · ${ad.protocol})${C.reset}`);
  console.log(rule);
  console.log(`  Serial      ${C.bold}${ad.serial}${C.reset}`);
  console.log(`  Setup PIN   ${C.bold}${ad.pairingPin}${C.reset}  ${C.dim}(printed on the device)${C.reset}`);
  console.log(rule);
  console.log(`  ${C.yellow}Waiting to be added…${C.reset}`);
  console.log(`  Open the operator console (${C.cyan}http://localhost:3000${C.reset}) →`);
  console.log(`  "Devices waiting to pair" → ${C.bold}Add${C.reset}, then choose a room.`);
  console.log(rule);
  console.log(`  ${C.cyan}Ctrl+C${C.reset}  cancel pairing`);
}

/**
 * Register a new device the faithful way: it enters pairing mode and is
 * commissioned either here (pick a room now) or from the operator app.
 */
async function registerFlow(rl: readline.Interface, hub: HubClient, snap: HubSnapshot): Promise<DeviceRecord> {
  console.log(`\n${C.bold}New device — pairing${C.reset}`);
  SENSOR_TYPES.forEach((t, i) => {
    const cat = DEVICE_CATALOG[t];
    console.log(`  ${C.cyan}${i + 1}${C.reset}) ${SENSOR_TYPE_LABELS[t].padEnd(24)} ${C.dim}${cat.manufacturer} ${cat.model} · ${cat.protocol}${C.reset}`);
  });
  const typeIdx = Number(await ask(rl, "Device type > ")) - 1;
  const type: SensorType = SENSOR_TYPES[typeIdx] ?? "door";

  const how = (await ask(rl, "Commission [h]ere now, or wait for the [a]pp? (h/a) > ")).toLowerCase();
  const ad = makeAd(type);

  if (how === "a") {
    rl.close();
    const session = await advertisePairing(ad);
    renderPairingScreen(ad);
    process.once("SIGINT", () => {
      session.cancel().finally(() => process.exit(0));
    });
    const msg = await session.waitForCommission();
    await session.close();
    return recordFromCommissioned(msg, type, ad);
  }

  // Commission here: pick a room now (the console acts as the app).
  const rooms = [...snap.rooms.values()];
  let roomId: string | null = null;
  if (rooms.length > 0) {
    console.log(`\nAssign to a room (its QR code), or skip:`);
    rooms.forEach((r, i) =>
      console.log(`  ${C.cyan}${i + 1}${C.reset}) ${r.room} ${C.dim}(${r.building} / ${r.floor})${C.reset}`),
    );
    console.log(`  ${C.cyan}0${C.reset}) skip — install unassigned`);
    const roomIdx = Number(await ask(rl, "Room > ")) - 1;
    if (roomIdx >= 0 && roomIdx < rooms.length) roomId = rooms[roomIdx].roomId;
  } else {
    console.log(`${C.dim}(no rooms yet — installing unassigned; assign later in the web console)${C.reset}`);
  }
  const name = (await ask(rl, "Custom name (blank = auto) > ")) || undefined;
  rl.close();

  console.log(`${C.dim}pairing and commissioning …${C.reset}`);
  const session = await advertisePairing(ad);
  const commissionedP = session.waitForCommission();
  // Retry: the hub must process the retained pairing ad before it can honour the
  // commission request (the two are published on different connections, so their
  // arrival order at the hub is not guaranteed). Re-sending is safe - once
  // commissioned the hub clears the ad, so later requests are no-ops.
  let done = false;
  void commissionedP.then(() => {
    done = true;
  });
  for (let i = 0; i < 8 && !done; i++) {
    hub.commission(ad.serial, name, roomId);
    await delay(400);
  }
  const msg = await commissionedP;
  await session.close();
  return recordFromCommissioned(msg, type, ad);
}

// ----- Live dashboard ------------------------------------------------------

function renderDashboard(s: SensorSnapshot, log: string[]): void {
  const r = s.record;
  clearScreen();
  const link = s.linkUp ? `${C.green}● ONLINE${C.reset}` : `${C.red}● OFFLINE (link down)${C.reset}`;
  const battery =
    r.powerSource === "mains"
      ? `${C.dim}mains powered${C.reset}`
      : `${batteryColor(s.battery)}${s.battery}%${C.reset}`;
  const loc = r.location
    ? `${r.location.building} / ${r.location.floor} / ${r.location.room}`
    : `${C.yellow}unassigned${C.reset}`;

  console.log(heavy);
  console.log(`  ${C.bold}SMART-HOME DEVICE CONSOLE${C.reset}  ${C.dim}— you are this device${C.reset}`);
  console.log(heavy);
  console.log(`  ${C.bold}${r.name}${C.reset}  ${C.gray}${r.deviceId}${C.reset}`);
  console.log(rule);
  console.log(`  Link        ${link.padEnd(28)}  Mode    ${s.mode}`);
  console.log(`  Location    ${loc}`);
  console.log(`  Identity    ${r.manufacturer} ${r.model} ${C.dim}·${C.reset} ${r.protocol} ${C.dim}·${C.reset} ${r.powerSource}`);
  console.log(`  Firmware    ${r.firmware}    ${C.dim}caps:${C.reset} ${r.capabilities.join(", ")}`);
  console.log(`  Battery     ${battery.padEnd(28)}  Signal  ${s.rssi} dBm  ${signalBars(s.rssi)}`);
  console.log(`  State       ${C.bold}${describe(s)}${C.reset}`);
  console.log(`  Telemetry   ${s.telemetryCount} sent ${C.dim}· last ${s.lastPublish ? new Date(s.lastPublish).toLocaleTimeString("en-GB") : "—"}${C.reset}`);
  console.log(rule);
  console.log(`  ${C.dim}Recent activity:${C.reset}`);
  if (log.length === 0) console.log(`    ${C.dim}—${C.reset}`);
  for (const line of log.slice(-8)) console.log(`    ${C.gray}${line}${C.reset}`);
  console.log(rule);
  console.log(
    `  ${C.cyan}[space]${C.reset} toggle state   ${C.cyan}[m]${C.reset} auto/manual   ${C.cyan}[o]${C.reset} simulate offline`,
  );
  console.log(`  ${C.cyan}[q]${C.reset} / ${C.cyan}Ctrl+C${C.reset}  go offline & quit`);
}

function describe(s: SensorSnapshot): string {
  // Reuse the sensor's own describeState via a thin lookup on the state kind.
  switch (s.state.kind) {
    case "door": return s.state.open ? "OPEN" : "closed";
    case "motion": return `${s.state.motion ? "MOTION" : "still"} · ${s.state.lux} lux`;
    case "bed": return s.state.occupied ? `OCCUPIED · ${s.state.heartRate} bpm` : "empty";
    case "stove": return `${s.state.on ? "ON" : "off"} · ${s.state.temperatureC}°C`;
    case "sos": return s.state.pressed ? "PRESSED — help requested" : "idle";
  }
}

function batteryColor(pct: number): string {
  return pct < 20 ? C.red : pct < 50 ? C.yellow : C.green;
}

async function embody(record: DeviceRecord): Promise<void> {
  const sensor: BaseSensor = makeSensor(record, "auto");
  const log: string[] = [];
  let quitting = false;

  const draw = (): void => renderDashboard(sensor.snapshot(), log);
  sensor.on("log", (line: string) => {
    log.push(`${time()}  ${line}`);
    if (log.length > 40) log.shift();
    draw();
  });
  sensor.on("update", draw);

  const quit = async (): Promise<void> => {
    if (quitting) return;
    quitting = true;
    if (process.stdin.isTTY) process.stdin.setRawMode(false);
    clearScreen();
    console.log(`${C.yellow}Going offline …${C.reset} (${record.deviceId})`);
    await sensor.stop();
    console.log(`${C.dim}Device is now offline. Bye.${C.reset}`);
    process.exit(0);
  };

  await sensor.start();
  setupKeys(sensor, quit);
  process.on("SIGINT", quit);
  process.on("SIGTERM", quit);
  draw();
}

function setupKeys(sensor: BaseSensor, onQuit: () => void): void {
  if (!process.stdin.isTTY) return; // piped/non-interactive: rely on signals + LWT
  readline.emitKeypressEvents(process.stdin);
  process.stdin.setRawMode(true);
  process.stdin.resume();
  process.stdin.on("keypress", (_str, key) => {
    if (!key) return;
    if (key.ctrl && key.name === "c") return onQuit();
    switch (key.name) {
      case "q": return onQuit();
      case "space": sensor.toggle(); break;
      case "m": sensor.setMode(sensor.snapshot().mode === "auto" ? "manual" : "auto"); break;
      case "a": sensor.setMode("auto"); break;
      case "o": sensor.setLinkFromConsole(!sensor.isLinkUp()); break;
    }
  });
}

// ----- Entry ---------------------------------------------------------------

export interface ConsolePreset {
  id?: string;
  type?: SensorType;
  room?: string;
}

export async function runConsole(preset: ConsolePreset): Promise<void> {
  const hub = await HubClient.open();
  const snap = await hub.snapshot();

  let record: DeviceRecord;
  if (preset.id && snap.records.has(preset.id)) {
    // Take over an existing device by id.
    record = snap.records.get(preset.id)!;
    if (record.status === "online") {
      console.error(`${C.yellow}Warning:${C.reset} ${preset.id} reports online — taking over anyway.`);
    }
  } else if (preset.type) {
    // Register a new device (optionally with a specific id / room).
    const location = preset.room ? looseLocation(preset.room, snap) : undefined;
    record = await hub.register(preset.type, preset.id, location);
  } else if (preset.id) {
    await hub.close();
    throw new Error(
      `No device '${preset.id}' in the registry. Pass --type to register a new one, or run \`pnpm run sensor\` to pick interactively.`,
    );
  } else {
    record = await interactiveSelect(hub, snap);
  }

  await hub.close();
  await embody(record);
}
