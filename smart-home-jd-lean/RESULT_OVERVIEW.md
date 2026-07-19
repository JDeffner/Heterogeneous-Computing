# Ergebnisuebersicht - Smart-Home-Demonstrator (Assisted Living)

Kurze Einordnung, wie die in der Aufgabenstellung geforderten Ueberlegungen im
Prototyp umgesetzt wurden. Der Demonstrator simuliert ein Assisted-Living-Zuhause
mit **Mosquitto + MQTT** und einem **zentralisierten** Modell: alle Geraete
kommunizieren ueber den Broker mit einem zentralen **Hub** (Registry, Regeln,
Alarme). Geraete sind echte **ESP32-Firmware in C**; Hub und Dashboard sind zwei
eigenstaendige **Rust-Binaerdateien**.

---

## 1. Recherche-Grundlage

Bestehende Smart-Home-Systeme sind zentral, dezentral oder hybrid aufgebaut.
Offene Plattformen (Home Assistant, openHAB, ioBroker, Node-RED) integrieren
heterogene Geraete ueber Adapter. Auf Kommunikationsebene dominieren **MQTT**
(Broker-basiertes Publish/Subscribe) und CoAP; auf Netzwerkebene **Zigbee,
Z-Wave, Thread, Wi-Fi, BLE**, zunehmend **Matter** als IP-basierte
Interoperabilitaetsschicht. Aktueller Trend ist die Aufteilung in **Cloud**
(Fernzugriff, Verwaltung) und **Edge** (lokale Logik, Latenz, Ausfallsicherheit).
Daraus ergeben sich die zentralen Anforderungen: Verteilung, Skalierbarkeit,
Fehlertoleranz, Robustheit, lose Kopplung, Heterogenitaet und Erweiterbarkeit.

## 2. Architektur, Technologiewahl & Kommunikationsmodell

Umgesetzt wurde das Modell **Geraet -> Gateway/Hub -> Automatisierung**
(zentralisiert, Edge-basiert). Kein Geraet spricht ein anderes direkt an; die
gesamte Kommunikation laeuft als **Publish/Subscribe ueber den MQTT-Broker**.

Die Technologien sind bewusst gemaess Einsatzschicht gewaehlt, statt eine Sprache
ueberall zu erzwingen:

- **Geraete: C auf ESP32 (ESP-IDF).** Echte Firmware laeuft direkt auf dem
  Mikrocontroller, ohne Laufzeitumgebung und ohne Docker. Die Geraete sprechen
  MQTT ueber `esp-mqtt`; die simulierte Sensorik (`firmware/main/sensors/`) wuerde
  auf realer Hardware durch GPIO/ADC-Lesungen ersetzt.
- **Hub + Dashboard: Rust.** Zwei statische Binaerdateien (`hub`, `dashboard`),
  per `cargo build --release` erzeugt und auf ein Gateway (z. B. Raspberry Pi)
  cross-kompilierbar. Deployment heisst: Datei kopieren, als `systemd`-Dienst
  starten. Kein Container, keine Laufzeit.
- **Broker: Mosquitto, nativ.** Laeuft direkt auf dem Gateway (`mosquitto -c ...`);
  die optionale `docker-compose.yml` ist nur eine Bequemlichkeit fuer die Workstation.

Protokoll-Details:

- **MQTT** mit hierarchischen Topics und Wildcards (`+` / `#`), sodass neue
  Geraete ohne zentrale Konfiguration erkannt werden.
- **Retained Messages** halten den aktuellen Zustand (Registry, Online-Status,
  Raeume), sodass spaet startende Komponenten sofort den gesamten Bestand sehen.
- **Last-Will (LWT):** Stirbt ein Geraet, meldet der Broker es automatisch als
  offline; das ist die Grundlage der Ausfallerkennung.
- **Cloud vs. Edge:** Bewusst rein lokal/Edge. Sensorik, Regelauswertung und
  Alarmierung laufen auf dem Hub; eine Cloud-Schicht bleibt out of scope.

## 3. Interoperabilitaet & ein gemeinsamer Vertrag

- **Heterogenitaet:** Fuenf Geraetetypen ueber vier Funkstandards (Zigbee,
  Z-Wave, Wi-Fi, BLE) mit realistischen Hersteller-/Modelldaten, hinter **einem
  gemeinsamen Datenmodell**. Der Wire-Vertrag (camelCase-Felder, `kind`-
  Diskriminator im Geraetezustand) ist einmal in der Rust-Crate `shared`
  definiert und wird von den Firmware-Serialisierern in C gespiegelt. Das ist
  konzeptionell dieselbe Idee wie Matter: eine gemeinsame Schicht ueber
  heterogenen Funktechniken.
- **Onboarding wie bei echter Hardware:** Neue Geraete starten unprovisioniert
  (nur Serien-Nr. + Setup-PIN) im **Pairing-Modus**. Hub bzw. Dashboard nehmen sie
  in Betrieb (**Commissioning**: ID und Raum werden zugewiesen). Das Geraet
  persistiert seine Identitaet im **NVS** und bootet neu in den Normalbetrieb.
  Angelehnt an Zigbee/Z-Wave-Inclusion bzw. Matter-Setup-Codes.

## 4. Wie die zentralen Anforderungen geloest wurden

| Anforderung | Umsetzung im Prototyp |
|---|---|
| **Verteilung** | Eigenstaendige Prozesse (Hub, Dashboard, je Geraet eine ESP32-Firmware), gekoppelt nur ueber den Broker; Logik laeuft lokal/verteilt, nicht in der Cloud. |
| **Skalierbarkeit** | Geraete melden sich selbst an; das 1. wie das 100. Geraet ist derselbe Vorgang. Durch Topics pro Geraet (`.../<id>/...`) verteilt der Broker die Nachrichten von selbst. |
| **Fehlertoleranz** | Last-Will + retained Presence wandeln einen Absturz in einen `device_offline`-Alarm; ein Reconnect loest ihn automatisch wieder auf. Der Hub stellt seinen Zustand beim Neustart aus Dateien wieder her, das Geraet aus dem NVS. |
| **Robustheit** | Automatischer Reconnect (esp-mqtt bzw. rumqttc); fehlerhafte Nachrichten werden ignoriert, nicht weitergereicht; Komponenten laufen unabhaengig weiter, wenn eine ausfaellt. Rust gibt zusaetzlich Speichersicherheit ohne GC. |
| **Lose Kopplung** | Einzige Schnittstelle ist JSON ueber MQTT (plus kleine Dateien/NVS). Dashboard, ein Geraet oder sogar der Hub koennen jederzeit neu gestartet werden. |
| **Heterogenitaet** | Verschiedene Geraetetypen, Funkstandards und Energiequellen hinter einem einheitlichen Nachrichtenformat. |
| **Erweiterbarkeit** | Neuer Geraetetyp = ein Katalogeintrag + ein kleiner Zweig in der portablen Sensorlogik (+ ggf. eine Regel im Hub). Hub und Dashboard brauchen keine Aenderung, da sie generisch ueber das Datenmodell arbeiten. |

## 5. Bewusste Grenzen

Keine Cloud-Schicht; **Sicherheit/Identitaet** nur skizziert (anonymer Broker,
kein TLS - der Commissioning-Pfad waere der Ort fuer Authentifizierung sowie pro
Geraet TLS + ACLs auf dem Broker); nur ein einzelner Broker und Hub.

## 6. Verifikation

Die **portable C-Sensorlogik** (Zustandsmaschinen + Telemetrie-JSON) ist
unabhaengig von ESP-IDF und wird mit gcc getestet
(`firmware/host-test/build.sh`): Zustaende bleiben im gueltigen Bereich, jedes
JSON-Fragment ist wohlgeformt und traegt den korrekten `kind`-Diskriminator,
Kommandos werden korrekt angewandt, der Katalog ist vollstaendig. Der Wire-Vertrag
ist identisch zur Rust-Crate `shared` gehalten (gleiche Topics, gleiche
camelCase-Felder), sodass Firmware und Hub nicht auseinanderlaufen.

> Bedienung/Start: siehe `README.md` und `hub/README.md`.
