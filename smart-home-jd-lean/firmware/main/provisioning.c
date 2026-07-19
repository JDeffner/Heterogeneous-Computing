#include "provisioning.h"
#include "net.h"
#include "sensors/catalog.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "freertos/FreeRTOS.h"
#include "freertos/event_groups.h"
#include "esp_log.h"
#include "mqtt_client.h"
#include "cJSON.h"

static const char *TAG = "provisioning";
#define COMMISSIONED_BIT BIT0

static EventGroupHandle_t s_events;
static const identity_t *s_id;
static char s_ad_topic[96];
static char s_ack_topic[112];

static void build_pairing_ad(const identity_t *id, char *buf, size_t n) {
    const catalog_entry_t *c = catalog_get(id->type);
    char ts[40];
    net_iso_now(ts, sizeof(ts));
    int pin = 1000 + (rand() % 9000);
    snprintf(buf, n,
        "{\"serial\":\"%s\",\"type\":\"%s\",\"manufacturer\":\"%s\",\"model\":\"%s\","
        "\"protocol\":\"%s\",\"powerSource\":\"%s\",\"firmware\":\"%s\","
        "\"pairingPin\":\"%04d\",\"advertisedAt\":\"%s\"}",
        id->serial, sensor_type_str(id->type), c->manufacturer, c->model,
        c->protocol, c->power_source, c->firmware, pin, ts);
}

static void on_commissioned(const char *data, int len) {
    cJSON *root = cJSON_ParseWithLength(data, len);
    if (!root) {
        ESP_LOGW(TAG, "could not parse commissioned message");
        return;
    }
    const cJSON *device_id = cJSON_GetObjectItem(root, "deviceId");
    const cJSON *name = cJSON_GetObjectItem(root, "name");
    const cJSON *location = cJSON_GetObjectItem(root, "location");

    char loc_json[LOC_LEN] = "";
    if (location && cJSON_IsObject(location)) {
        char *printed = cJSON_PrintUnformatted(location);
        if (printed) {
            strncpy(loc_json, printed, sizeof(loc_json) - 1);
            cJSON_free(printed);
        }
    }
    if (device_id && cJSON_IsString(device_id)) {
        identity_save_commissioned(device_id->valuestring,
                                   (name && cJSON_IsString(name)) ? name->valuestring : "",
                                   loc_json);
        ESP_LOGI(TAG, "commissioned as %s", device_id->valuestring);
        xEventGroupSetBits(s_events, COMMISSIONED_BIT);
    }
    cJSON_Delete(root);
}

static void mqtt_handler(void *arg, esp_event_base_t base, int32_t id, void *data) {
    (void)arg; (void)base;
    esp_mqtt_event_handle_t e = (esp_mqtt_event_handle_t)data;
    switch ((esp_mqtt_event_id_t)id) {
        case MQTT_EVENT_CONNECTED: {
            esp_mqtt_client_subscribe(e->client, s_ack_topic, 1);
            char ad[512];
            build_pairing_ad(s_id, ad, sizeof(ad));
            // Retained advertisement; the last-will (empty retained) clears it
            // automatically if we drop before commissioning.
            esp_mqtt_client_publish(e->client, s_ad_topic, ad, 0, 1, 1);
            ESP_LOGI(TAG, "advertising %s (serial %s)", sensor_type_str(s_id->type), s_id->serial);
            break;
        }
        case MQTT_EVENT_DATA:
            if (e->topic_len == (int)strlen(s_ack_topic) &&
                strncmp(e->topic, s_ack_topic, e->topic_len) == 0 && e->data_len > 0) {
                on_commissioned(e->data, e->data_len);
            }
            break;
        default:
            break;
    }
}

int provisioning_run(const identity_t *id) {
    s_id = id;
    s_events = xEventGroupCreate();
    snprintf(s_ad_topic, sizeof(s_ad_topic), "smarthome/pairing/%s", id->serial);
    snprintf(s_ack_topic, sizeof(s_ack_topic), "smarthome/pairing/%s/commissioned", id->serial);

    char client_id[80];
    snprintf(client_id, sizeof(client_id), "pairing-%s", id->serial);

    esp_mqtt_client_config_t cfg = { 0 };
    cfg.broker.address.uri = CONFIG_ALSH_MQTT_URI;
    cfg.credentials.client_id = client_id;
    cfg.session.last_will.topic = s_ad_topic;
    cfg.session.last_will.msg = "";
    cfg.session.last_will.msg_len = 0;
    cfg.session.last_will.qos = 1;
    cfg.session.last_will.retain = 1;

    esp_mqtt_client_handle_t client = esp_mqtt_client_init(&cfg);
    esp_mqtt_client_register_event(client, ESP_EVENT_ANY_ID, mqtt_handler, NULL);
    esp_mqtt_client_start(client);

    ESP_LOGI(TAG, "waiting to be commissioned ...");
    xEventGroupWaitBits(s_events, COMMISSIONED_BIT, pdFALSE, pdTRUE, portMAX_DELAY);

    esp_mqtt_client_stop(client);
    esp_mqtt_client_destroy(client);
    return 0;
}
