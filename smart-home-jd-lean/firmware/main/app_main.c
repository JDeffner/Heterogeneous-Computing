// Assisted-Living smart-home device firmware (ESP32, ESP-IDF).
//
// One firmware, configurable to one of five device roles (menuconfig). On first
// boot it mints a factory serial and enters pairing mode; once the hub
// commissions it, the operational identity is stored in NVS and the device
// reboots into normal operation (presence + telemetry + commands).
#include <stdlib.h>

#include "esp_log.h"
#include "esp_system.h"
#include "esp_random.h"
#include "nvs_flash.h"

#include "net.h"
#include "identity.h"
#include "provisioning.h"
#include "device_runtime.h"
#include "sensors/sensor.h"

static const char *TAG = "app";

static sensor_type_t configured_type(void) {
#if defined(CONFIG_ALSH_DEVICE_TYPE_MOTION)
    return DEV_MOTION;
#elif defined(CONFIG_ALSH_DEVICE_TYPE_BED)
    return DEV_BED;
#elif defined(CONFIG_ALSH_DEVICE_TYPE_STOVE)
    return DEV_STOVE;
#elif defined(CONFIG_ALSH_DEVICE_TYPE_SOS)
    return DEV_SOS;
#else
    return DEV_DOOR;
#endif
}

void app_main(void) {
    esp_err_t err = nvs_flash_init();
    if (err == ESP_ERR_NVS_NO_FREE_PAGES || err == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        err = nvs_flash_init();
    }
    ESP_ERROR_CHECK(err);

    srand(esp_random()); // seed the simulation RNG from hardware entropy

    net_connect_blocking();

    identity_t id;
    if (identity_load(&id, configured_type()) != 0) {
        ESP_LOGE(TAG, "identity load failed");
        return;
    }

    if (!id.provisioned) {
        ESP_LOGI(TAG, "unprovisioned -> entering pairing mode");
        provisioning_run(&id);
        ESP_LOGI(TAG, "commissioned; rebooting into normal operation");
        esp_restart();
    }

    ESP_LOGI(TAG, "provisioned as %s (%s); starting", id.device_id, sensor_type_str(id.type));
    device_runtime_run(&id); // never returns
}
