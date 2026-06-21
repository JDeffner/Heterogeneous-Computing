/**
 * Entry point for `pnpm run sensor`.
 *
 * With no arguments it opens the INTERACTIVE console (register a new device or
 * take over an offline one). Arguments allow non-interactive use:
 *
 *   pnpm run sensor                         # interactive picker
 *   pnpm run sensor -- --id bed-abc         # take over an existing device
 *   pnpm run sensor -- --type stove         # register + run a new stove (auto id)
 *   pnpm run sensor -- --type door --room Kitchen
 *   pnpm run sensor -- --type bed --id bed-1 # new device with a chosen id
 */
import { runConsole } from "./console.js";
import { SENSOR_TYPE_LABELS, type SensorType } from "../shared/types.js";

function parseArgs(argv: string[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2);
      const val = argv[i + 1] && !argv[i + 1].startsWith("--") ? argv[++i] : "true";
      out[key] = val;
    }
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
const type = args.type as SensorType | undefined;
if (type && !(type in SENSOR_TYPE_LABELS)) {
  console.error(`Invalid --type '${type}'. Allowed: ${Object.keys(SENSOR_TYPE_LABELS).join(" | ")}`);
  process.exit(1);
}

runConsole({ id: args.id, type, room: args.room }).catch((err) => {
  console.error("Sensor console failed:", err instanceof Error ? err.message : err);
  process.exit(1);
});
