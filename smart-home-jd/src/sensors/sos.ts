import { BaseSensor } from "./base-sensor.js";
import type { SensorCommand, SosState } from "../shared/types.js";

/**
 * SOS panic button (wearable pendant): the resident presses it to call for help.
 * Normally idle; a press raises an immediate critical alarm at the hub. In auto
 * mode it presses only rarely (deliberate event), then releases again.
 */
export class SosSensor extends BaseSensor {
  private pressed = false;

  protected simulateStep(): void {
    if (this.pressed) {
      // Auto-release shortly after a press.
      if (Math.random() < 0.6) this.pressed = false;
    } else if (Math.random() < 0.03) {
      this.pressed = true; // rare deliberate press
    }
  }

  protected currentState(): SosState {
    return { kind: "sos", pressed: this.pressed };
  }

  protected applyCommand(cmd: SensorCommand): void {
    if (typeof cmd.pressed === "boolean") this.pressed = cmd.pressed;
  }

  protected flipPrimary(): void {
    this.pressed = !this.pressed;
  }

  describeState(): string {
    return this.pressed ? "PRESSED — help requested" : "idle";
  }
}
