//! Live status for linked things: Home Assistant states (entity links, and
//! the entities in linked areas) and Outline document titles and URLs,
//! read through the grain's optional `home_assistant` and `outline` slots.
//!
//! Cap'n Proto clients are not `Send`, so everything here runs on the RPC
//! LocalSet; HTTP handlers reach it through `LiveHandle`'s channel. Status is
//! never part of layout or the document: the viewer fetches it on its own,
//! after drawing, and a slow or missing bridge only means no status yet.
use crate::homeassistant_v1_capnp::homeassistant_v1 as ha;
use crate::outline_v1_capnp::outline_v1 as outline;
use crate::{App, load_snapshot};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// Home Assistant areas: id (and lowercase name) → (display name, entity ids).
type AreaMap = BTreeMap<String, (String, Vec<String>)>;
/// One row of Home Assistant's entity registry, as far as status cares.
#[derive(Clone, Debug)]
struct RegEntry {
    id: String,
    name: String,
    category: String,
    disabled: bool,
}
/// Home Assistant devices: device id → its entities.
type DeviceMap = BTreeMap<String, Vec<RegEntry>>;
/// Slot names, as declared at registration.
pub const HA_SLOT: &str = "home_assistant";
pub const OUTLINE_SLOT: &str = "outline";
const STATE_TTL: Duration = Duration::from_secs(10);
const AREA_TTL: Duration = Duration::from_secs(600);
const DOC_TTL: Duration = Duration::from_secs(3600);

pub enum Request {
    Status(oneshot::Sender<Value>),
    Documents(oneshot::Sender<Value>),
}
/// The HTTP side's handle: send a request, await the reply.
#[derive(Clone)]
pub struct LiveHandle(mpsc::UnboundedSender<Request>);
pub fn channel() -> (LiveHandle, mpsc::UnboundedReceiver<Request>) {
    let (tx, rx) = mpsc::unbounded_channel();
    (LiveHandle(tx), rx)
}
impl LiveHandle {
    pub async fn ask(&self, make: impl FnOnce(oneshot::Sender<Value>) -> Request) -> Value {
        let (tx, rx) = oneshot::channel();
        if self.0.send(make(tx)).is_err() {
            return json!({ "connected": false });
        }
        rx.await.unwrap_or_else(|_| json!({ "connected": false }))
    }
}

/// Which provider names mean Home Assistant and Outline in external links.
fn is_ha(provider: &str) -> bool {
    matches!(provider, "home_assistant" | "homeassistant" | "ha")
}

#[derive(Default)]
pub struct Live {
    self_cap: RefCell<Option<darude_runtime::self_::Client>>,
    ha: RefCell<Option<ha::Client>>,
    outline: RefCell<Option<outline::Client>>,
    areas: RefCell<Option<(Instant, AreaMap)>>,
    devices: RefCell<Option<(Instant, DeviceMap)>>,
    status: RefCell<Option<(Instant, Value)>>,
    docs: RefCell<BTreeMap<String, (Instant, Value)>>,
}
impl Live {
    /// Take the bindings from a session's requires (pulled from Self).
    pub fn adopt(&self, requires: &darude_runtime::sessions::SessionRequires) {
        let ha: Option<ha::Client> = requires.get(HA_SLOT);
        let outline: Option<outline::Client> = requires.get(OUTLINE_SLOT);
        let changed = ha.is_some() != self.ha.borrow().is_some()
            || outline.is_some() != self.outline.borrow().is_some();
        *self.ha.borrow_mut() = ha;
        *self.outline.borrow_mut() = outline;
        if changed {
            // A new binding: forget anything learned without it.
            *self.status.borrow_mut() = None;
            *self.areas.borrow_mut() = None;
            *self.devices.borrow_mut() = None;
            self.docs.borrow_mut().clear();
        }
    }
    pub fn keep_self(&self, sc: darude_runtime::self_::Client) {
        *self.self_cap.borrow_mut() = Some(sc);
    }
    /// Re-pull the bindings after a wiring change.
    pub async fn repull(self: Rc<Self>) {
        let Some(sc) = self.self_cap.borrow().clone() else {
            return;
        };
        let mut requires = darude_runtime::sessions::SessionRequires::default();
        requires.pull_from_self(&sc).await;
        self.adopt(&requires);
    }

    /// Serve the HTTP side's requests; run on the RPC LocalSet.
    pub fn run(self: &Rc<Self>, app: App, mut rx: mpsc::UnboundedReceiver<Request>) {
        let live = self.clone();
        tokio::task::spawn_local(async move {
            while let Some(req) = rx.recv().await {
                let live = live.clone();
                let app = app.clone();
                // Each request runs on its own; a slow bridge never queues others.
                tokio::task::spawn_local(async move {
                    match req {
                        Request::Status(reply) => {
                            let _ = reply.send(live.status(&app).await);
                        }
                        Request::Documents(reply) => {
                            let _ = reply.send(live.documents(&app).await);
                        }
                    }
                });
            }
        });
    }

    async fn area_map(&self, client: &ha::Client) -> Result<AreaMap, String> {
        if let Some((at, map)) = self.areas.borrow().as_ref()
            && at.elapsed() < AREA_TTL
        {
            return Ok(map.clone());
        }
        let resp = client
            .list_areas_request()
            .send()
            .promise
            .await
            .map_err(|e| e.to_string())?;
        let mut map = BTreeMap::new();
        for a in resp
            .get()
            .map_err(|e| e.to_string())?
            .get_areas()
            .map_err(|e| e.to_string())?
            .iter()
        {
            let text = |t: capnp::Result<capnp::text::Reader<'_>>| {
                t.ok()
                    .and_then(|t| t.to_str().ok())
                    .unwrap_or("")
                    .to_string()
            };
            let id = text(a.get_id());
            let name = text(a.get_name());
            let entities: Vec<String> = a
                .get_entity_ids()
                .map(|l| {
                    l.iter()
                        .filter_map(|t| t.ok().and_then(|t| t.to_str().ok()).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            map.insert(name.to_lowercase(), (name.clone(), entities.clone()));
            map.insert(id, (name, entities));
        }
        *self.areas.borrow_mut() = Some((Instant::now(), map.clone()));
        Ok(map)
    }

    /// The entity registry grouped by device, cached like areas. Disabled
    /// rows are kept so the viewer can say what could be turned on.
    async fn device_map(&self, client: &ha::Client) -> Result<DeviceMap, String> {
        if let Some((at, map)) = self.devices.borrow().as_ref()
            && at.elapsed() < AREA_TTL
        {
            return Ok(map.clone());
        }
        let mut req = client.list_entities_request();
        req.get().init_req().set_include_disabled(true);
        let resp = req.send().promise.await.map_err(|e| e.to_string())?;
        let mut map: DeviceMap = BTreeMap::new();
        for r in resp
            .get()
            .map_err(|e| e.to_string())?
            .get_entries()
            .map_err(|e| e.to_string())?
            .iter()
        {
            let text = |t: capnp::Result<capnp::text::Reader<'_>>| {
                t.ok()
                    .and_then(|t| t.to_str().ok())
                    .unwrap_or("")
                    .to_string()
            };
            let device = text(r.get_device_id());
            if device.is_empty() {
                continue;
            }
            map.entry(device).or_default().push(RegEntry {
                id: text(r.get_entity_id()),
                name: text(r.get_name()),
                category: text(r.get_category()),
                disabled: r.get_disabled() || r.get_hidden(),
            });
        }
        *self.devices.borrow_mut() = Some((Instant::now(), map.clone()));
        Ok(map)
    }

    /// Current states for every linked entity and every entity in a linked
    /// area, cached briefly so many viewers cost one bridge call.
    async fn status(&self, app: &App) -> Value {
        let Some(client) = self.ha.borrow().clone() else {
            return json!({ "connected": false });
        };
        if let Some((at, v)) = self.status.borrow().as_ref()
            && at.elapsed() < STATE_TTL
        {
            return v.clone();
        }
        let value = match self.fetch_status(app, &client).await {
            Ok(v) => v,
            Err(e) => json!({ "connected": true, "error": e }),
        };
        *self.status.borrow_mut() = Some((Instant::now(), value.clone()));
        value
    }
    async fn fetch_status(&self, app: &App, client: &ha::Client) -> Result<Value, String> {
        let doc = load_snapshot(app).map_err(|e| e.1)?.document;
        let mut entity_ids = BTreeSet::new();
        let mut area_ids = BTreeSet::new();
        let mut device_ids = BTreeSet::new();
        for l in doc
            .entities
            .iter()
            .flat_map(|e| &e.links)
            .filter(|l| is_ha(&l.provider))
        {
            match l.kind.as_str() {
                "area" => {
                    area_ids.insert(l.external_id.clone());
                }
                _ if is_device_id(&l.kind, &l.external_id) => {
                    device_ids.insert(l.external_id.clone());
                }
                _ => {
                    entity_ids.insert(l.external_id.clone());
                }
            }
        }
        let mut areas = serde_json::Map::new();
        if !area_ids.is_empty() {
            let map = self.area_map(client).await?;
            for id in &area_ids {
                if let Some((name, members)) = map.get(id).or_else(|| map.get(&id.to_lowercase())) {
                    entity_ids.extend(members.iter().cloned());
                    areas.insert(id.clone(), json!({ "name": name, "entities": members }));
                }
            }
        }
        let mut devices = serde_json::Map::new();
        if !device_ids.is_empty() {
            let map = self.device_map(client).await?;
            for id in &device_ids {
                let Some(rows) = map.get(id) else { continue };
                let enabled: Vec<&RegEntry> = rows.iter().filter(|r| !r.disabled).collect();
                entity_ids.extend(enabled.iter().map(|r| r.id.clone()));
                devices.insert(id.clone(), device_json(rows));
            }
        }
        let mut entities = serde_json::Map::new();
        if !entity_ids.is_empty() {
            let mut req = client.get_states_request();
            {
                let mut q = req.get().init_req();
                q.set_limit(2000);
                let mut ids = q.init_entity_ids(entity_ids.len() as u32);
                for (i, id) in entity_ids.iter().enumerate() {
                    ids.set(i as u32, id);
                }
            }
            let resp = req.send().promise.await.map_err(|e| e.to_string())?;
            let list = resp
                .get()
                .and_then(|r| r.get_resp())
                .map_err(|e| e.to_string())?;
            for s in list.get_states().map_err(|e| e.to_string())?.iter() {
                let text = |t: capnp::Result<capnp::text::Reader<'_>>| {
                    t.ok()
                        .and_then(|t| t.to_str().ok())
                        .unwrap_or("")
                        .to_string()
                };
                let attrs: Value =
                    serde_json::from_str(&text(s.get_attributes_json())).unwrap_or(json!({}));
                entities.insert(
                    text(s.get_entity_id()),
                    json!({
                        "state": text(s.get_state()),
                        "name": text(s.get_friendly_name()),
                        "changed": s.get_last_changed_millis(),
                        "attributes": small_attributes(&attrs),
                    }),
                );
            }
        }
        Ok(json!({
            "connected": true,
            "updated": crate::now() * 1000,
            "entities": entities,
            "areas": areas,
            "devices": devices,
        }))
    }

    /// Titles and URLs for linked Outline documents, cached for an hour.
    async fn documents(&self, app: &App) -> Value {
        let Some(client) = self.outline.borrow().clone() else {
            return json!({ "connected": false });
        };
        let Ok(snapshot) = load_snapshot(app) else {
            return json!({ "connected": true, "error": "storage" });
        };
        let ids: BTreeSet<String> = snapshot
            .document
            .entities
            .iter()
            .flat_map(|e| &e.links)
            .filter(|l| l.provider == "outline")
            .map(|l| l.external_id.clone())
            .collect();
        let stale: Vec<String> = ids
            .iter()
            .filter(|id| {
                self.docs
                    .borrow()
                    .get(*id)
                    .is_none_or(|(at, _)| at.elapsed() >= DOC_TTL)
            })
            .cloned()
            .collect();
        // A few at a time: Outline answers each document separately.
        for batch in stale.chunks(8) {
            let calls = batch.iter().map(|id| {
                let mut req = client.get_document_request();
                req.get().set_id(id);
                let id = id.clone();
                async move {
                    let v = match req.send().promise.await {
                        Ok(resp) => match resp.get().and_then(|r| r.get_doc()) {
                            Ok(d) => {
                                let text = |t: capnp::Result<capnp::text::Reader<'_>>| {
                                    t.ok()
                                        .and_then(|t| t.to_str().ok())
                                        .unwrap_or("")
                                        .to_string()
                                };
                                json!({ "title": text(d.get_title()), "url": text(d.get_url()) })
                            }
                            Err(e) => json!({ "error": e.to_string() }),
                        },
                        Err(e) => json!({ "error": e.to_string() }),
                    };
                    (id, v)
                }
            });
            for (id, v) in futures::future::join_all(calls).await {
                self.docs.borrow_mut().insert(id, (Instant::now(), v));
            }
        }
        let docs: serde_json::Map<String, Value> = ids
            .iter()
            .filter_map(|id| {
                self.docs
                    .borrow()
                    .get(id)
                    .map(|(_, v)| (id.clone(), v.clone()))
            })
            .collect();
        json!({ "connected": true, "documents": docs })
    }
}

/// A device link: kind "device", or a bare registry id (32 hex digits).
fn is_device_id(kind: &str, id: &str) -> bool {
    kind == "device" || (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The port number an entity belongs to, from "…_port_8_poe" or "Port 8 PoE".
fn port_number(entry: &RegEntry) -> Option<u32> {
    let find = |s: &str, marker: &str| {
        let lower = s.to_lowercase();
        let at = lower.find(marker)? + marker.len();
        let digits: String = lower[at..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let rest = lower[at + digits.len()..].chars().next();
        // "port_8_poe" or "port 8" at the end; never "port_80x".
        if digits.is_empty() || rest.is_some_and(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        digits.parse().ok()
    };
    find(&entry.id, "_port_").or_else(|| find(&entry.name, "port "))
}

/// A device for the viewer: its entities (enabled, with category), those
/// per port, and the disabled ones that could be turned on in HA.
fn device_json(rows: &[RegEntry]) -> Value {
    let mut entities = Vec::new();
    let mut ports: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut disabled = Vec::new();
    for r in rows {
        if r.disabled {
            disabled.push(json!(r.id));
            continue;
        }
        entities.push(json!({ "id": r.id, "category": r.category }));
        if let Some(n) = port_number(r) {
            ports.entry(n.to_string()).or_default().push(r.id.clone());
        }
    }
    json!({ "entities": entities, "ports": ports, "disabled": disabled })
}

/// The attributes worth showing: scalars, not pictures or long lists.
fn small_attributes(attrs: &Value) -> Value {
    const SKIP: [&str; 6] = [
        "friendly_name",
        "entity_picture",
        "icon",
        "supported_features",
        "attribution",
        "editable",
    ];
    let mut out = serde_json::Map::new();
    if let Some(map) = attrs.as_object() {
        for (k, v) in map {
            if SKIP.contains(&k.as_str()) || out.len() >= 16 {
                continue;
            }
            let keep = match v {
                Value::String(s) => s.len() <= 80,
                Value::Number(_) | Value::Bool(_) => true,
                _ => false,
            };
            if keep {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use capnp::capability::Promise;
    use std::cell::Cell;
    use stufftracker_core::ExternalLink;

    /// A Home Assistant with one area (the kitchen: a light and a
    /// thermometer) and a door sensor; counts getStates calls.
    struct FakeHa(Rc<Cell<u32>>);
    impl ha::Server for FakeHa {
        fn list_areas(
            &mut self,
            _: ha::ListAreasParams,
            mut r: ha::ListAreasResults,
        ) -> Promise<(), capnp::Error> {
            let mut a = r.get().init_areas(1).get(0);
            a.set_id("kitchen");
            a.set_name("Kitchen");
            let mut ids = a.init_entity_ids(2);
            ids.set(0, "light.pendants");
            ids.set(1, "sensor.kitchen_temp");
            Promise::ok(())
        }
        fn list_entities(
            &mut self,
            _: ha::ListEntitiesParams,
            mut r: ha::ListEntitiesResults,
        ) -> Promise<(), capnp::Error> {
            let rows = [
                ("switch.sw_port_3_poe", "Port 3 PoE", "", false),
                ("sensor.sw_port_3_poe_power", "Port 3 PoE Power", "", false),
                ("sensor.sw_port_3_rx", "Port 3 RX", "diagnostic", true),
                ("sensor.sw_cpu", "CPU", "diagnostic", false),
                ("sensor.sw_port_30x", "Port 30x", "", false),
            ];
            let mut list = r.get().init_entries(rows.len() as u32);
            for (i, (id, name, category, disabled)) in rows.iter().enumerate() {
                let mut e = list.reborrow().get(i as u32);
                e.set_entity_id(id);
                e.set_name(name);
                e.set_device_id("0123456789abcdef0123456789abcdef");
                e.set_category(category);
                e.set_disabled(*disabled);
            }
            Promise::ok(())
        }
        fn get_states(
            &mut self,
            p: ha::GetStatesParams,
            mut r: ha::GetStatesResults,
        ) -> Promise<(), capnp::Error> {
            self.0.set(self.0.get() + 1);
            let want: Vec<String> = p
                .get()
                .unwrap()
                .get_req()
                .unwrap()
                .get_entity_ids()
                .unwrap()
                .iter()
                .map(|t| t.unwrap().to_string().unwrap())
                .collect();
            let all = [
                (
                    "light.pendants",
                    "on",
                    r#"{"brightness":200,"friendly_name":"Pendants"}"#,
                ),
                (
                    "sensor.kitchen_temp",
                    "21.4",
                    r#"{"unit_of_measurement":"°C","device_class":"temperature"}"#,
                ),
                (
                    "binary_sensor.back_door",
                    "off",
                    r#"{"device_class":"door","entity_picture":"/x"}"#,
                ),
                ("switch.sw_port_3_poe", "on", "{}"),
                (
                    "sensor.sw_port_3_poe_power",
                    "4.2",
                    r#"{"unit_of_measurement":"W","device_class":"power"}"#,
                ),
                ("sensor.sw_cpu", "12", "{}"),
            ];
            let found: Vec<_> = all
                .iter()
                .filter(|(id, ..)| want.iter().any(|w| w == id))
                .collect();
            let mut list = r.get().init_resp().init_states(found.len() as u32);
            for (i, (id, state, attrs)) in found.iter().enumerate() {
                let mut s = list.reborrow().get(i as u32);
                s.set_entity_id(id);
                s.set_state(state);
                s.set_attributes_json(attrs);
                s.set_last_changed_millis(1_700_000_000_000);
            }
            Promise::ok(())
        }
    }
    struct FakeOutline;
    impl outline::Server for FakeOutline {
        fn get_document(
            &mut self,
            p: outline::GetDocumentParams,
            mut r: outline::GetDocumentResults,
        ) -> Promise<(), capnp::Error> {
            let id = p.get().unwrap().get_id().unwrap().to_string().unwrap();
            let mut d = r.get().init_doc();
            d.set_id(&id);
            d.set_title(format!("Doc {id}"));
            d.set_url(format!("https://outline.example/doc/{id}"));
            Promise::ok(())
        }
    }
    fn app_with_links() -> App {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::initialize(&mut db).unwrap();
        let app = App {
            db: std::sync::Arc::new(std::sync::Mutex::new(db)),
            grain: true,
            live: None,
        };
        let mut s = load_snapshot(&app).unwrap();
        let link = |provider: &str, kind: &str, id: &str| ExternalLink {
            provider: provider.into(),
            kind: kind.into(),
            external_id: id.into(),
        };
        for e in &mut s.document.entities {
            e.links = match e.id.as_str() {
                "kitchen" => vec![
                    link("home_assistant", "area", "kitchen"),
                    link("outline", "document", "k1"),
                ],
                "sensor" => vec![link("home_assistant", "entity", "binary_sensor.back_door")],
                "panel" => vec![
                    link("outline", "document", "p1"),
                    link(
                        "home_assistant",
                        "device",
                        "0123456789abcdef0123456789abcdef",
                    ),
                ],
                _ => Vec::new(),
            };
        }
        crate::store_snapshot(&app, s).unwrap();
        app
    }
    #[tokio::test(flavor = "current_thread")]
    async fn status_and_documents_come_from_the_bound_bridges() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let app = app_with_links();
                let live = Rc::new(Live::default());
                // Not wired yet: a plain answer, no error.
                assert_eq!(live.status(&app).await, json!({ "connected": false }));
                assert_eq!(live.documents(&app).await, json!({ "connected": false }));
                let calls = Rc::new(Cell::new(0));
                *live.ha.borrow_mut() = Some(capnp_rpc::new_client(FakeHa(calls.clone())));
                *live.outline.borrow_mut() = Some(capnp_rpc::new_client(FakeOutline));
                let s = live.status(&app).await;
                assert_eq!(s["connected"], true);
                // The linked entity, and both entities of the linked area.
                assert_eq!(s["entities"]["binary_sensor.back_door"]["state"], "off");
                assert_eq!(
                    s["entities"]["sensor.kitchen_temp"]["attributes"]["unit_of_measurement"],
                    "°C"
                );
                assert_eq!(s["entities"]["light.pendants"]["state"], "on");
                assert_eq!(s["areas"]["kitchen"]["name"], "Kitchen");
                assert!(
                    s["entities"]["binary_sensor.back_door"]["attributes"]
                        .get("entity_picture")
                        .is_none()
                );
                // A device link brings its enabled entities, grouped by port.
                let dev = &s["devices"]["0123456789abcdef0123456789abcdef"];
                assert_eq!(
                    dev["ports"],
                    json!({ "3": ["switch.sw_port_3_poe", "sensor.sw_port_3_poe_power"] })
                );
                assert_eq!(dev["disabled"], json!(["sensor.sw_port_3_rx"]));
                assert_eq!(
                    dev["entities"][2],
                    json!({ "id": "sensor.sw_cpu", "category": "diagnostic" })
                );
                assert_eq!(s["entities"]["sensor.sw_port_3_poe_power"]["state"], "4.2");
                // A second ask within the TTL is served from cache.
                live.status(&app).await;
                assert_eq!(calls.get(), 1);
                let d = live.documents(&app).await;
                assert_eq!(d["documents"]["k1"]["title"], "Doc k1");
                assert_eq!(
                    d["documents"]["p1"]["url"],
                    "https://outline.example/doc/p1"
                );
            })
            .await;
    }

    #[test]
    fn only_small_scalar_attributes_are_kept() {
        let a = small_attributes(&json!({
            "unit_of_measurement": "°C",
            "device_class": "temperature",
            "brightness": 128,
            "is_on": true,
            "friendly_name": "Kitchen",
            "entity_picture": "/api/x.png",
            "rgb_color": [255, 0, 0],
            "forecast": [{"t": 1}],
            "note": "x".repeat(200),
        }));
        assert_eq!(
            a,
            json!({ "unit_of_measurement": "°C", "device_class": "temperature", "brightness": 128, "is_on": true })
        );
    }
}
