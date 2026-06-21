/**
 * Thin helper layer over mqtt.js: establish a connection, publish JSON, receive
 * JSON messages in a typed way. Keeps the services small and consistent.
 */
import mqtt, { type IClientOptions, type MqttClient } from "mqtt";

/** Broker URL is read at runtime (not fixed at import time). */
function brokerUrl(): string {
  return process.env.MQTT_URL ?? "mqtt://localhost:1883";
}

export interface ConnectOptions {
  clientId: string;
  /** Last-will: sent by the broker when the connection drops unexpectedly. */
  will?: { topic: string; payload: string; retain?: boolean };
}

export function connect(opts: ConnectOptions): Promise<MqttClient> {
  const options: IClientOptions = {
    clientId: opts.clientId,
    clean: true,
    reconnectPeriod: 2000, // automatic reconnect -> robustness
    connectTimeout: 10_000,
  };
  if (opts.will) {
    options.will = {
      topic: opts.will.topic,
      payload: Buffer.from(opts.will.payload),
      qos: 1,
      retain: opts.will.retain ?? true,
    };
  }

  const url = brokerUrl();
  const client = mqtt.connect(url, options);

  return new Promise((resolve, reject) => {
    const onError = (err: Error) => {
      reject(err);
    };
    client.once("error", onError);
    client.once("connect", () => {
      client.removeListener("error", onError);
      console.log(`[${opts.clientId}] connected to ${url}`);
      resolve(client);
    });
  });
}

export function publishJson(
  client: MqttClient,
  topic: string,
  payload: unknown,
  opts: { retain?: boolean; qos?: 0 | 1 | 2 } = {},
): void {
  client.publish(topic, JSON.stringify(payload), {
    retain: opts.retain ?? false,
    qos: opts.qos ?? 1,
  });
}

/**
 * Registers a typed JSON handler. Malformed payloads are logged but not
 * forwarded (tolerance against transient errors).
 */
export function onJson<T>(
  client: MqttClient,
  handler: (topic: string, payload: T) => void,
): void {
  client.on("message", (topic, buf) => {
    const raw = buf.toString();
    if (raw.length === 0) return; // ignore empty (cleared retained) message
    try {
      handler(topic, JSON.parse(raw) as T);
    } catch (err) {
      console.warn(`Invalid JSON message on ${topic}: ${String(err)}`);
    }
  });
}

export function newId(prefix: string): string {
  return `${prefix}-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 7)}`;
}
