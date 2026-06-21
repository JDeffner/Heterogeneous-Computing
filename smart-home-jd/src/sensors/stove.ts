import { BaseSensor } from "./base-sensor.js";
import type { SensorCommand, StoveState } from "../shared/types.js";

/**
 * Stove guard: reports on/off and an estimated cooktop temperature. Models
 * cooking including the risk of the stove being left switched on.
 */
export class StoveSensor extends BaseSensor {
  private isOn = false;
  private temperatureC = 20;

  protected simulateStep(): void {
    if (this.isOn) {
      if (Math.random() < 0.25) this.isOn = false; // switched off
    } else if (Math.random() < 0.25) {
      this.isOn = true; // switched on
    }
    this.updateTemperature();
  }

  /** Temperature model: rises while cooking, cools down slowly otherwise. */
  private updateTemperature(): void {
    if (this.isOn) {
      this.temperatureC = Math.min(230, this.temperatureC + 18 + Math.random() * 10);
    } else {
      this.temperatureC = Math.max(20, this.temperatureC - 12);
    }
  }

  protected currentState(): StoveState {
    return { kind: "stove", on: this.isOn, temperatureC: Math.round(this.temperatureC) };
  }

  protected applyCommand(cmd: SensorCommand): void {
    if (typeof cmd.on === "boolean") this.isOn = cmd.on;
    if (typeof cmd.temperatureC === "number") this.temperatureC = cmd.temperatureC;
  }

  protected flipPrimary(): void {
    this.isOn = !this.isOn;
    this.updateTemperature();
  }

  describeState(): string {
    return (this.isOn ? "ON" : "off") + ` · ${Math.round(this.temperatureC)}°C`;
  }
}
