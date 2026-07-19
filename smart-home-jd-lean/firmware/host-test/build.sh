#!/usr/bin/env bash
# Compile and run the portable sensor-core tests on the host (no ESP-IDF needed).
set -euo pipefail
cd "$(dirname "$0")"
cc -std=c11 -Wall -Wextra -I../main/sensors -o test_sensors \
   test_sensors.c ../main/sensors/sensor.c ../main/sensors/catalog.c -lm
./test_sensors
