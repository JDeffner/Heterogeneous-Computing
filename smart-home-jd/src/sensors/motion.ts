import { BaseSensor } from "./base-sensor.js";
import type { MotionState, SensorCommand } from "../shared/types.js";

/**
 * Motion sensor (PIR): reports motion in the room plus ambient light. Motion is
 * the exception (people do not move continuously), so natural quiet periods
 * arise that the hub's rules interpret.
 */
export class MotionSensor extends BaseSensor {
  private motion = false;
  private lux = 120;

  protected simulateStep(): void {
    this.motion = Math.random() < 0.22;
    // Ambient light drifts a little; brighter when there is activity.
    const target = this.motion ? 240 : 90;
    this.lux = Math.round(this.lux + (target - this.lux) * 0.3 + (Math.random() - 0.5) * 20);
    this.lux = Math.max(0, this.lux);
  }

  protected currentState(): MotionState {
    return { kind: "motion", motion: this.motion, lux: this.lux };
  }

  protected applyCommand(cmd: SensorCommand): void {
    if (typeof cmd.motion === "boolean") this.motion = cmd.motion;
  }

  protected flipPrimary(): void {
    this.motion = !this.motion;
  }

  describeState(): string {
    return (this.motion ? "MOTION" : "still") + ` · ${this.lux} lux`;
  }
}
