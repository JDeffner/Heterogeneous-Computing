#include "identity.h"
#include "sensors/catalog.h"

#include <string.h>
#include <stdlib.h>

#include "nvs.h"
#include "esp_log.h"

static const char *NS = "alsh";
static const char *TAG = "identity";

static void get_str(nvs_handle_t h, const char *key, char *buf, size_t n) {
    size_t len = n;
    buf[0] = '\0';
    nvs_get_str(h, key, buf, &len);
}

int identity_load(identity_t *out, sensor_type_t type) {
    memset(out, 0, sizeof(*out));
    out->type = type;

    nvs_handle_t h;
    esp_err_t err = nvs_open(NS, NVS_READWRITE, &h);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "nvs_open failed: %s", esp_err_to_name(err));
        return -1;
    }

    get_str(h, "serial", out->serial, sizeof(out->serial));
    if (out->serial[0] == '\0') {
        // First boot: mint and persist a stable factory serial.
        catalog_make_serial(type, out->serial, sizeof(out->serial));
        nvs_set_str(h, "serial", out->serial);
        nvs_commit(h);
        ESP_LOGI(TAG, "minted factory serial %s", out->serial);
    }

    get_str(h, "device_id", out->device_id, sizeof(out->device_id));
    get_str(h, "name", out->name, sizeof(out->name));
    get_str(h, "loc", out->location_json, sizeof(out->location_json));
    out->provisioned = out->device_id[0] != '\0';

    nvs_close(h);
    return 0;
}

int identity_save_commissioned(const char *device_id, const char *name,
                               const char *location_json) {
    nvs_handle_t h;
    if (nvs_open(NS, NVS_READWRITE, &h) != ESP_OK) return -1;
    nvs_set_str(h, "device_id", device_id);
    nvs_set_str(h, "name", name ? name : "");
    nvs_set_str(h, "loc", location_json ? location_json : "");
    esp_err_t err = nvs_commit(h);
    nvs_close(h);
    return err == ESP_OK ? 0 : -1;
}
