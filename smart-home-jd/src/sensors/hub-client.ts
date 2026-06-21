/**
 * Control-plane helper used by the sensor console and the wake launcher to talk
 * to the hub before a device actually "comes alive":
 *   - snapshot the retained registry + rooms (so the user can pick a device), and
 *   - register a brand-new device and await the hub's ack.
 *
 * This is a separate, short-lived MQTT connection from the one the BaseSensor
 * opens for the device itself.
 */
import type { MqttClient } from "mqtt";
import { connect, publishJson, newId } from "../shared/mqtt.js";
import { subscriptions, topics, deviceIdFromRegistry, roomIdFromTopic } from "../shared/topics.js";
import type {
  CommissionRequest,
  CommissionedMessage,
  DeviceRecord,
  Location,
  PairingAd,
  RegisterAck,
  RegisterRequest,
  Room,
  SensorType,
} from "../shared/types.js";

export interface HubSnapshot {
  records: Map<string, DeviceRecord>;
  rooms: Map<string, Room>;
}

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

export class HubClient {
  private constructor(private readonly client: MqttClient) {}

  static async open(clientId?: string): Promise<HubClient> {
    const client = await connect({
      clientId: clientId ?? `console-${Date.now().toString(36)}`,
    });
    return new HubClient(client);
  }

  /** Collect the retained registry + rooms over a short listening window. */
  async snapshot(windowMs = 900): Promise<HubSnapshot> {
    const records = new Map<string, DeviceRecord>();
    const rooms = new Map<string, Room>();
    const onMessage = (topic: string, buf: Buffer): void => {
      const raw = buf.toString();
      const devId = deviceIdFromRegistry(topic);
      if (devId) {
        if (raw.length === 0) records.delete(devId);
        else records.set(devId, JSON.parse(raw) as DeviceRecord);
        return;
      }
      const roomId = roomIdFromTopic(topic);
      if (roomId) {
        if (raw.length === 0) rooms.delete(roomId);
        else rooms.set(roomId, JSON.parse(raw) as Room);
      }
    };
    this.client.on("message", onMessage);
    this.client.subscribe([subscriptions.allRegistry, subscriptions.allRooms]);
    await delay(windowMs);
    this.client.removeListener("message", onMessage);
    return { records, rooms };
  }

  /** Register a new device with the hub and wait for the record it minted. */
  async register(
    type: SensorType,
    desiredId?: string,
    location?: Location,
    timeoutMs = 5000,
  ): Promise<DeviceRecord> {
    const reqId = newId("req");
    const ackTopic = topics.registerAck(reqId);
    return new Promise<DeviceRecord>((resolve, reject) => {
      const timer = setTimeout(() => {
        cleanup();
        reject(new Error("hub did not answer the registration (is `pnpm run hub` up?)"));
      }, timeoutMs);
      const onMessage = (topic: string, buf: Buffer): void => {
        if (topic !== ackTopic) return;
        const ack = JSON.parse(buf.toString()) as RegisterAck;
        if (ack.reqId !== reqId) return;
        cleanup();
        if (ack.ok && ack.record) resolve(ack.record);
        else reject(new Error(ack.error ?? "registration rejected"));
      };
      const cleanup = (): void => {
        clearTimeout(timer);
        this.client.removeListener("message", onMessage);
        this.client.unsubscribe(ackTopic);
      };
      this.client.on("message", onMessage);
      this.client.subscribe(ackTopic, () => {
        const req: RegisterRequest = { reqId, type, desiredId, location };
        publishJson(this.client, topics.register(), req, { qos: 1 });
      });
    });
  }

  /** Ask the hub to commission a device that is advertising in pairing mode. */
  commission(serial: string, name?: string, roomId?: string | null): void {
    const req: CommissionRequest = { serial, name, roomId };
    publishJson(this.client, topics.commissionControl(), req, { qos: 1 });
  }

  /** Ask the hub to create a room (used by the seed script). */
  createRoom(building: string, floor: string, room: string): void {
    publishJson(
      this.client,
      topics.roomControl(),
      { action: "create", building, floor, room },
      { qos: 1 },
    );
  }

  async close(): Promise<void> {
    await new Promise<void>((resolve) => this.client.end(false, undefined, () => resolve()));
  }
}

/** A live pairing advertisement the console is broadcasting. */
export interface PairingSession {
  /** Resolves when the hub commissions this device (app or local). */
  waitForCommission(): Promise<CommissionedMessage>;
  /** Clear the retained ad and disconnect (user gave up before commissioning). */
  cancel(): Promise<void>;
  /** Disconnect after a successful commission (the hub already cleared the ad). */
  close(): Promise<void>;
}

/**
 * Enter pairing mode: publish a retained pairing advertisement on a dedicated
 * connection whose LAST-WILL clears that ad, so an abandoned/crashed pairing
 * disappears from the operator app automatically.
 */
export async function advertisePairing(ad: PairingAd): Promise<PairingSession> {
  const adTopic = topics.pairing(ad.serial);
  const ackTopic = topics.pairingCommissioned(ad.serial);
  const client = await connect({
    clientId: `pairing-${ad.serial}`,
    will: { topic: adTopic, payload: "", retain: true },
  });
  client.subscribe(ackTopic);
  publishJson(client, adTopic, ad, { retain: true });

  const end = () =>
    new Promise<void>((resolve) => client.end(false, undefined, () => resolve()));

  return {
    waitForCommission: () =>
      new Promise<CommissionedMessage>((resolve) => {
        client.on("message", (topic, buf) => {
          if (topic !== ackTopic || buf.length === 0) return;
          try {
            resolve(JSON.parse(buf.toString()) as CommissionedMessage);
          } catch {
            /* ignore malformed ack */
          }
        });
      }),
    cancel: async () => {
      await new Promise<void>((resolve) =>
        client.publish(adTopic, "", { retain: true, qos: 1 }, () => resolve()),
      );
      await end();
    },
    close: end,
  };
}
