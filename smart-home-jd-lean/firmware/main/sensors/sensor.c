// Portable device-simulation core. Mirrors the original TypeScript behaviour so
// the simulated devices stay realistic. No ESP-IDF includes here on purpose:
// the same file is compiled into the firmware and into the host gcc test.
#include "sensor.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

// Uniform random in [0,1). Seeded once by the caller (firmware or host test).
static double frand(void) {
    return (double)rand() / ((double)RAND_MAX + 1.0);
}

const char *sensor_type_str(sensor_type_t t) {
    switch (t) {
        case DEV_DOOR:   return "door";
        case DEV_MOTION: return "motion";
        case DEV_BED:    return "bed";
        case DEV_STOVE:  return "stove";
        case DEV_SOS:    return "sos";
        default:         return "door";
    }
}

bool sensor_type_from_str(const char *s, sensor_type_t *out) {
    if (!s || !out) return false;
    if (!strcmp(s, "door"))   { *out = DEV_DOOR;   return true; }
    if (!strcmp(s, "motion")) { *out = DEV_MOTION; return true; }
    if (!strcmp(s, "bed"))    { *out = DEV_BED;    return true; }
    if (!strcmp(s, "stove"))  { *out = DEV_STOVE;  return true; }
    if (!strcmp(s, "sos"))    { *out = DEV_SOS;    return true; }
    return false;
}

void sensor_init(sensor_t *s, sensor_type_t type) {
    memset(s, 0, sizeof(*s));
    s->type = type;
    switch (type) {
        case DEV_MOTION: s->lux = 120; break;
        case DEV_BED:    s->bed_occupied = true; s->heart_rate = 62; break;
        case DEV_STOVE:  s->stove_temp_c = 20.0; break;
        default: break;
    }
}

void sensor_simulate_step(sensor_t *s) {
    switch (s->type) {
        case DEV_DOOR:
            if (s->door_open) {
                if (frand() < 0.5) s->door_open = false;
            } else if (frand() < 0.3) {
                s->door_open = true;
            }
            break;
        case DEV_MOTION: {
            s->motion = frand() < 0.22;
            double target = s->motion ? 240.0 : 90.0;
            double lux = (double)s->lux + (target - (double)s->lux) * 0.3 + (frand() - 0.5) * 20.0;
            if (lux < 0) lux = 0;
            s->lux = (int)lround(lux);
            break;
        }
        case DEV_BED:
            if (s->bed_occupied) {
                if (frand() < 0.15) s->bed_occupied = false;
            } else if (frand() < 0.4) {
                s->bed_occupied = true;
            }
            s->heart_rate = s->bed_occupied ? (int)lround(58.0 + frand() * 12.0) : 0;
            break;
        case DEV_STOVE:
            if (s->stove_on) {
                if (frand() < 0.25) s->stove_on = false;
            } else if (frand() < 0.25) {
                s->stove_on = true;
            }
            if (s->stove_on) {
                s->stove_temp_c += 18.0 + frand() * 10.0;
                if (s->stove_temp_c > 230.0) s->stove_temp_c = 230.0;
            } else {
                s->stove_temp_c -= 12.0;
                if (s->stove_temp_c < 20.0) s->stove_temp_c = 20.0;
            }
            break;
        case DEV_SOS:
            if (s->sos_pressed) {
                if (frand() < 0.6) s->sos_pressed = false;
            } else if (frand() < 0.03) {
                s->sos_pressed = true;
            }
            break;
    }
}

void sensor_flip_primary(sensor_t *s) {
    switch (s->type) {
        case DEV_DOOR:   s->door_open = !s->door_open; break;
        case DEV_MOTION: s->motion = !s->motion; break;
        case DEV_BED:
            s->bed_occupied = !s->bed_occupied;
            s->heart_rate = s->bed_occupied ? 62 : 0;
            break;
        case DEV_STOVE:
            s->stove_on = !s->stove_on;
            if (s->stove_on) {
                s->stove_temp_c += 18.0;
                if (s->stove_temp_c > 230.0) s->stove_temp_c = 230.0;
            } else {
                s->stove_temp_c -= 12.0;
                if (s->stove_temp_c < 20.0) s->stove_temp_c = 20.0;
            }
            break;
        case DEV_SOS:    s->sos_pressed = !s->sos_pressed; break;
    }
}

void sensor_apply_command(sensor_t *s,
                          const bool *open,
                          const bool *motion,
                          const bool *occupied,
                          const bool *on,
                          const bool *pressed,
                          const int  *temperature_c) {
    switch (s->type) {
        case DEV_DOOR:   if (open)   s->door_open = *open; break;
        case DEV_MOTION: if (motion) s->motion = *motion; break;
        case DEV_BED:
            if (occupied) {
                s->bed_occupied = *occupied;
                s->heart_rate = s->bed_occupied ? 62 : 0;
            }
            break;
        case DEV_STOVE:
            if (on) s->stove_on = *on;
            if (temperature_c) s->stove_temp_c = (double)(*temperature_c);
            break;
        case DEV_SOS:    if (pressed) s->sos_pressed = *pressed; break;
    }
}

int sensor_state_json(const sensor_t *s, char *buf, size_t n) {
    switch (s->type) {
        case DEV_DOOR:
            return snprintf(buf, n, "{\"kind\":\"door\",\"open\":%s}",
                            s->door_open ? "true" : "false");
        case DEV_MOTION:
            return snprintf(buf, n, "{\"kind\":\"motion\",\"motion\":%s,\"lux\":%d}",
                            s->motion ? "true" : "false", s->lux);
        case DEV_BED:
            return snprintf(buf, n, "{\"kind\":\"bed\",\"occupied\":%s,\"heartRate\":%d}",
                            s->bed_occupied ? "true" : "false", s->heart_rate);
        case DEV_STOVE:
            return snprintf(buf, n, "{\"kind\":\"stove\",\"on\":%s,\"temperatureC\":%d}",
                            s->stove_on ? "true" : "false", (int)lround(s->stove_temp_c));
        case DEV_SOS:
            return snprintf(buf, n, "{\"kind\":\"sos\",\"pressed\":%s}",
                            s->sos_pressed ? "true" : "false");
        default:
            return snprintf(buf, n, "{}");
    }
}

int sensor_describe(const sensor_t *s, char *buf, size_t n) {
    switch (s->type) {
        case DEV_DOOR:   return snprintf(buf, n, "%s", s->door_open ? "OPEN" : "closed");
        case DEV_MOTION: return snprintf(buf, n, "%s %d lux", s->motion ? "MOTION" : "still", s->lux);
        case DEV_BED:    return s->bed_occupied
                             ? snprintf(buf, n, "OCCUPIED %d bpm", s->heart_rate)
                             : snprintf(buf, n, "empty");
        case DEV_STOVE:  return snprintf(buf, n, "%s %dC", s->stove_on ? "ON" : "off",
                                         (int)lround(s->stove_temp_c));
        case DEV_SOS:    return snprintf(buf, n, "%s", s->sos_pressed ? "PRESSED" : "idle");
        default:         return snprintf(buf, n, "-");
    }
}
