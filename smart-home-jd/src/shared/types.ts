/**
 * Shared data types: the "interface contracts" between the loosely coupled
 * components. All communication happens over MQTT with JSON payloads, so the
 * components can be developed, replaced and restarted independently.
 *
 * Architecture note (centralized model):
 *   - SENSORS own their dynamic data: presence (online/offline), telemetry
 *     (state + battery + signal). Each sensor is an independent process.
 *   - The HUB owns the persisted device REGISTRY (the static descriptor of every
 *     device it has ever onboarded) plus rooms, rules and alarms.
 * A device therefore has two halves: a registry record kept by the hub, and a
 * live presence/telemetry stream produced by whatever process currently "is"
 * that device (an interactive console, the wake launcher, or the smoke test).
 */

export type SensorType = "door" | "motion" | "bed" | "stove" | "sos";

/** Whether a sensor evolves on its own (auto) or is driven manually. */
export type SensorMode = "auto" | "manual";

/** Human-readable label per sensor type (single source of truth). */
export const SENSOR_TYPE_LABELS: Record<SensorType, string> = {
  door: "Door / contact sensor",
  motion: "Motion sensor",
  bed: "Bed occupancy sensor",
  stove: "Stove guard",
  sos: "SOS panic button",
};

export const SENSOR_TYPES = Object.keys(SENSOR_TYPE_LABELS) as SensorType[];

/** Radio / network transport a device uses to reach the hub. */
export type Protocol = "Zigbee" | "Z-Wave" | "Wi-Fi" | "Thread" | "BLE";

/** Where a device draws its power from (affects the battery model). */
export type PowerSource = "battery" | "mains";

/**
 * Physical location of a device. The hub stores it in the registry; it is
 * assigned by "scanning" a room's QR code during installation.
 */
export interface Location {
  building: string; // e.g. "House A"
  floor: string; // e.g. "Ground floor"
  room: string; // e.g. "Kitchen"
  /** Id of the registered room this location came from (the room's "QR code"). */
  roomId?: string;
}

/**
 * A room that exists in the building, identified by a stable id that is also the
 * content of the room's QR code. Rooms are created and persisted by the hub and
 * published as retained messages so the UIs can offer them for assignment.
 */
export interface Room {
  roomId: string;
  building: string;
  floor: string;
  room: string;
  createdAt: string; // ISO 8601
}

/**
 * The hub's persisted descriptor for a device. Published RETAINED on
 * `smarthome/registry/<deviceId>` so every UI and every sensor console sees the
 * full fleet immediately - including devices that are currently OFFLINE (no
 * process is running them). This retained registry is what makes it possible to
 * "take over the role" of a previously installed device.
 */
export interface DeviceRecord {
  deviceId: string;
  type: SensorType;
  /** Display name, e.g. "Door / contact sensor - Kitchen". */
  name: string;
  /** Physical location; absent until a room's QR code has been scanned. */
  location?: Location;

  /** Factory serial number (printed on the hardware; stable across re-pairing). */
  serialNumber: string;

  // --- Faithful device identity (would be printed on the real hardware) ------
  manufacturer: string;
  model: string;
  protocol: Protocol;
  powerSource: PowerSource;
  firmware: string;
  /** Which measurements/commands the device serves. */
  capabilities: string[];

  // --- Lifecycle / liveness --------------------------------------------------
  status: "online" | "offline";
  registeredAt: string; // first onboarded (stable)
  lastSeen?: string; // last presence/telemetry timestamp
}

/**
 * Onboarding request: a sensor console asks the hub to register a NEW device.
 * The hub allocates/validates the id, stores the record and replies on the
 * matching ack topic.
 */
export interface RegisterRequest {
  /** Correlation id so the console can await the matching ack. */
  reqId: string;
  type: SensorType;
  /** Optional desired id; the hub assigns one if omitted or already taken. */
  desiredId?: string;
  /** Optional initial location (room) chosen during installation. */
  location?: Location;
}

/** Hub's reply to a {@link RegisterRequest}. */
export interface RegisterAck {
  reqId: string;
  ok: boolean;
  /** The device record the console should now embody (present when ok). */
  record?: DeviceRecord;
  error?: string;
}

/**
 * Pairing advertisement (the faithful onboarding path). A factory-fresh,
 * UNPROVISIONED device enters "pairing/inclusion mode" and advertises only its
 * hardware identity - no operational id and no room yet. Published RETAINED on
 * `smarthome/pairing/<serial>` so the operator app's "waiting to pair" list sees
 * it; the device sets a last-will that clears this ad if it gives up.
 */
export interface PairingAd {
  serial: string;
  type: SensorType;
  manufacturer: string;
  model: string;
  protocol: Protocol;
  powerSource: PowerSource;
  firmware: string;
  /** Short setup PIN printed on the device (cosmetic confirmation, like Matter). */
  pairingPin: string;
  advertisedAt: string; // ISO 8601
}

/** App/console -> hub: commission a waiting device (assign id/name/room). */
export interface CommissionRequest {
  serial: string;
  name?: string;
  roomId?: string | null;
}

/** Hub -> device: the device has been commissioned; here is its operational id. */
export interface CommissionedMessage {
  serial: string;
  deviceId: string;
  name: string;
  location?: Location;
}

/** Hub liveness/config snapshot for the UI (retained). */
export interface HubStatus {
  /** Night window (hours) used by the night-based rules. */
  nightStartHour: number;
  nightEndHour: number;
  /** Demo override: when true the hub always treats the time as night. */
  forceNight: boolean;
}

/** UI -> hub control: tweak hub behaviour at runtime (e.g. the night toggle). */
export interface HubControl {
  forceNight?: boolean;
}

/** A single telemetry sample produced by a live sensor. */
export interface Telemetry {
  deviceId: string;
  type: SensorType;
  /** Sensor-specific state, see below. */
  state: DoorState | MotionState | BedState | StoveState | SosState;
  /** Current mode of the sensor (auto/manual). */
  mode: SensorMode;
  /** Battery charge in percent (mains devices report 100). */
  battery: number;
  /** Received signal strength to the hub in dBm (negative; closer to 0 = better). */
  rssi: number;
  ts: string; // ISO 8601
}

export interface DoorState {
  kind: "door";
  open: boolean;
}
export interface MotionState {
  kind: "motion";
  motion: boolean;
  /** Illuminance in lux (many PIR sensors also report ambient light). */
  lux: number;
}
export interface BedState {
  kind: "bed";
  /** true = a person is lying in the bed. */
  occupied: boolean;
  /** Estimated heart rate in bpm while occupied (0 when empty). */
  heartRate: number;
}
export interface StoveState {
  kind: "stove";
  on: boolean;
  /** Estimated cooktop temperature in degrees Celsius. */
  temperatureC: number;
}
export interface SosState {
  kind: "sos";
  /** true = the wearer has pressed the panic button. */
  pressed: boolean;
}

/**
 * Command sent to a single live device (from the console, a UI, or the hub).
 * Published on the device's command topic. Unset fields are ignored.
 */
export interface SensorCommand {
  /** Switch between automatic simulation and manual control. */
  mode?: SensorMode;
  /**
   * Simulate the device's network link. false = the device "drops off" the
   * network (marks itself offline and stops sending telemetry); true = back.
   */
  online?: boolean;
  // Type-specific value overrides (manual control from the console / UI):
  open?: boolean; // door
  motion?: boolean; // motion
  occupied?: boolean; // bed
  on?: boolean; // stove
  pressed?: boolean; // sos
  temperatureC?: number; // stove
}

/** Control message for the room registry (create/delete a room). */
export type RoomControl =
  | { action: "create"; building: string; floor: string; room: string }
  | { action: "delete"; roomId: string };

/**
 * Control message for the device registry, handled by the hub.
 *  - assign: place a device in a room (or clear with roomId null) = "QR scan".
 *  - remove: permanently delete a device from the registry.
 */
export type DeviceControl =
  | { action: "assign"; deviceId: string; roomId: string | null }
  | { action: "remove"; deviceId: string };

/** Rule keys the hub's evaluation engine can report. */
export type RuleKey =
  | "stove_on_no_motion"
  | "bed_left_at_night_no_return"
  | "door_open_no_motion"
  | "sos_pressed"
  | "device_offline";

/** Situation detected by the hub's rule engine. */
export interface SituationEvent {
  eventId: string;
  rule: RuleKey;
  severity: "info" | "warning" | "critical";
  message: string;
  location: Location;
  deviceIds: string[];
  /** When true the situation has ended, so the matching alarm auto-resolves. */
  cleared?: boolean;
  detectedAt: string; // ISO 8601
}

/** Alarm/warning object created by the hub's alarm engine. */
export interface Alarm {
  alarmId: string;
  rule: RuleKey;
  severity: SituationEvent["severity"];
  message: string;
  location: Location;
  deviceIds: string[];
  status: "active" | "acknowledged" | "resolved";
  raisedAt: string; // ISO 8601
  updatedAt: string; // ISO 8601
}

/** Control message for alarms (e.g. "resolve" from the operator console). */
export interface AlarmControl {
  alarmId: string;
  action: "resolve";
}
