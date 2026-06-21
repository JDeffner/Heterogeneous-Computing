/** Maps a device record onto the concrete sensor implementation. */
import type { BaseSensor } from "./base-sensor.js";
import type { DeviceRecord, SensorMode } from "../shared/types.js";
import { DoorSensor } from "./door.js";
import { MotionSensor } from "./motion.js";
import { BedSensor } from "./bed.js";
import { StoveSensor } from "./stove.js";
import { SosSensor } from "./sos.js";

export function makeSensor(record: DeviceRecord, mode: SensorMode = "auto"): BaseSensor {
  switch (record.type) {
    case "door":
      return new DoorSensor(record, mode);
    case "motion":
      return new MotionSensor(record, mode);
    case "bed":
      return new BedSensor(record, mode);
    case "stove":
      return new StoveSensor(record, mode);
    case "sos":
      return new SosSensor(record, mode);
  }
}
