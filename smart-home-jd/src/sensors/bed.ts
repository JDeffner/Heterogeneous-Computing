import { BaseSensor } from "./base-sensor.js";
import type { BedState, SensorCommand } from "../shared/types.js";

/**
 * Bed occupancy sensor (pressure mat): reports whether the person is in bed and,
 * while occupied, an estimated heart rate. Models occasionally leaving the bed
 * with sometimes longer absences (e.g. a nightly bathroom trip that takes long).
 */
export class BedSensor extends BaseSensor {
  private occupied = true;
  private heartRate = 62;

  protected simulateStep(): void {
    if (this.occupied) {
      if (Math.random() < 0.15) this.occupied = false; // left the bed
    } else if (Math.random() < 0.4) {
      this.occupied = true; // back in bed
    }
    this.heartRate = this.occupied
      ? Math.round(58 + Math.random() * 12) // resting heart rate
      : 0;
  }

  protected currentState(): BedState {
    return { kind: "bed", occupied: this.occupied, heartRate: this.heartRate };
  }

  protected applyCommand(cmd: SensorCommand): void {
    if (typeof cmd.occupied === "boolean") {
      this.occupied = cmd.occupied;
      this.heartRate = this.occupied ? 62 : 0;
    }
  }

  protected flipPrimary(): void {
    this.occupied = !this.occupied;
    this.heartRate = this.occupied ? 62 : 0;
  }

  describeState(): string {
    return this.occupied ? `OCCUPIED · ${this.heartRate} bpm` : "empty";
  }
}
