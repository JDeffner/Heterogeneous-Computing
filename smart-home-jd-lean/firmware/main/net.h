// Wi-Fi station bring-up + SNTP time sync (ESP-IDF).
#ifndef NET_H
#define NET_H

// Connect to the configured AP and block until an IP is obtained, then start
// SNTP so telemetry timestamps are real ISO-8601. Returns 0 on success.
int net_connect_blocking(void);

// Format the current UTC time as ISO-8601 ("...Z") into buf.
void net_iso_now(char *buf, int n);

// Current STA RSSI in dBm (real radio link to the AP), or 0 if unavailable.
int net_wifi_rssi(void);

#endif // NET_H
