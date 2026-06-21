/**
 * Device catalog: the "spec sheet" of every supported device type.
 *
 * This is what makes the simulation faithful instead of abstract. Each entry
 * mirrors a plausible real product: a manufacturer/model, the radio/network
 * transport it uses (Zigbee, Z-Wave, Wi-Fi, Thread, BLE), whether it runs on a
 * battery or mains power, its firmware string and its MQTT capabilities.
 *
 * The hub stores these fields in the registry; the sensor console displays them;
 * and they map directly onto the protocol layer discussed in the architecture
 * overview (short-range radios feeding an IP-based hub).
 */
import type {
  PowerSource,
  Protocol,
  SensorType,
} from "../shared/types.js";

export interface CatalogEntry {
  manufacturer: string;
  model: string;
  protocol: Protocol;
  powerSource: PowerSource;
  firmware: string;
  capabilities: string[];
  /** Typical signal strength to the hub in dBm (simulation centres on this). */
  nominalRssi: number;
  /** Suggested reporting interval in ms. */
  reportIntervalMs: number;
}

/** Factory serial, e.g. "SN-DW-S100-3f9ac2" - stable per physical device. */
export function makeSerial(type: SensorType): string {
  const model = DEVICE_CATALOG[type].model.replace(/[^A-Za-z0-9]/g, "");
  const hex = Math.floor(Math.random() * 0xffffff).toString(16).padStart(6, "0");
  return `SN-${model}-${hex}`;
}

export const DEVICE_CATALOG: Record<SensorType, CatalogEntry> = {
  door: {
    manufacturer: "Aqara",
    model: "DW-S100",
    protocol: "Zigbee",
    powerSource: "battery",
    firmware: "sim-2.1.0",
    capabilities: ["door.open"],
    nominalRssi: -62,
    reportIntervalMs: 2500,
  },
  motion: {
    manufacturer: "Philips Hue",
    model: "SML-002",
    protocol: "Zigbee",
    powerSource: "battery",
    firmware: "sim-2.1.0",
    capabilities: ["motion.detected", "motion.lux"],
    nominalRssi: -58,
    reportIntervalMs: 2500,
  },
  bed: {
    manufacturer: "Emfit",
    model: "QS-Care",
    protocol: "Wi-Fi",
    powerSource: "mains",
    firmware: "sim-3.0.4",
    capabilities: ["bed.occupied", "bed.heartRate"],
    nominalRssi: -47,
    reportIntervalMs: 2500,
  },
  stove: {
    manufacturer: "Inirv",
    model: "Guard-Z",
    protocol: "Z-Wave",
    powerSource: "mains",
    firmware: "sim-1.4.2",
    capabilities: ["stove.on", "stove.temp"],
    nominalRssi: -51,
    reportIntervalMs: 2500,
  },
  sos: {
    manufacturer: "CareTech",
    model: "SOS-Pendant",
    protocol: "BLE",
    powerSource: "battery",
    firmware: "sim-1.0.1",
    capabilities: ["sos.pressed"],
    nominalRssi: -70,
    reportIntervalMs: 2500,
  },
};
