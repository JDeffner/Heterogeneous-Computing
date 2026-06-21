/**
 * Hub rule thresholds. Deliberately small (seconds) so the demo reacts quickly.
 * In a real installation these would be minutes. All values can be overridden
 * via environment variables.
 */
function num(name: string, def: number): number {
  const v = Number(process.env[name]);
  return Number.isFinite(v) && v > 0 ? v : def;
}
function bool(name: string, def: boolean): boolean {
  const v = process.env[name];
  if (v === undefined) return def;
  return v === "1" || v.toLowerCase() === "true";
}

export const config = {
  tickMs: num("EVAL_TICK_MS", 2000),

  stoveOnSeconds: num("STOVE_ON_SECONDS", 12),
  stoveNoMotionSeconds: num("STOVE_NO_MOTION_SECONDS", 12),

  doorOpenSeconds: num("DOOR_OPEN_SECONDS", 10),
  doorNoMotionSeconds: num("DOOR_NO_MOTION_SECONDS", 12),

  bedAbsenceSeconds: num("BED_ABSENCE_SECONDS", 15),
  nightStartHour: num("NIGHT_START", 22),
  nightEndHour: num("NIGHT_END", 7),

  /**
   * Demo override: forces "night" so the bed rule is demonstrable any time.
   * Defaults to false so day/night follows the real clock; the web UI's
   * "Simulate night" button flips it at runtime.
   */
  forceNight: bool("FORCE_NIGHT", false),

  dataDir: process.env.DATA_DIR ?? "",
};
