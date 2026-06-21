/**
 * Central topic schema (single source of truth) for the CENTRALIZED model:
 * every sensor talks to the hub, and the hub owns the registry, rooms, rules
 * and alarms.
 *
 * Layout:
 *   smarthome/registry/<deviceId>            (retained, HUB)   -> DeviceRecord
 *   smarthome/devices/<deviceId>/presence    (retained, LWT)   -> "online"/"offline"
 *   smarthome/devices/<deviceId>/telemetry                     -> Telemetry
 *   smarthome/devices/<deviceId>/command                       -> SensorCommand
 *   smarthome/hub/register                                     -> RegisterRequest
 *   smarthome/hub/register-ack/<reqId>                         -> RegisterAck
 *   smarthome/events/<eventId>                                 -> SituationEvent
 *   smarthome/alarms/<alarmId>               (retained, HUB)   -> Alarm
 *   smarthome/control/alarms                                   -> AlarmControl
 *   smarthome/control/rooms                                    -> RoomControl
 *   smarthome/control/devices                                  -> DeviceControl
 *   smarthome/rooms/<roomId>                 (retained, HUB)   -> Room
 *
 * The hierarchy and wildcards (+ / #) follow the standard MQTT pub/sub pattern
 * and enable plug-and-play discovery without any shared configuration file.
 */

export const ROOT = "smarthome";

export const topics = {
  registry: (deviceId: string) => `${ROOT}/registry/${deviceId}`,
  presence: (deviceId: string) => `${ROOT}/devices/${deviceId}/presence`,
  telemetry: (deviceId: string) => `${ROOT}/devices/${deviceId}/telemetry`,
  command: (deviceId: string) => `${ROOT}/devices/${deviceId}/command`,
  register: () => `${ROOT}/hub/register`,
  registerAck: (reqId: string) => `${ROOT}/hub/register-ack/${reqId}`,
  pairing: (serial: string) => `${ROOT}/pairing/${serial}`,
  pairingCommissioned: (serial: string) => `${ROOT}/pairing/${serial}/commissioned`,
  commissionControl: () => `${ROOT}/control/commission`,
  event: (eventId: string) => `${ROOT}/events/${eventId}`,
  alarm: (alarmId: string) => `${ROOT}/alarms/${alarmId}`,
  alarmControl: () => `${ROOT}/control/alarms`,
  room: (roomId: string) => `${ROOT}/rooms/${roomId}`,
  roomControl: () => `${ROOT}/control/rooms`,
  deviceControl: () => `${ROOT}/control/devices`,
  hubStatus: () => `${ROOT}/hub/status`,
  hubControl: () => `${ROOT}/control/hub`,
};

/** Subscriptions for discovery and data processing. */
export const subscriptions = {
  allRegistry: `${ROOT}/registry/+`,
  allPresence: `${ROOT}/devices/+/presence`,
  allTelemetry: `${ROOT}/devices/+/telemetry`,
  register: `${ROOT}/hub/register`,
  registerAckAll: `${ROOT}/hub/register-ack/+`,
  allPairing: `${ROOT}/pairing/+`, // only the ads, not the .../commissioned replies
  commissionControl: `${ROOT}/control/commission`,
  allEvents: `${ROOT}/events/+`,
  allAlarms: `${ROOT}/alarms/+`,
  alarmControl: `${ROOT}/control/alarms`,
  allRooms: `${ROOT}/rooms/+`,
  roomControl: `${ROOT}/control/rooms`,
  deviceControl: `${ROOT}/control/devices`,
  hubStatus: `${ROOT}/hub/status`,
  hubControl: `${ROOT}/control/hub`,
};

/** Extracts the deviceId from a `smarthome/registry/<id>` topic. */
export function deviceIdFromRegistry(topic: string): string | null {
  const m = topic.match(/^smarthome\/registry\/([^/]+)$/);
  return m ? m[1] : null;
}

/** Extracts the deviceId from a `smarthome/devices/<id>/...` topic. */
export function deviceIdFromTopic(topic: string): string | null {
  const m = topic.match(/^smarthome\/devices\/([^/]+)\//);
  return m ? m[1] : null;
}

/** Extracts the serial from a `smarthome/pairing/<serial>` advertisement topic. */
export function serialFromPairing(topic: string): string | null {
  const m = topic.match(/^smarthome\/pairing\/([^/]+)$/);
  return m ? m[1] : null;
}

/** Extracts the roomId from a `smarthome/rooms/<roomId>` topic. */
export function roomIdFromTopic(topic: string): string | null {
  const m = topic.match(/^smarthome\/rooms\/([^/]+)$/);
  return m ? m[1] : null;
}
