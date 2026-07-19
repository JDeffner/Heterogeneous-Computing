//! Operator web console. An axum server that mirrors the MQTT bus to the browser
//! via Server-Sent Events. It holds no own truth: if it goes down, the hub and
//! the devices keep running (loose coupling). Control actions are published as
//! MQTT control messages that the hub acts upon.
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        Html,
    },
    routing::{delete, get, post},
    Json, Router,
};
use futures_core::Stream;
use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex};

use shared::util::now_iso;
use shared::{
    topics, Alarm, AlarmControl, CommissionRequest, DeviceControl, DeviceRecord, HubControl,
    HubStatus, PairingAd, Resident, Room, RoomControl, SensorMode, SensorState, SituationEvent,
    Telemetry,
};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView {
    announce: DeviceRecord,
    online: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_state: Option<SensorState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<SensorMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    battery: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rssi: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen: Option<String>,
}

#[derive(Default)]
struct AppState {
    devices: HashMap<String, DeviceView>,
    rooms: HashMap<String, Room>,
    alarms: HashMap<String, Alarm>,
    pairing: HashMap<String, PairingAd>,
    events: Vec<SituationEvent>,
    hub_status: Option<HubStatus>,
    resident: Option<Resident>,
}

struct App {
    state: Mutex<AppState>,
    tx: broadcast::Sender<String>,
    client: AsyncClient,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot<'a> {
    devices: Vec<&'a DeviceView>,
    rooms: Vec<&'a Room>,
    alarms: Vec<&'a Alarm>,
    pairing: Vec<&'a PairingAd>,
    hub_status: Option<&'a HubStatus>,
    resident: Option<&'a Resident>,
    events: Vec<&'a SituationEvent>,
    ts: String,
}

fn room_name_of(d: &DeviceView) -> &str {
    d.announce.location.as_ref().map(|l| l.room.as_str()).unwrap_or("")
}

fn snapshot_json(s: &AppState) -> String {
    let mut devices: Vec<&DeviceView> = s.devices.values().collect();
    devices.sort_by(|a, b| room_name_of(a).cmp(room_name_of(b)));
    let mut rooms: Vec<&Room> = s.rooms.values().collect();
    rooms.sort_by(|a, b| a.room.cmp(&b.room));
    let mut alarms: Vec<&Alarm> = s.alarms.values().collect();
    alarms.sort_by(|a, b| b.raised_at.cmp(&a.raised_at));
    let mut pairing: Vec<&PairingAd> = s.pairing.values().collect();
    pairing.sort_by(|a, b| a.advertised_at.cmp(&b.advertised_at));
    let events: Vec<&SituationEvent> = s.events.iter().rev().take(25).collect();

    let snap = Snapshot {
        devices,
        rooms,
        alarms,
        pairing,
        hub_status: s.hub_status.as_ref(),
        resident: s.resident.as_ref(),
        events,
        ts: now_iso(),
    };
    serde_json::to_string(&snap).unwrap_or_else(|_| "{}".to_string())
}

fn parse_broker(url: &str) -> (String, u16) {
    let stripped = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("tcp://"))
        .unwrap_or(url);
    let stripped = stripped.split('/').next().unwrap_or(stripped);
    match stripped.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
        None => (stripped.to_string(), 1883),
    }
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::var("WEB_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3000);
    let broker = std::env::var("MQTT_URL").unwrap_or_else(|_| "mqtt://localhost:1883".into());
    let (host, mport) = parse_broker(&broker);

    let mut opts = MqttOptions::new("web-ui", host.as_str(), mport);
    opts.set_keep_alive(Duration::from_secs(30));
    let (client, mut eventloop) = AsyncClient::new(opts, 128);

    let (tx, _rx) = broadcast::channel::<String>(64);
    let app = Arc::new(App {
        state: Mutex::new(AppState::default()),
        tx,
        client: client.clone(),
    });

    for s in [
        topics::sub::all_registry(),
        topics::sub::all_presence(),
        topics::sub::all_telemetry(),
        topics::sub::all_pairing(),
        topics::sub::all_events(),
        topics::sub::all_alarms(),
        topics::sub::all_rooms(),
        topics::hub_status(),
        topics::resident(),
    ] {
        if let Err(e) = client.subscribe(s.clone(), QoS::AtLeastOnce).await {
            eprintln!("[web] subscribe {s} failed: {e}");
        }
    }

    // MQTT mirror task.
    {
        let app = app.clone();
        tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(MqttEvent::Incoming(Packet::Publish(p))) => {
                        handle_message(&app, &p.topic, &p.payload).await;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("[web] mqtt error: {e}; retrying in 2s");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        });
    }

    let router = Router::new()
        .route("/", get(index))
        .route("/api/state", get(get_state))
        .route("/api/stream", get(stream))
        .route("/api/alarms/:id/resolve", post(resolve_alarm))
        .route("/api/alarms/:id/ack", post(ack_alarm))
        .route("/api/alarms/:id/call-logged", post(call_logged))
        .route("/api/resident", post(update_resident))
        .route("/api/rooms", post(create_room))
        .route("/api/rooms/:room_id", delete(delete_room))
        .route("/api/devices/:id/assign", post(assign_device))
        .route("/api/devices/:id/command", post(command_device))
        .route("/api/devices/:id", delete(remove_device))
        .route("/api/commission", post(commission))
        .route("/api/hub/force-night", post(force_night))
        .with_state(app);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    println!("Operator console at http://localhost:{port}");
    axum::serve(listener, router).await.expect("serve");
}

async fn broadcast_snapshot(app: &Arc<App>) {
    let data = {
        let s = app.state.lock().await;
        snapshot_json(&s)
    };
    let _ = app.tx.send(data);
}

async fn handle_message(app: &Arc<App>, topic: &str, payload: &[u8]) {
    {
        let mut s = app.state.lock().await;

        // Presence is plain text.
        if topic.ends_with("/presence") {
            if let Some(id) = topics::device_id_from_topic(topic) {
                if !payload.is_empty() {
                    if let Some(d) = s.devices.get_mut(id) {
                        d.online = payload == b"online";
                    }
                }
            }
            drop(s);
            broadcast_snapshot(app).await;
            return;
        }

        // Pairing ads (retained; empty clears).
        if let Some(serial) = topics::serial_from_pairing(topic) {
            if payload.is_empty() {
                s.pairing.remove(serial);
            } else if let Ok(ad) = serde_json::from_slice::<PairingAd>(payload) {
                s.pairing.insert(serial.to_string(), ad);
            }
            drop(s);
            broadcast_snapshot(app).await;
            return;
        }

        // Empty payload = retained message cleared.
        if payload.is_empty() {
            if let Some(id) = topics::device_id_from_registry(topic) {
                s.devices.remove(id);
            } else if let Some(id) = topics::alarm_id_from_topic(topic) {
                s.alarms.remove(id);
            } else if let Some(id) = topics::room_id_from_topic(topic) {
                s.rooms.remove(id);
            }
            drop(s);
            broadcast_snapshot(app).await;
            return;
        }

        if let Some(_id) = topics::device_id_from_registry(topic) {
            if let Ok(rec) = serde_json::from_slice::<DeviceRecord>(payload) {
                let existing = s.devices.get(&rec.device_id);
                let online = existing
                    .map(|e| e.online)
                    .unwrap_or(matches!(rec.status, shared::Status::Online));
                let view = DeviceView {
                    online,
                    last_state: existing.and_then(|e| e.last_state.clone()),
                    mode: existing.and_then(|e| e.mode),
                    battery: existing.and_then(|e| e.battery),
                    rssi: existing.and_then(|e| e.rssi),
                    last_seen: existing.and_then(|e| e.last_seen.clone()),
                    announce: rec.clone(),
                };
                s.devices.insert(rec.device_id, view);
            }
        } else if topic.ends_with("/telemetry") {
            if let Ok(t) = serde_json::from_slice::<Telemetry>(payload) {
                if let Some(d) = s.devices.get_mut(&t.device_id) {
                    d.last_state = Some(t.state);
                    d.mode = Some(t.mode);
                    d.battery = Some(t.battery);
                    d.rssi = Some(t.rssi);
                    d.last_seen = Some(t.ts);
                }
            }
        } else if topics::room_id_from_topic(topic).is_some() {
            if let Ok(r) = serde_json::from_slice::<Room>(payload) {
                s.rooms.insert(r.room_id.clone(), r);
            }
        } else if topic.starts_with("smarthome/events/") {
            if let Ok(ev) = serde_json::from_slice::<SituationEvent>(payload) {
                s.events.push(ev);
                if s.events.len() > 200 {
                    s.events.remove(0);
                }
            }
        } else if topics::alarm_id_from_topic(topic).is_some() {
            if let Ok(al) = serde_json::from_slice::<Alarm>(payload) {
                s.alarms.insert(al.alarm_id.clone(), al);
            }
        } else if topic == topics::hub_status().as_str() {
            if let Ok(hs) = serde_json::from_slice::<HubStatus>(payload) {
                s.hub_status = Some(hs);
            }
        } else if topic == topics::resident().as_str() {
            if let Ok(r) = serde_json::from_slice::<Resident>(payload) {
                s.resident = Some(r);
            }
        }
    }
    broadcast_snapshot(app).await;
}

// ----- HTTP handlers -------------------------------------------------------

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn get_state(State(app): State<Arc<App>>) -> Json<Value> {
    let s = app.state.lock().await;
    Json(serde_json::from_str(&snapshot_json(&s)).unwrap_or(json!({})))
}

async fn stream(
    State(app): State<Arc<App>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = app.tx.subscribe();
    let initial = {
        let s = app.state.lock().await;
        snapshot_json(&s)
    };
    let s = async_stream::stream! {
        yield Ok(Event::default().data(initial));
        loop {
            match rx.recv().await {
                Ok(data) => yield Ok(Event::default().data(data)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(s).keep_alive(KeepAlive::default())
}

async fn pub_json<T: Serialize>(app: &Arc<App>, topic: String, payload: &T) {
    if let Ok(bytes) = serde_json::to_vec(payload) {
        let _ = app.client.publish(topic, QoS::AtLeastOnce, false, bytes).await;
    }
}

async fn resolve_alarm(State(app): State<Arc<App>>, Path(id): Path<String>) -> Json<Value> {
    let ctrl = AlarmControl { alarm_id: id, action: "resolve".into() };
    pub_json(&app, topics::alarm_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

async fn ack_alarm(State(app): State<Arc<App>>, Path(id): Path<String>) -> Json<Value> {
    let ctrl = AlarmControl { alarm_id: id, action: "ack".into() };
    pub_json(&app, topics::alarm_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

async fn call_logged(State(app): State<Arc<App>>, Path(id): Path<String>) -> Json<Value> {
    let ctrl = AlarmControl { alarm_id: id, action: "call_logged".into() };
    pub_json(&app, topics::alarm_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

async fn update_resident(State(app): State<Arc<App>>, Json(r): Json<Resident>) -> Json<Value> {
    pub_json(&app, topics::resident_control(), &r).await;
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct RoomBody {
    building: String,
    floor: String,
    room: String,
}

async fn create_room(State(app): State<Arc<App>>, Json(b): Json<RoomBody>) -> Json<Value> {
    let ctrl = RoomControl::Create {
        building: b.building,
        floor: b.floor,
        room: b.room,
    };
    pub_json(&app, topics::room_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

async fn delete_room(State(app): State<Arc<App>>, Path(room_id): Path<String>) -> Json<Value> {
    let ctrl = RoomControl::Delete { room_id };
    pub_json(&app, topics::room_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignBody {
    #[serde(default)]
    room_id: Option<String>,
}

async fn assign_device(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(b): Json<AssignBody>,
) -> Json<Value> {
    let ctrl = DeviceControl::Assign {
        device_id: id,
        room_id: b.room_id,
    };
    pub_json(&app, topics::device_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

async fn command_device(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Json<Value> {
    if let Ok(bytes) = serde_json::to_vec(&body) {
        let _ = app.client.publish(topics::command(&id), QoS::AtLeastOnce, false, bytes).await;
    }
    Json(json!({"ok": true}))
}

async fn remove_device(State(app): State<Arc<App>>, Path(id): Path<String>) -> Json<Value> {
    let ctrl = DeviceControl::Remove { device_id: id };
    pub_json(&app, topics::device_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommissionBody {
    serial: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    room_id: Option<String>,
}

async fn commission(State(app): State<Arc<App>>, Json(b): Json<CommissionBody>) -> Json<Value> {
    let ctrl = CommissionRequest {
        serial: b.serial,
        name: b.name,
        room_id: b.room_id,
    };
    pub_json(&app, topics::commission_control(), &ctrl).await;
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct NightBody {
    #[serde(default)]
    on: bool,
}

async fn force_night(State(app): State<Arc<App>>, Json(b): Json<NightBody>) -> Json<Value> {
    let ctrl = HubControl { force_night: Some(b.on) };
    pub_json(&app, topics::hub_control(), &ctrl).await;
    Json(json!({"ok": true}))
}
