// Device identity persisted in NVS. A factory-fresh device only has a serial
// (generated and stored on first boot); the operational deviceId / name /
// location are written when the hub commissions it.
#ifndef IDENTITY_H
#define IDENTITY_H

#include <stdbool.h>
#include "sensors/sensor.h"

#define ID_LEN   80
#define NAME_LEN 96
#define LOC_LEN  256

typedef struct {
    sensor_type_t type;
    char serial[64];
    bool provisioned;       // true once commissioned (device_id present)
    char device_id[ID_LEN];
    char name[NAME_LEN];
    char location_json[LOC_LEN]; // serialised Location object, or "" if none
} identity_t;

// Load identity from NVS for the compile-time device type. If no serial exists
// yet, generate + persist one (factory provisioning). Returns 0 on success.
int identity_load(identity_t *out, sensor_type_t type);

// Persist the operational identity after commissioning.
int identity_save_commissioned(const char *device_id, const char *name,
                               const char *location_json);

#endif // IDENTITY_H
