#include "catalog.h"

#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>

static const catalog_entry_t CATALOG[] = {
    [DEV_DOOR] = {
        .manufacturer = "Aqara", .model = "DW-S100", .protocol = "Zigbee",
        .power_source = "battery", .firmware = "esp-2.1.0",
        .capabilities = "[\"door.open\"]", .nominal_rssi = -62, .report_interval_ms = 2500,
    },
    [DEV_MOTION] = {
        .manufacturer = "Philips Hue", .model = "SML-002", .protocol = "Zigbee",
        .power_source = "battery", .firmware = "esp-2.1.0",
        .capabilities = "[\"motion.detected\",\"motion.lux\"]", .nominal_rssi = -58,
        .report_interval_ms = 2500,
    },
    [DEV_BED] = {
        .manufacturer = "Emfit", .model = "QS-Care", .protocol = "Wi-Fi",
        .power_source = "mains", .firmware = "esp-3.0.4",
        .capabilities = "[\"bed.occupied\",\"bed.heartRate\"]", .nominal_rssi = -47,
        .report_interval_ms = 2500,
    },
    [DEV_STOVE] = {
        .manufacturer = "Inirv", .model = "Guard-Z", .protocol = "Z-Wave",
        .power_source = "mains", .firmware = "esp-1.4.2",
        .capabilities = "[\"stove.on\",\"stove.temp\"]", .nominal_rssi = -51,
        .report_interval_ms = 2500,
    },
    [DEV_SOS] = {
        .manufacturer = "CareTech", .model = "SOS-Pendant", .protocol = "BLE",
        .power_source = "battery", .firmware = "esp-1.0.1",
        .capabilities = "[\"sos.pressed\"]", .nominal_rssi = -70, .report_interval_ms = 2500,
    },
};

const catalog_entry_t *catalog_get(sensor_type_t t) {
    return &CATALOG[t];
}

int catalog_make_serial(sensor_type_t t, char *buf, size_t n) {
    const catalog_entry_t *c = catalog_get(t);
    char model[32];
    size_t j = 0;
    for (const char *p = c->model; *p && j < sizeof(model) - 1; ++p) {
        if (isalnum((unsigned char)*p)) model[j++] = *p;
    }
    model[j] = '\0';
    unsigned hex = (unsigned)(rand() & 0xffffff);
    return snprintf(buf, n, "SN-%s-%06x", model, hex);
}
