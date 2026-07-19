# Code Guide - Understanding the Assisted-Living Smart Home

This explains the project from the ground up: the infrastructure concepts (MQTT,
the broker), then the three codebases (the shared contract, the Rust
hub/dashboard, the ESP32 firmware), and finally a few traced end-to-end flows.

---

## Part 1 - The big picture

Four kinds of running programs, coupled only through the broker:

```
   ESP32 device   ESP32 device   ESP32 device      (one C firmware per role)
        |              |              |
        |   publish / subscribe (MQTT)|
        v              v              v
   ============== MQTT BROKER (Mosquitto) ==============   routes by topic
        ^              ^              ^
        |              |              |
   hub (Rust binary)        dashboard (Rust binary)
   registry/rules/alarms    web UI + SSE mirror
```

The golden rule: no program talks to another directly. Everything is MQTT
messages with JSON payloads. That is what makes the system loosely coupled: any
piece can stop and restart while the others keep running.

- The broker (Mosquitto) is a dumb-but-reliable message router.
- The hub is the brain: device registry, safety rules, alarms.
- Each device is an ESP32 running C firmware that pretends to be a physical
  sensor. It sends readings and reacts to commands.
- The dashboard is a read-mostly operator console.

### MQTT features we rely on

1. Wildcards (`+`, `#`) for discovery without shared config (the hub subscribes
   to `smarthome/devices/+/telemetry`).
2. Retained messages for current state (registry, presence, rooms, alarms): the
   broker replays the last one to late subscribers. Clear one by publishing an
   empty retained payload.
3. QoS 1 (at least once) everywhere.
4. Last Will (LWT): a message the broker publishes automatically if a client
   dies. Powers automatic offline detection and clearing of abandoned pairing ads.

---

## Part 2 - The shared contract (`hub/crates/shared`)

The single source of truth for the wire format.

- `types.rs` - every message/object as a serde struct or enum. Field names are
  camelCase (`deviceId`, `powerSource`); device state is a tagged enum with a
  `kind` discriminator (`{"kind":"stove","on":true,"temperatureC":210}`). The
  ESP32 firmware emits byte-identical JSON.
- `topics.rs` - every topic string in one place, plus wildcard subscriptions and
  helpers that pull an id back out of a topic (`device_id_from_topic`, ...).
- `util.rs` - ISO-8601 timestamps and base36 id generation (`evt-...`, `alm-...`).

---

## Part 3 - The hub (`hub/crates/hub`) - the brain

One async (`tokio`) process built on `rumqttc`. `main.rs` is the switchboard: it
connects, restores rooms + registry from disk, subscribes, then runs a loop that
`select!`s between incoming MQTT messages and a periodic rule-evaluation tick.

- `config.rs` - thresholds from env vars (small seconds for demos).
- `catalog.rs` - the faithful spec sheet per device type + serial generation.
- `registry.rs` - the single owner of `DeviceRecord`s. Persists to
  `data/devices.json` and publishes each record retained to `registry/<id>`.
- `rules.rs` - watches presence + telemetry, keeps small per-device/room state,
  and is edge-triggered: each rule method returns the situations it produced
  (raised once, cleared once). SOS latches: a press raises a critical alarm that
  does not auto-clear.
- `alarms.rs` - turns situations into `Alarm` objects published retained to
  `alarms/<id>`, deduplicated; resolves by clearing the retained message.

The rule engine returns events that `main` publishes and feeds to the alarm
engine, so there are no shared callbacks or locks.

---

## Part 4 - The dashboard (`hub/crates/dashboard`)

An `axum` server that is itself another MQTT client. It mirrors the bus into an
in-memory snapshot and streams it to the browser via Server-Sent Events
(`/api/stream`). Control endpoints only publish a control message to the hub; the
server has no logic of its own. The browser page is the single self-contained
`static/index.html`, embedded into the binary with `include_str!`.

---

## Part 5 - The firmware (`firmware/`) - the devices

ESP-IDF C, one image configurable to a role via menuconfig. See
`firmware/README.md` for build/flash.

- `main/sensors/sensor.c` + `catalog.c` - portable, dependency-free C: the
  per-type state machine, the telemetry-`state` JSON serialiser, and the spec
  sheets. Compiled into both the firmware and the host gcc test.
- `main/net.c` - Wi-Fi station + SNTP (real ISO timestamps).
- `main/identity.c` - identity in NVS (factory serial on first boot; operational
  id/name/room after commissioning).
- `main/provisioning.c` - pairing: advertise a retained ad with a last-will that
  clears it, wait for the commissioned reply, persist identity, reboot.
- `main/device_runtime.c` - normal operation: presence online (retained) with an
  offline last-will, a telemetry task every ~2.5 s, and command handling.

---

## Part 6 - Flows, traced end to end

### A - Onboarding (pairing)
1. A fresh ESP32 mints a serial (NVS) and enters pairing mode: `provisioning.c`
   publishes a retained `PairingAd` to `pairing/<serial>` (will = empty retained).
2. The hub records the ad; the dashboard shows it under "Devices waiting to pair".
3. You commission it (pick a room). The dashboard publishes a `CommissionRequest`
   to `control/commission`.
4. The hub mints a `DeviceRecord`, publishes it retained to `registry/<id>`,
   replies on `pairing/<serial>/commissioned`, and clears the ad.
5. The device stores its identity and reboots into normal operation.

### B - A reading becomes an alarm
1. The stove firmware publishes telemetry `{on:true, temperatureC:210}`.
2. The hub routes it to `rules.handle_telemetry` (records "stove on since T").
3. Every ~2 s `rules.evaluate` sees the stove on past threshold with no motion in
   the room and returns a `stove_on_no_motion` situation.
4. `main` publishes the event and calls `alarms.handle_event`, which publishes a
   retained `Alarm`. The dashboard streams it; Resolve clears the retained alarm.

### C - A device drops off
1. The ESP32 loses power / Wi-Fi. The broker publishes its last-will
   `presence=offline` (retained).
2. The hub flips the registry to offline and raises `device_offline`.
3. On reconnect it republishes `presence=online`; the warning auto-clears.

### D - Simulate night
1. The dashboard publishes `{forceNight:true}` to `control/hub`.
2. The hub sets the flag and republishes `hub/status` (retained); the
   bed-at-night rule can now fire at any time.

---

## Glossary

- Broker - the MQTT message router (Mosquitto).
- Retained message - last message the broker replays to new subscribers.
- Last-Will (LWT) - message published automatically when a client dies.
- Commissioning - giving a fresh device its operational identity + room.
- NVS - the ESP32's non-volatile key/value store (keeps the device identity).
- SSE - one-way stream from the dashboard server to the browser.
