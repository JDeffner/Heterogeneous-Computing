idf_component_register(
    SRCS
        "app_main.c"
        "net.c"
        "identity.c"
        "provisioning.c"
        "device_runtime.c"
        "sensors/sensor.c"
        "sensors/catalog.c"
    INCLUDE_DIRS "." "sensors"
    REQUIRES nvs_flash esp_wifi esp_netif esp_event esp_timer mqtt json lwip
)
