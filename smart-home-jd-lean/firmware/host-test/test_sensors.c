// Host-side test of the portable sensor core (no ESP-IDF). Compile with:
//   cc -I../main/sensors -o test_sensors test_sensors.c ../main/sensors/sensor.c ../main/sensors/catalog.c -lm
// Verifies state evolution stays in range and the JSON fragments are well formed.
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "sensor.h"
#include "catalog.h"

static int fails = 0;
#define CHECK(cond, msg)                                        \
    do {                                                        \
        if (!(cond)) { printf("FAIL: %s\n", msg); fails++; }    \
    } while (0)

static int contains(const char *hay, const char *needle) {
    return strstr(hay, needle) != NULL;
}

int main(void) {
    srand(12345);
    char buf[256];

    // Type string round-trip.
    for (sensor_type_t t = DEV_DOOR; t <= DEV_SOS; ++t) {
        sensor_type_t back;
        CHECK(sensor_type_from_str(sensor_type_str(t), &back) && back == t,
              "type string round-trip");
    }

    // Each type: init, many steps, JSON well formed and discriminator present.
    const char *kinds[] = {"door", "motion", "bed", "stove", "sos"};
    for (sensor_type_t t = DEV_DOOR; t <= DEV_SOS; ++t) {
        sensor_t s;
        sensor_init(&s, t);
        for (int i = 0; i < 2000; ++i) {
            sensor_simulate_step(&s);
            int w = sensor_state_json(&s, buf, sizeof(buf));
            CHECK(w > 0 && (size_t)w < sizeof(buf), "state json fits");
            CHECK(buf[0] == '{' && buf[strlen(buf) - 1] == '}', "state json braces");
            char kindkey[32];
            snprintf(kindkey, sizeof(kindkey), "\"kind\":\"%s\"", kinds[t]);
            CHECK(contains(buf, kindkey), "state json kind discriminator");
        }
    }

    // Range invariants.
    {
        sensor_t s;
        sensor_init(&s, DEV_STOVE);
        for (int i = 0; i < 5000; ++i) {
            sensor_simulate_step(&s);
            CHECK(s.stove_temp_c >= 20.0 - 1e-9 && s.stove_temp_c <= 230.0 + 1e-9,
                  "stove temp in [20,230]");
        }
    }
    {
        sensor_t s;
        sensor_init(&s, DEV_MOTION);
        for (int i = 0; i < 5000; ++i) {
            sensor_simulate_step(&s);
            CHECK(s.lux >= 0, "motion lux non-negative");
        }
    }
    {
        sensor_t s;
        sensor_init(&s, DEV_BED);
        for (int i = 0; i < 5000; ++i) {
            sensor_simulate_step(&s);
            if (s.bed_occupied) {
                CHECK(s.heart_rate >= 50 && s.heart_rate <= 80, "bed bpm sane when occupied");
            } else {
                CHECK(s.heart_rate == 0, "bed bpm 0 when empty");
            }
        }
    }

    // Manual command override.
    {
        sensor_t s;
        sensor_init(&s, DEV_STOVE);
        bool on = true;
        int temp = 200;
        sensor_apply_command(&s, NULL, NULL, NULL, &on, NULL, &temp);
        CHECK(s.stove_on == true && (int)s.stove_temp_c == 200, "stove command applied");
        sensor_state_json(&s, buf, sizeof(buf));
        CHECK(contains(buf, "\"on\":true") && contains(buf, "\"temperatureC\":200"),
              "stove command reflected in json");
    }

    // flip primary.
    {
        sensor_t s;
        sensor_init(&s, DEV_DOOR);
        bool before = s.door_open;
        sensor_flip_primary(&s);
        CHECK(s.door_open != before, "door flip toggles");
    }

    // Catalog + serial format.
    for (sensor_type_t t = DEV_DOOR; t <= DEV_SOS; ++t) {
        const catalog_entry_t *c = catalog_get(t);
        CHECK(c->manufacturer && c->model && c->protocol && c->power_source, "catalog populated");
        char serial[64];
        int w = catalog_make_serial(t, serial, sizeof(serial));
        CHECK(w > 0 && strncmp(serial, "SN-", 3) == 0, "serial format SN-...");
    }

    if (fails == 0) {
        printf("ALL SENSOR-CORE TESTS PASSED\n");
        return 0;
    }
    printf("%d CHECK(S) FAILED\n", fails);
    return 1;
}
