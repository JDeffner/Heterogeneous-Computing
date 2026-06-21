/**
 * `pnpm run wake` — bring every OFFLINE device back online, each in its own
 * console window.
 *
 * It asks the hub registry which devices are currently offline, then opens one
 * new terminal window per device, each running the interactive console bound to
 * that device id (`pnpm run sensor -- --id <id>`). Close a window and that one
 * device goes offline again; run `pnpm run wake` to bring the stragglers back.
 *
 * Flags:
 *   --dry      list what would be launched, but don't open any windows
 */
import { spawn } from "node:child_process";
import { HubClient } from "../src/sensors/hub-client.js";
import { SENSOR_TYPE_LABELS } from "../src/shared/types.js";

const dry = process.argv.includes("--dry");

function launchWindow(deviceId: string): void {
  const inner = `pnpm run sensor -- --id ${deviceId}`;
  if (process.platform === "win32") {
    // cmd parses the whole string; `start "title" cmd /k "<cmd>"` opens a window
    // that stays open. windowsVerbatimArguments stops Node re-quoting it.
    const command = `start "device ${deviceId}" cmd /k "${inner}"`;
    spawn("cmd.exe", ["/c", command], {
      detached: true,
      stdio: "ignore",
      cwd: process.cwd(),
      windowsVerbatimArguments: true,
    }).unref();
  } else if (process.platform === "darwin") {
    const script = `tell app "Terminal" to do script "cd ${process.cwd()} && ${inner}"`;
    spawn("osascript", ["-e", script], { detached: true, stdio: "ignore" }).unref();
  } else {
    // Best-effort on Linux; falls back to a hint if no terminal is found.
    const term = process.env.TERMINAL || "x-terminal-emulator";
    spawn(term, ["-e", `bash -lc '${inner}; exec bash'`], {
      detached: true,
      stdio: "ignore",
    }).unref();
  }
}

async function main(): Promise<void> {
  const hub = await HubClient.open(`wake-${Date.now().toString(36)}`);
  const { records } = await hub.snapshot();
  await hub.close();

  const offline = [...records.values()].filter((r) => r.status === "offline");
  if (records.size === 0) {
    console.log("Registry is empty. Register a device first: `pnpm run sensor`.");
    process.exit(0);
  }
  if (offline.length === 0) {
    console.log(`All ${records.size} registered device(s) are already online. Nothing to wake.`);
    process.exit(0);
  }

  console.log(`Waking ${offline.length} offline device(s):`);
  for (const r of offline) {
    const where = r.location ? r.location.room : "unassigned";
    console.log(`  • ${r.deviceId}  (${SENSOR_TYPE_LABELS[r.type]}, ${where})`);
    if (!dry) launchWindow(r.deviceId);
  }

  if (dry) {
    console.log("\n(dry run — no windows opened)");
  } else if (process.platform !== "win32" && process.platform !== "darwin") {
    console.log(
      "\nIf no windows opened, run these manually (set $TERMINAL or open tabs yourself):",
    );
    for (const r of offline) console.log(`  pnpm run sensor -- --id ${r.deviceId}`);
  } else {
    console.log("\nOpened one console window per device. Close a window to take that device offline.");
  }

  // Give detached children a moment to spin up before this process exits.
  setTimeout(() => process.exit(0), 500);
}

main().catch((err) => {
  console.error("wake failed:", err instanceof Error ? err.message : err);
  process.exit(1);
});
