// Portable device-simulation core (no ESP-IDF dependency, host-testable).
//
// Each device type evolves its own state and serialises it to the exact JSON
// fragment the hub/dashboard expect (the `state` field of a Telemetry message).
#ifndef SENSOR_H
#define SENSOR_H

#include <stdbool.h>
#include <stddef.h>

typedef enum {
    DEV_DOOR = 0,
    DEV_MOTION,
    DEV_BED,
    DEV_STOVE,
    DEV_SOS,
} sensor_type_t;

typedef struct {
    sensor_type_t type;
    // Per-type live state (only the fields for `type` are meaningful).
    bool door_open;
    bool motion;
    int  lux;
    bool bed_occupied;
    int  heart_rate;
    bool stove_on;
    double stove_temp_c;
    bool sos_pressed;
} sensor_t;

// Map a type to/from its wire string ("door","motion","bed","stove","sos").
const char *sensor_type_str(sensor_type_t t);
bool        sensor_type_from_str(const char *s, sensor_type_t *out);

// Lifecycle.
void sensor_init(sensor_t *s, sensor_type_t type);
// Auto-mode evolution (one tick). Uses the process RNG.
void sensor_simulate_step(sensor_t *s);
// Manual toggle of the primary boolean (the "spacebar" action).
void sensor_flip_primary(sensor_t *s);

// Manual value overrides from a command. Pass NULL for fields not being set.
void sensor_apply_command(sensor_t *s,
                          const bool *open,
                          const bool *motion,
                          const bool *occupied,
                          const bool *on,
                          const bool *pressed,
                          const int  *temperature_c);

// Serialise the current state as a JSON object into buf; returns bytes written
// (excluding the NUL), or a negative value on truncation/error.
int sensor_state_json(const sensor_t *s, char *buf, size_t n);

// One-line human description (for logs).
int sensor_describe(const sensor_t *s, char *buf, size_t n);

#endif // SENSOR_H
