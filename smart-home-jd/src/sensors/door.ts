import { BaseSensor } from "./base-sensor.js";
import type { DoorState, SensorCommand } from "../shared/types.js";

/**
 * Door / contact sensor: reports open/closed. Models opening with occasional
 * "left open" (e.g. after leaving the apartment).
 */
export class DoorSensor extends BaseSensor {
  private open = false;

  protected simulateStep(): void {
    if (this.open) {
      if (Math.random() < 0.5) this.open = false; // close again
    } else if (Math.random() < 0.3) {
      this.open = true;
    }
  }

  protected currentState(): DoorState {
    return { kind: "door", open: this.open };
  }

  protected applyCommand(cmd: SensorCommand): void {
    if (typeof cmd.open === "boolean") this.open = cmd.open;
  }

  protected flipPrimary(): void {
    this.open = !this.open;
  }

  describeState(): string {
    return this.open ? "OPEN" : "closed";
  }
}
