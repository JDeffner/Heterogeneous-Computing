// The live device: presence (online + last-will offline), periodic telemetry,
// and command handling. Runs forever once the device is provisioned.
#ifndef DEVICE_RUNTIME_H
#define DEVICE_RUNTIME_H

#include "identity.h"

void device_runtime_run(const identity_t *id);

#endif // DEVICE_RUNTIME_H
