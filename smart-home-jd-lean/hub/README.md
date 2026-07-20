# Hub backend (Rust workspace)

Two long-running binaries plus one helper, all self-contained (no runtime, no
Docker): copy the binary to the target and run it. Cross-compiles to a Raspberry
Pi or any Linux gateway.

```
hub/
  Cargo.toml                 workspace
  crates/
    shared/                  wire types + topic schema (the contract)
    hub/        -> bin hub          registry, rules, alarms, commissioning
    dashboard/  -> bin dashboard    axum web UI + SSE, MQTT mirror
    ctl/        -> bin ctl          seed rooms / clear retained state
    sim/        -> bin sim          simulated devices (all five roles, no hardware)
```

## Build

```bash
cd hub
cargo build --release
# binaries in target/release/{hub,dashboard,ctl}
```

Cross-compile for a 64-bit Raspberry Pi:

```bash
rustup target add aarch64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu
```

## Run (broker must be up first)

```bash
# 1) broker (native, no Docker):  mosquitto -c ../mosquitto/mosquitto.conf
# 2) hub
MQTT_URL=mqtt://localhost:1883 DATA_DIR=../data ./target/release/hub
# 3) dashboard -> http://localhost:3000
MQTT_URL=mqtt://localhost:1883 WEB_PORT=3000 ./target/release/dashboard
# 4) optional: create a default set of rooms
MQTT_URL=mqtt://localhost:1883 ./target/release/ctl seed-rooms
```

## Environment variables

| Var | Default | Used by | Meaning |
|---|---|---|---|
| `MQTT_URL` | `mqtt://localhost:1883` | all | broker address |
| `DATA_DIR` | `./data` | hub, ctl | where `devices.json` / `rooms.json` live |
| `WEB_PORT` | `3000` | dashboard | HTTP port |
| `EVAL_TICK_MS` | `2000` | hub | rule evaluation interval |
| `STOVE_ON_SECONDS` / `STOVE_NO_MOTION_SECONDS` | `12` / `12` | hub | stove rule |
| `DOOR_OPEN_SECONDS` / `DOOR_NO_MOTION_SECONDS` | `10` / `12` | hub | door rule |
| `BED_ABSENCE_SECONDS` | `15` | hub | bed-at-night rule |
| `NIGHT_START` / `NIGHT_END` | `22` / `7` | hub | night window (hours) |
| `FORCE_NIGHT` | `false` | hub | force night (also toggled from the UI) |

## Reset

```bash
./target/release/ctl clear   # clears retained broker state + data/*.json
```
