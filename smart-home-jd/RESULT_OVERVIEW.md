# Ergebnisübersicht — Smart-Home-Demonstrator (Assisted Living)

Kurze Einordnung, **wie** die in der Aufgabenstellung geforderten Überlegungen im
Prototyp umgesetzt wurden. Der Demonstrator simuliert ein Assisted-Living-Zuhause
mit **Docker + Mosquitto + MQTT** und einem **zentralisierten** Modell: alle Geräte
kommunizieren über den Broker mit einem zentralen **Hub** (Registry, Regeln,
Alarme).

---

## 1. Recherche-Grundlage

Bestehende Smart-Home-Systeme sind **zentral, dezentral oder hybrid** aufgebaut.
Offene Plattformen (Home Assistant, openHAB, ioBroker, Node-RED) integrieren
heterogene Geräte über Adapter. Auf Kommunikationsebene dominieren **MQTT**
(Broker-basiertes Publish/Subscribe) und CoAP; auf Netzwerkebene **Zigbee, Z-Wave,
Thread, Wi-Fi, BLE**, zunehmend **Matter** als IP-basierte Interoperabilitätsschicht.
Aktueller Trend ist die Aufteilung in **Cloud** (Fernzugriff, Verwaltung) und
**Edge** (lokale Logik, Latenz, Ausfallsicherheit). Daraus ergeben sich die
zentralen Anforderungen: **Verteilung, Skalierbarkeit, Fehlertoleranz, Robustheit,
lose Kopplung, Heterogenität und Erweiterbarkeit.**

## 2. Architektur & Kommunikationsmodell

Umgesetzt wurde das Modell **Gerät → Gateway/Hub → Automatisierung** (zentralisiert,
Edge-basiert). Kein Gerät spricht ein anderes direkt an — die gesamte Kommunikation
läuft als **Publish/Subscribe über den MQTT-Broker** (Pattern aus der Vorlesung):

- **Protokoll:** MQTT mit hierarchischen Topics und Wildcards (`+` / `#`) — so
  werden neue Geräte ohne zentrale Konfiguration erkannt.
- **Retained Messages** halten den aktuellen Zustand (Registry, Online-Status,
  Räume), sodass spät startende Komponenten sofort den gesamten Bestand sehen.
- **Last-Will (LWT):** Stirbt ein Geräteprozess, meldet der Broker es automatisch
  als „offline“ — Grundlage der Ausfallerkennung.
- **Cloud vs. Edge:** Bewusst **rein lokal/Edge**. Sensorik, Regelauswertung und
  Alarmierung laufen auf dem Hub; eine Cloud-Schicht (Fernzugriff, Flottenverwaltung)
  bleibt bewusst außen vor (out of scope).

## 3. Interoperabilität & aktuelle Trends

- **Heterogenität:** Fünf Gerätetypen über vier Funkstandards (Zigbee, Z-Wave,
  Wi-Fi, BLE) mit realistischen Hersteller-/Modelldaten, hinter **einem gemeinsamen
  Datenmodell** (`src/shared/types.ts`) — analog zur Idee von Matter (gemeinsame,
  IP-basierte Schicht über heterogenen Funktechniken).
- **Onboarding wie bei echter Hardware:** Neue Geräte starten im
  **Auslieferungszustand** (noch ohne ID/Raum) im **Pairing-Modus**. Hub bzw. App
  nehmen sie anschließend in Betrieb (**Commissioning**: ID und Raum werden
  zugewiesen) — angelehnt an Zigbee/Z-Wave-Inclusion bzw. Matter-Setup-Codes.

## 4. Wie die zentralen Anforderungen gelöst wurden

| Anforderung | Umsetzung im Prototyp |
|---|---|
| **Verteilung** | Eigenständige Prozesse (Hub, Sensoren, Web-UI), gekoppelt nur über den Broker; Logik läuft lokal/verteilt, nicht in der Cloud. |
| **Skalierbarkeit** | Geräte registrieren sich selbst; das 1. wie das 100. Gerät ist derselbe Vorgang. Durch Topics pro Gerät (`.../<id>/...`) verteilt der Broker die Nachrichten von selbst. |
| **Fehlertoleranz** | Last-Will + retained Presence wandeln einen Absturz in einen `device_offline`-Alarm; ein Reconnect löst ihn automatisch wieder auf. Der Hub stellt seinen Zustand beim Neustart aus Dateien wieder her. |
| **Robustheit** | Automatischer Reconnect (alle 2 s); fehlerhafte Nachrichten werden protokolliert, aber nicht weitergereicht; Komponenten laufen unabhängig weiter, wenn eine ausfällt. |
| **Lose Kopplung** | Einzige Schnittstelle ist JSON über MQTT (plus kleine Dateien). Web-UI, ein Sensor oder sogar der Hub können jederzeit neu gestartet werden. |
| **Heterogenität** | Verschiedene Gerätetypen, Funkstandards und Energiequellen hinter einem einheitlichen Nachrichtenformat. |
| **Erweiterbarkeit** | Neuer Gerätetyp = ein Katalogeintrag + kleine Sensorklasse (+ ggf. eine Regel). Der **SOS-Notfallknopf** wurde exakt so ergänzt, ohne Änderungen an Registry, Konsole oder UI. |

## 5. Bewusste Grenzen

Keine Cloud-Schicht; **Sicherheit/Identität** nur skizziert (anonymer Broker, kein
TLS — der Commissioning-Pfad wäre der Ort für die Authentifizierung); die Geräte
sind simuliert (Funkstandards als Metadaten); nur ein einzelner Broker und Hub.

## 6. Verifikation

`pnpm run typecheck` und `pnpm run smoke` laufen durch; zusätzlich live gegen
Docker/Mosquitto geprüft: Pairing → Commissioning → Registry, Online/Offline per
Last-Will, die Regel- und Alarmketten (der SOS-Alarm bleibt aktiv, bis er manuell
quittiert wird) sowie die Uhr und der „Simulate night“-Schalter im Web-UI.

> Bedienung/Start: siehe `README.md`.
