// Factory onboarding: advertise in pairing mode and wait for the hub to
// commission this device, then persist the operational identity.
#ifndef PROVISIONING_H
#define PROVISIONING_H

#include "identity.h"

// Advertise the pairing ad and block until commissioned (identity persisted to
// NVS). Returns 0 on success; the caller should then esp_restart().
int provisioning_run(const identity_t *id);

#endif // PROVISIONING_H
