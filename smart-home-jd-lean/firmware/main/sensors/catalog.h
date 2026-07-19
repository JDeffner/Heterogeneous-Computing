// Device catalog: faithful "spec sheet" per device type. Portable / host-testable.
#ifndef CATALOG_H
#define CATALOG_H

#include <stddef.h>
#include "sensor.h"

typedef struct {
    const char *manufacturer;
    const char *model;
    const char *protocol;     // "Zigbee" | "Z-Wave" | "Wi-Fi" | "BLE"
    const char *power_source; // "battery" | "mains"
    const char *firmware;
    const char *capabilities; // pre-rendered JSON array, e.g. ["door.open"]
    int   nominal_rssi;
    int   report_interval_ms;
} catalog_entry_t;

const catalog_entry_t *catalog_get(sensor_type_t t);

// Factory serial like "SN-DWS100-3f9ac2". Writes into buf.
int catalog_make_serial(sensor_type_t t, char *buf, size_t n);

#endif // CATALOG_H
