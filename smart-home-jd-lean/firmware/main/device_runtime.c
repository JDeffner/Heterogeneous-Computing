#include "device_runtime.h"
#include "net.h"
#include "sensors/sensor.h"
#include "sensors/catalog.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "freertos/semphr.h"
#include "esp_log.h"
#include "mqtt_client.h"
#include "cJSON.h"

static const char *TAG = "device";

static esp_mqtt_client_handle_t s_client;
static SemaphoreHandle_t s_lock;
static sensor_t s_sensor;
static const identity_t *s_id;
static int s_mode_manual = 0;   // 0 = auto, 1 = manual
static int s_link_up = 1;
static double s_battery = 100.0;
static int s_rssi = -60;
static int s_report_ms = 2500;

static char s_t_presence[96];
static char s_t_telemetry[96];
static char s_t_command[96];
static char s_t_registry[96];

static double frand(void) { return (double)rand() / ((double)RAND_MAX + 1.0); }

static void update_metrics(void) {
    const catalog_entry_t *c = catalog_get(s_id->type);
    if (strcmp(c->power_source, "battery") == 0) {
        s_battery -= frand() * 0.4;
        if (s_battery < 1.0) s_battery = 1.0;
    }
    double nominal = (double)c->nominal_rssi;
    double r = (double)s_rssi + (frand() - 0.5) * 6.0 + (nominal - (double)s_rssi) * 0.2;
    if (r < -95) r = -95;
    if (r > -35) r = -35;
    s_rssi = (int)lround(r);
}

static void publish_telemetry(void) {
    char state[160];
    char msg[400];
    xSemaphoreTake(s_lock, portMAX_DELAY);
    sensor_state_json(&s_sensor, state, sizeof(state));
    char ts[40];
    net_iso_now(ts, sizeof(ts));
    snprintf(msg, sizeof(msg),
        "{\"deviceId\":\"%s\",\"type\":\"%s\",\"state\":%s,\"mode\":\"%s\","
        "\"battery\":%d,\"rssi\":%d,\"ts\":\"%s\"}",
        s_id->device_id, sensor_type_str(s_sensor.type), state,
        s_mode_manual ? "manual" : "auto", (int)lround(s_battery), s_rssi, ts);
    xSemaphoreGive(s_lock);
    esp_mqtt_client_publish(s_client, s_t_telemetry, msg, 0, 1, 0);
}

static void set_link(int up) {
    if (up == s_link_up) return;
    s_link_up = up;
    esp_mqtt_client_publish(s_client, s_t_presence, up ? "online" : "offline", 0, 1, 1);
    ESP_LOGI(TAG, "simulated link %s", up ? "UP" : "DOWN");
}

static void handle_command(const char *data, int len) {
    cJSON *root = cJSON_ParseWithLength(data, len);
    if (!root) return;

    const cJSON *mode = cJSON_GetObjectItem(root, "mode");
    if (mode && cJSON_IsString(mode)) {
        s_mode_manual = strcmp(mode->valuestring, "manual") == 0;
    }
    const cJSON *online = cJSON_GetObjectItem(root, "online");
    if (online && cJSON_IsBool(online)) {
        set_link(cJSON_IsTrue(online) ? 1 : 0);
        if (!s_link_up) { cJSON_Delete(root); return; }
    }

    bool b_open, b_motion, b_occ, b_on, b_press; int temp;
    const bool *p_open = NULL, *p_motion = NULL, *p_occ = NULL, *p_on = NULL, *p_press = NULL;
    const int *p_temp = NULL;
    const cJSON *it;
    if ((it = cJSON_GetObjectItem(root, "open")) && cJSON_IsBool(it))    { b_open = cJSON_IsTrue(it); p_open = &b_open; }
    if ((it = cJSON_GetObjectItem(root, "motion")) && cJSON_IsBool(it))  { b_motion = cJSON_IsTrue(it); p_motion = &b_motion; }
    if ((it = cJSON_GetObjectItem(root, "occupied")) && cJSON_IsBool(it)){ b_occ = cJSON_IsTrue(it); p_occ = &b_occ; }
    if ((it = cJSON_GetObjectItem(root, "on")) && cJSON_IsBool(it))      { b_on = cJSON_IsTrue(it); p_on = &b_on; }
    if ((it = cJSON_GetObjectItem(root, "pressed")) && cJSON_IsBool(it)) { b_press = cJSON_IsTrue(it); p_press = &b_press; }
    if ((it = cJSON_GetObjectItem(root, "temperatureC")) && cJSON_IsNumber(it)) { temp = it->valueint; p_temp = &temp; }

    xSemaphoreTake(s_lock, portMAX_DELAY);
    sensor_apply_command(&s_sensor, p_open, p_motion, p_occ, p_on, p_press, p_temp);
    xSemaphoreGive(s_lock);

    cJSON_Delete(root);
    publish_telemetry();
}

static void mqtt_handler(void *arg, esp_event_base_t base, int32_t id, void *data) {
    (void)arg; (void)base;
    esp_mqtt_event_handle_t e = (esp_mqtt_event_handle_t)data;
    switch ((esp_mqtt_event_id_t)id) {
        case MQTT_EVENT_CONNECTED:
            // Announce alive (retained), then listen for commands + registry.
            esp_mqtt_client_publish(e->client, s_t_presence, "online", 0, 1, 1);
            esp_mqtt_client_subscribe(e->client, s_t_command, 1);
            esp_mqtt_client_subscribe(e->client, s_t_registry, 1);
            ESP_LOGI(TAG, "online as %s (%s)", sensor_type_str(s_id->type), s_id->device_id);
            break;
        case MQTT_EVENT_DATA:
            if (e->data_len > 0 && e->topic_len == (int)strlen(s_t_command) &&
                strncmp(e->topic, s_t_command, e->topic_len) == 0) {
                handle_command(e->data, e->data_len);
            }
            // registry updates (name/room) are informational here; ignored.
            break;
        default:
            break;
    }
}

static void telemetry_task(void *arg) {
    (void)arg;
    publish_telemetry(); // first report immediately
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(s_report_ms));
        if (!s_link_up) continue; // unreachable: stay silent
        if (!s_mode_manual) {
            xSemaphoreTake(s_lock, portMAX_DELAY);
            sensor_simulate_step(&s_sensor);
            xSemaphoreGive(s_lock);
        }
        update_metrics();
        publish_telemetry();
    }
}

void device_runtime_run(const identity_t *id) {
    s_id = id;
    s_lock = xSemaphoreCreateMutex();
    sensor_init(&s_sensor, id->type);

    const catalog_entry_t *c = catalog_get(id->type);
    s_report_ms = c->report_interval_ms;
    s_rssi = c->nominal_rssi;
    s_battery = (strcmp(c->power_source, "battery") == 0) ? (70.0 + (rand() % 30)) : 100.0;

    snprintf(s_t_presence, sizeof(s_t_presence), "smarthome/devices/%s/presence", id->device_id);
    snprintf(s_t_telemetry, sizeof(s_t_telemetry), "smarthome/devices/%s/telemetry", id->device_id);
    snprintf(s_t_command, sizeof(s_t_command), "smarthome/devices/%s/command", id->device_id);
    snprintf(s_t_registry, sizeof(s_t_registry), "smarthome/registry/%s", id->device_id);

    esp_mqtt_client_config_t cfg = { 0 };
    cfg.broker.address.uri = CONFIG_ALSH_MQTT_URI;
    char client_id[96];
    snprintf(client_id, sizeof(client_id), "sensor-%s", id->device_id);
    cfg.credentials.client_id = client_id;
    cfg.session.last_will.topic = s_t_presence;
    cfg.session.last_will.msg = "offline";
    cfg.session.last_will.msg_len = 0; // 0 -> use strlen
    cfg.session.last_will.qos = 1;
    cfg.session.last_will.retain = 1;

    s_client = esp_mqtt_client_init(&cfg);
    esp_mqtt_client_register_event(s_client, ESP_EVENT_ANY_ID, mqtt_handler, NULL);
    esp_mqtt_client_start(s_client);

    xTaskCreate(telemetry_task, "telemetry", 4096, NULL, 5, NULL);

    // device_runtime_run never returns; the telemetry task + mqtt drive the device.
    for (;;) {
        vTaskDelay(pdMS_TO_TICKS(1000));
    }
}
