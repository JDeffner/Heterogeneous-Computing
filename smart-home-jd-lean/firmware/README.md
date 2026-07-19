# Device firmware (ESP32 / ESP-IDF, C)

One firmware image, configurable to any of the five device roles. It simulates a
realistic sensor and talks MQTT to the hub. No physical sensor is required: the
device-simulation core (`main/sensors/`) produces realistic state; on real
hardware you would replace those state functions with GPIO/ADC reads.

## Layout

```
firmware/
  CMakeLists.txt            top-level IDF project
  sdkconfig.defaults        build defaults
  main/
    app_main.c              orchestration: wifi -> identity -> pair OR run
    net.c                   Wi-Fi station + SNTP (real ISO timestamps)
    identity.c              NVS-persisted identity (serial, deviceId, room)
    provisioning.c          pairing advertise + wait-for-commission
    device_runtime.c        presence (LWT), telemetry loop, command handling
    sensors/
      sensor.c              portable per-type simulation + state JSON
      catalog.c             faithful spec sheets per device type
  host-test/
    test_sensors.c          gcc test of the portable core (no hardware needed)
```

The files in `main/sensors/` are plain, dependency-free C and are compiled both
into the firmware and into the host test, so the simulation logic and the exact
telemetry JSON are verifiable without an ESP32.

## Onboarding model (faithful to real hardware)

1. First boot: the device mints a factory **serial** (stored in NVS) and enters
   **pairing mode**. It advertises a retained `PairingAd` on
   `smarthome/pairing/<serial>`; its last-will (empty retained) clears that ad if
   it drops before being commissioned.
2. From the operator dashboard ("Devices waiting to pair") you **commission** it:
   the hub mints the operational `deviceId`, optionally assigns a room, replies on
   `smarthome/pairing/<serial>/commissioned`, and clears the ad.
3. The device persists its identity to NVS and **reboots** into normal operation:
   presence online (retained, with an offline last-will), periodic telemetry, and
   command handling on `smarthome/devices/<id>/command`.

## Build & flash

```bash
# one-time: install ESP-IDF v5.x and export its environment
. $IDF_PATH/export.sh

cd firmware
idf.py set-target esp32          # or esp32c3, esp32s3, ...
idf.py menuconfig                # set Wi-Fi SSID/password, MQTT broker URI, device role
idf.py build flash monitor
```

Flash one board per role (door, motion, bed, stove, sos). Each board with the
same role uses a distinct serial (minted per chip), so you can run several.

## No board handy?

Run the logic test on your PC:

```bash
cd firmware/host-test && ./build.sh
```

ESP-IDF also ships a QEMU target (`idf.py qemu`) for the esp32 if you want to run
the firmware without hardware; networking in QEMU requires extra setup.
