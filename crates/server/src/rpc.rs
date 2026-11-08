//! App-owned Darude ABI v2 surface. No ambient HTTP credentials are used.
use crate::stufftracker_v1_capnp as wire;
use crate::{
    App, SavedView, Snapshot, load_snapshot, load_views, remove_view, store_snapshot, store_view,
};
use capnp::capability::{FromClientHook, Promise};
use std::rc::Rc;
use stufftracker_core::{
    Connection, Document, Endpoint, Entity, ExternalLink, Faceplate, Layer, Port, Protection, Role,
};
use wire::stuff_tracker_v1 as api;

fn error(e: crate::Error) -> capnp::Error {
    capnp::Error::failed(format!("{}: {}", e.0.as_u16(), e.1))
}
fn invalid(message: impl Into<String>) -> capnp::Error {
    capnp::Error::failed(format!("400: {}", message.into()))
}
fn text(value: capnp::Result<capnp::text::Reader<'_>>) -> capnp::Result<String> {
    Ok(value?
        .to_str()
        .map_err(|_| invalid("Invalid UTF-8"))?
        .into())
}
fn finish(result: capnp::Result<()>) -> Promise<(), capnp::Error> {
    match result {
        Ok(()) => Promise::ok(()),
        Err(e) => Promise::err(e),
    }
}
fn entity_read(r: wire::entity::Reader<'_>) -> capnp::Result<Entity> {
    let parent = text(r.get_parent())?;
    Ok(Entity {
        id: text(r.get_id())?,
        name: text(r.get_name())?,
        kind: text(r.get_kind())?,
        parent: if parent.is_empty() {
            None
        } else {
            Some(parent)
        },
        links: r
            .get_links()?
            .iter()
            .map(|l| {
                Ok(ExternalLink {
                    provider: text(l.get_provider())?,
                    kind: text(l.get_kind())?,
                    external_id: text(l.get_external_id())?,
                })
            })
            .collect::<capnp::Result<_>>()?,
        faceplate: faceplate_read(r.get_faceplate())?,
        ports: ports_read(r.get_ports()?)?,
    })
}
fn faceplate_read(
    f: Result<wire::Faceplate, capnp::NotInSchema>,
) -> capnp::Result<Option<Faceplate>> {
    Ok(match f.map_err(|_| invalid("Unknown faceplate"))? {
        wire::Faceplate::None => None,
        wire::Faceplate::Panel => Some(Faceplate::Panel),
        wire::Faceplate::Switch => Some(Faceplate::Switch),
        wire::Faceplate::Terminals => Some(Faceplate::Terminals),
    })
}
fn faceplate_wire(f: Option<Faceplate>) -> wire::Faceplate {
    match f {
        None => wire::Faceplate::None,
        Some(Faceplate::Panel) => wire::Faceplate::Panel,
        Some(Faceplate::Switch) => wire::Faceplate::Switch,
        Some(Faceplate::Terminals) => wire::Faceplate::Terminals,
    }
}
fn ports_read(list: capnp::struct_list::Reader<'_, wire::port::Owned>) -> capnp::Result<Vec<Port>> {
    list.iter()
        .map(|p| {
            let half = text(p.get_half())?;
            Ok(Port {
                id: text(p.get_id())?,
                label: text(p.get_label())?,
                position: Some(p.get_position()).filter(|v| *v > 0),
                poles: Some(p.get_poles() as u32).filter(|v| *v > 1),
                half: Some(half).filter(|h| !h.is_empty()),
                halves: p
                    .get_halves()?
                    .iter()
                    .map(text)
                    .collect::<capnp::Result<_>>()?,
                main: p.get_main(),
                amps: Some(p.get_amps() as u32).filter(|a| *a > 0),
                protection: match p
                    .get_protection()
                    .map_err(|_| invalid("Unknown protection"))?
                {
                    wire::Protection::None => None,
                    wire::Protection::Gfci => Some(Protection::Gfci),
                    wire::Protection::Afci => Some(Protection::Afci),
                    wire::Protection::Dual => Some(Protection::Dual),
                },
            })
        })
        .collect()
}
fn ports_write(mut list: capnp::struct_list::Builder<'_, wire::port::Owned>, ports: &[Port]) {
    for (i, p) in ports.iter().enumerate() {
        let mut b = list.reborrow().get(i as u32);
        b.set_id(&p.id);
        b.set_label(&p.label);
        b.set_position(p.position.unwrap_or(0));
        b.set_poles(p.poles.unwrap_or(1).min(255) as u8);
        b.set_half(p.half.as_deref().unwrap_or(""));
        b.set_main(p.main);
        b.set_amps(p.amps.unwrap_or(0).min(u16::MAX as u32) as u16);
        b.set_protection(match p.protection {
            None => wire::Protection::None,
            Some(Protection::Gfci) => wire::Protection::Gfci,
            Some(Protection::Afci) => wire::Protection::Afci,
            Some(Protection::Dual) => wire::Protection::Dual,
        });
        let mut halves = b.init_halves(p.halves.len() as u32);
        for (k, h) in p.halves.iter().enumerate() {
            halves.set(k as u32, h);
        }
    }
}
fn entity_write(mut b: wire::entity::Builder<'_>, e: &Entity) {
    b.set_id(&e.id);
    b.set_name(&e.name);
    b.set_kind(&e.kind);
    b.set_parent(e.parent.as_deref().unwrap_or(""));
    let mut links = b.reborrow().init_links(e.links.len() as u32);
    for (i, l) in e.links.iter().enumerate() {
        let mut b = links.reborrow().get(i as u32);
        b.set_provider(&l.provider);
        b.set_kind(&l.kind);
        b.set_external_id(&l.external_id);
    }
    b.set_faceplate(faceplate_wire(e.faceplate));
    ports_write(b.init_ports(e.ports.len() as u32), &e.ports);
}
fn connection_read(r: wire::connection::Reader<'_>) -> capnp::Result<Connection> {
    Ok(Connection {
        id: text(r.get_id())?,
        name: text(r.get_name())?,
        layer: text(r.get_layer())?,
        endpoints: r
            .get_endpoints()?
            .iter()
            .map(|ep| {
                let port = text(ep.get_port())?;
                Ok(Endpoint {
                    port: Some(port).filter(|p| !p.is_empty()),
                    entity: text(ep.get_entity())?,
                    role: match ep
                        .get_role()
                        .map_err(|_| invalid("Unknown endpoint role"))?
                    {
                        wire::Role::Source => Role::Source,
                        wire::Role::Target => Role::Target,
                        wire::Role::Peer => Role::Peer,
                    },
                })
            })
            .collect::<capnp::Result<_>>()?,
    })
}
fn connection_write(mut b: wire::connection::Builder<'_>, c: &Connection) {
    b.set_id(&c.id);
    b.set_name(&c.name);
    b.set_layer(&c.layer);
    let mut eps = b.init_endpoints(c.endpoints.len() as u32);
    for (i, ep) in c.endpoints.iter().enumerate() {
        let mut b = eps.reborrow().get(i as u32);
        b.set_entity(&ep.entity);
        b.set_port(ep.port.as_deref().unwrap_or(""));
        b.set_role(match ep.role {
            Role::Source => wire::Role::Source,
            Role::Target => wire::Role::Target,
            Role::Peer => wire::Role::Peer,
        });
    }
}
fn layer_read(r: wire::layer::Reader<'_>) -> capnp::Result<Layer> {
    Ok(Layer {
        id: text(r.get_id())?,
        name: text(r.get_name())?,
        color: text(r.get_color())?,
    })
}
fn layer_write(mut b: wire::layer::Builder<'_>, l: &Layer) {
    b.set_id(&l.id);
    b.set_name(&l.name);
    b.set_color(&l.color);
}
fn document_read(r: wire::document::Reader<'_>) -> capnp::Result<Document> {
    Ok(Document {
        entities: r
            .get_entities()?
            .iter()
            .map(entity_read)
            .collect::<capnp::Result<_>>()?,
        connections: r
            .get_connections()?
            .iter()
            .map(connection_read)
            .collect::<capnp::Result<_>>()?,
        layers: r
            .get_layers()?
            .iter()
            .map(layer_read)
            .collect::<capnp::Result<_>>()?,
    })
}
fn document_write(mut b: wire::document::Builder<'_>, d: &Document) {
    let mut list = b.reborrow().init_entities(d.entities.len() as u32);
    for (i, e) in d.entities.iter().enumerate() {
        entity_write(list.reborrow().get(i as u32), e);
    }
    let mut list = b.reborrow().init_connections(d.connections.len() as u32);
    for (i, c) in d.connections.iter().enumerate() {
        connection_write(list.reborrow().get(i as u32), c);
    }
    let mut list = b.init_layers(d.layers.len() as u32);
    for (i, l) in d.layers.iter().enumerate() {
        layer_write(list.reborrow().get(i as u32), l);
    }
}
fn view_write(mut b: wire::saved_view::Builder<'_>, v: &SavedView) -> capnp::Result<()> {
    b.set_id(&v.id);
    b.set_name(&v.name);
    b.set_view_json(serde_json::to_string(&v.view).map_err(|e| invalid(e.to_string()))?);
    Ok(())
}

struct Api {
    app: App,
    rights: Vec<String>,
}
impl Api {
    fn check(&self, write: bool) -> capnp::Result<()> {
        let allowed = darude_runtime::rights::allow_write(&self.rights)
            || (!write && darude_runtime::rights::allows(&self.rights, "read"));
        if allowed {
            Ok(())
        } else {
            Err(capnp::Error::failed(format!(
                "403: stufftracker.v1 requires {}",
                if write { "write" } else { "read" }
            )))
        }
    }
    fn snapshot(&self) -> capnp::Result<Snapshot> {
        self.check(false)?;
        load_snapshot(&self.app).map_err(error)
    }
    fn mutate(
        &self,
        revision: u64,
        change: impl FnOnce(&mut Document) -> capnp::Result<()>,
    ) -> capnp::Result<u64> {
        self.check(true)?;
        let mut s = load_snapshot(&self.app).map_err(error)?;
        if s.revision != revision {
            return Err(capnp::Error::failed(
                "409: Inventory changed; refetch its revision".into(),
            ));
        }
        change(&mut s.document)?;
        store_snapshot(&self.app, s).map_err(error)
    }
}
pub async fn serve(app: App) -> anyhow::Result<()> {
    use darude_runtime::sessions::{SessionFactory, SessionProvides, serve_sessions};
    let factory: SessionFactory = Rc::new(move |ctx| {
        let app = app.clone();
        Box::pin(async move {
            let cap: api::Client = capnp_rpc::new_client(Api {
                app,
                rights: ctx.rights,
            });
            Ok(SessionProvides::new().add_with_schema(
                "stufftracker.v1",
                cap.into_client_hook(),
                <api::Client as capnp::traits::HasTypeId>::TYPE_ID,
                include_bytes!(concat!(env!("OUT_DIR"), "/stufftracker-v1.schema.bin")).to_vec(),
            ))
        })
    });
    serve_sessions(factory).await
}

impl api::Server for Api {
    fn get_document(
        &mut self,
        params: api::GetDocumentParams,
        mut results: api::GetDocumentResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            let _ = params;
            let s = self.snapshot()?;
            let mut r = results.get();
            r.set_revision(s.revision);
            document_write(r.init_document(), &s.document);
            Ok(())
        })())
    }
    fn put_document(
        &mut self,
        params: api::PutDocumentParams,
        mut results: api::PutDocumentResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let revision = store_snapshot(
                &self.app,
                Snapshot {
                    revision: p.get_expected_revision(),
                    document: document_read(p.get_document()?)?,
                },
            )
            .map_err(error)?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
    fn get_entity(
        &mut self,
        params: api::GetEntityParams,
        mut results: api::GetEntityResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            let p = params.get()?;
            let id = text(p.get_id())?;
            let s = self.snapshot()?;
            let e = s
                .document
                .entities
                .iter()
                .find(|e| e.id == id)
                .ok_or_else(|| capnp::Error::failed("404: Entity not found".into()))?;
            let mut r = results.get();
            r.set_revision(s.revision);
            entity_write(r.init_entity(), e);
            Ok(())
        })())
    }
    fn put_entity(
        &mut self,
        params: api::PutEntityParams,
        mut results: api::PutEntityResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let mut item = entity_read(p.get_entity()?)?;
            let revision = self.mutate(p.get_expected_revision(), move |d| {
                if let Some(old) = d.entities.iter_mut().find(|e| e.id == item.id) {
                    // Ports belong to putPorts: keep what is stored.
                    item.faceplate = old.faceplate;
                    item.ports = std::mem::take(&mut old.ports);
                    *old = item;
                } else {
                    item.faceplate = None;
                    item.ports.clear();
                    d.entities.push(item);
                }
                Ok(())
            })?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
    fn delete_entity(
        &mut self,
        params: api::DeleteEntityParams,
        mut results: api::DeleteEntityResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let id = text(p.get_id())?;
            let revision = self.mutate(p.get_expected_revision(), move |d| {
                if !d.entities.iter().any(|e| e.id == id) {
                    return Err(capnp::Error::failed("404: Not found".into()));
                }
                if d.entities.iter().any(|e| e.parent.as_deref() == Some(&id))
                    || d.connections
                        .iter()
                        .any(|c| c.endpoints.iter().any(|ep| ep.entity == id))
                {
                    return Err(invalid(
                        "Entity has children or connections; remove them explicitly first",
                    ));
                }
                d.entities.retain(|e| e.id != id);
                Ok(())
            })?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
    fn put_connection(
        &mut self,
        params: api::PutConnectionParams,
        mut results: api::PutConnectionResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let item = connection_read(p.get_connection()?)?;
            let revision = self.mutate(p.get_expected_revision(), move |d| {
                if let Some(old) = d.connections.iter_mut().find(|e| e.id == item.id) {
                    *old = item;
                } else {
                    d.connections.push(item);
                }
                Ok(())
            })?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
    fn delete_connection(
        &mut self,
        params: api::DeleteConnectionParams,
        mut results: api::DeleteConnectionResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let id = text(p.get_id())?;
            let revision = self.mutate(p.get_expected_revision(), move |d| {
                if !d.connections.iter().any(|e| e.id == id) {
                    return Err(capnp::Error::failed("404: Not found".into()));
                }
                d.connections.retain(|e| e.id != id);
                Ok(())
            })?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
    fn put_layer(
        &mut self,
        params: api::PutLayerParams,
        mut results: api::PutLayerResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let item = layer_read(p.get_layer()?)?;
            let revision = self.mutate(p.get_expected_revision(), move |d| {
                if let Some(old) = d.layers.iter_mut().find(|e| e.id == item.id) {
                    *old = item;
                } else {
                    d.layers.push(item);
                }
                Ok(())
            })?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
    fn delete_layer(
        &mut self,
        params: api::DeleteLayerParams,
        mut results: api::DeleteLayerResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let id = text(p.get_id())?;
            let revision = self.mutate(p.get_expected_revision(), move |d| {
                if !d.layers.iter().any(|e| e.id == id) {
                    return Err(capnp::Error::failed("404: Not found".into()));
                }
                if d.connections.iter().any(|c| c.layer == id) {
                    return Err(invalid("Layer still has connections"));
                }
                d.layers.retain(|e| e.id != id);
                Ok(())
            })?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
    fn list_entities(
        &mut self,
        params: api::ListEntitiesParams,
        mut results: api::ListEntitiesResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(false)?;
            let p = params.get()?;
            let kind = text(p.get_kind())?;
            let parent = text(p.get_parent())?;
            let s = self.snapshot()?;
            let items: Vec<_> = s
                .document
                .entities
                .iter()
                .filter(|e| {
                    (kind.is_empty() || e.kind == kind)
                        && (parent.is_empty() || e.parent.as_deref() == Some(parent.as_str()))
                })
                .collect();
            let offset = (p.get_offset() as usize).min(items.len());
            let limit = if p.get_limit() == 0 {
                100
            } else {
                p.get_limit().min(200) as usize
            };
            let end = (offset + limit).min(items.len());
            let mut r = results.get();
            r.set_revision(s.revision);
            r.set_has_more(end < items.len());
            r.set_next_offset(end as u32);
            let mut list = r.init_entities((end - offset) as u32);
            for (i, item) in items[offset..end].iter().enumerate() {
                entity_write(list.reborrow().get(i as u32), item);
            }
            Ok(())
        })())
    }
    fn list_connections(
        &mut self,
        params: api::ListConnectionsParams,
        mut results: api::ListConnectionsResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(false)?;
            let p = params.get()?;
            let layer = text(p.get_layer())?;
            let entity = text(p.get_entity())?;
            let s = self.snapshot()?;
            let items: Vec<_> = s
                .document
                .connections
                .iter()
                .filter(|e| {
                    (layer.is_empty() || e.layer == layer)
                        && (entity.is_empty() || e.endpoints.iter().any(|ep| ep.entity == entity))
                })
                .collect();
            let offset = (p.get_offset() as usize).min(items.len());
            let limit = if p.get_limit() == 0 {
                100
            } else {
                p.get_limit().min(200) as usize
            };
            let end = (offset + limit).min(items.len());
            let mut r = results.get();
            r.set_revision(s.revision);
            r.set_has_more(end < items.len());
            r.set_next_offset(end as u32);
            let mut list = r.init_connections((end - offset) as u32);
            for (i, item) in items[offset..end].iter().enumerate() {
                connection_write(list.reborrow().get(i as u32), item);
            }
            Ok(())
        })())
    }
    fn list_layers(
        &mut self,
        params: api::ListLayersParams,
        mut results: api::ListLayersResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            let _ = params;
            let s = self.snapshot()?;
            let mut r = results.get();
            r.set_revision(s.revision);
            let mut list = r.init_layers(s.document.layers.len() as u32);
            for (i, l) in s.document.layers.iter().enumerate() {
                layer_write(list.reborrow().get(i as u32), l);
            }
            Ok(())
        })())
    }
    fn list_views(
        &mut self,
        params: api::ListViewsParams,
        mut results: api::ListViewsResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            let _ = params;
            self.check(false)?;
            let views = load_views(&self.app).map_err(error)?;
            let mut list = results.get().init_views(views.len() as u32);
            for (i, v) in views.iter().enumerate() {
                view_write(list.reborrow().get(i as u32), v)?;
            }
            Ok(())
        })())
    }
    fn put_view(
        &mut self,
        params: api::PutViewParams,
        mut results: api::PutViewResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            let _ = &mut results;
            self.check(true)?;
            let p = params.get()?.get_view()?;
            let json = text(p.get_view_json())?;
            if json.len() > 2 * 1024 * 1024 {
                return Err(invalid("View too large"));
            }
            let v = SavedView {
                id: text(p.get_id())?,
                name: text(p.get_name())?,
                view: serde_json::from_str(&json).map_err(|e| invalid(e.to_string()))?,
            };
            store_view(&self.app, v).map_err(error)?;
            Ok(())
        })())
    }
    fn delete_view(
        &mut self,
        params: api::DeleteViewParams,
        mut results: api::DeleteViewResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            let _ = &mut results;
            self.check(true)?;
            let id = text(params.get()?.get_id())?;
            remove_view(&self.app, &id).map_err(error)?;
            Ok(())
        })())
    }
    fn put_ports(
        &mut self,
        params: api::PutPortsParams,
        mut results: api::PutPortsResults,
    ) -> Promise<(), capnp::Error> {
        finish((|| {
            self.check(true)?;
            let p = params.get()?;
            let id = text(p.get_entity())?;
            let faceplate = faceplate_read(p.get_faceplate())?;
            let ports = ports_read(p.get_ports()?)?;
            let revision = self.mutate(p.get_expected_revision(), move |d| {
                let e = d
                    .entities
                    .iter_mut()
                    .find(|e| e.id == id)
                    .ok_or_else(|| capnp::Error::failed("404: Entity not found".into()))?;
                e.faceplate = faceplate;
                e.ports = ports;
                Ok(())
            })?;
            results.get().set_revision(revision);
            Ok(())
        })())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> App {
        let mut db = rusqlite::Connection::open_in_memory().unwrap();
        crate::initialize(&mut db).unwrap();
        App {
            db: std::sync::Arc::new(std::sync::Mutex::new(db)),
            grain: true,
        }
    }
    fn client(app: &App, rights: &[&str]) -> api::Client {
        capnp_rpc::new_client(Api {
            app: app.clone(),
            rights: rights.iter().map(|s| s.to_string()).collect(),
        })
    }
    fn entity(id: &str) -> Entity {
        Entity {
            id: id.into(),
            name: "Test device".into(),
            kind: "device".into(),
            parent: Some("kitchen".into()),
            links: vec![ExternalLink {
                provider: "home_assistant".into(),
                kind: "entity".into(),
                external_id: "light.test".into(),
            }],
            faceplate: None,
            ports: Vec::new(),
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn rights_are_enforced_on_every_method() {
        let app = app();
        for rights in [vec![], vec!["writer"], vec!["read-write"]] {
            let c = client(&app, &rights);
            assert!(c.get_document_request().send().promise.await.is_err());
            assert!(c.list_entities_request().send().promise.await.is_err());
            assert!(c.get_entity_request().send().promise.await.is_err());
            assert!(c.list_connections_request().send().promise.await.is_err());
            assert!(c.list_layers_request().send().promise.await.is_err());
            assert!(c.list_views_request().send().promise.await.is_err());
        }
        let c = client(&app, &["read"]);
        assert!(c.get_document_request().send().promise.await.is_ok());
        assert!(c.put_document_request().send().promise.await.is_err());
        assert!(c.put_entity_request().send().promise.await.is_err());
        assert!(c.delete_entity_request().send().promise.await.is_err());
        assert!(c.put_connection_request().send().promise.await.is_err());
        assert!(c.delete_connection_request().send().promise.await.is_err());
        assert!(c.put_layer_request().send().promise.await.is_err());
        assert!(c.delete_layer_request().send().promise.await.is_err());
        assert!(c.put_view_request().send().promise.await.is_err());
        assert!(c.delete_view_request().send().promise.await.is_err());
        assert!(c.put_ports_request().send().promise.await.is_err());
        assert_eq!(load_snapshot(&app).unwrap().revision, 1);
        for rights in [["write"], ["owner"]] {
            assert!(
                client(&app, &rights)
                    .get_document_request()
                    .send()
                    .promise
                    .await
                    .is_ok()
            );
        }
    }
    #[tokio::test(flavor = "current_thread")]
    async fn ports_round_trip_and_put_entity_keeps_them() {
        let app = app();
        let c = client(&app, &["read", "write"]);
        let revision = |r: capnp::Result<u64>| r.unwrap();
        let ports = vec![
            Port {
                id: "1".into(),
                label: "20 A kitchen".into(),
                position: Some(1),
                poles: None,
                half: None,
                halves: Vec::new(),
                main: false,
                amps: Some(20),
                protection: Some(Protection::Gfci),
            },
            Port {
                id: "3/5".into(),
                label: "30 A dryer".into(),
                position: Some(3),
                poles: Some(2),
                half: None,
                halves: vec!["B".into(), "A".into()],
                main: false,
                amps: Some(30),
                protection: None,
            },
            Port {
                id: "MAIN".into(),
                label: "100 A main".into(),
                position: None,
                poles: Some(2),
                half: None,
                halves: Vec::new(),
                main: true,
                amps: Some(100),
                protection: None,
            },
        ];
        let mut req = c.put_ports_request();
        req.get().set_expected_revision(1);
        req.get().set_entity("panel");
        req.get().set_faceplate(wire::Faceplate::Panel);
        ports_write(req.get().init_ports(ports.len() as u32), &ports);
        let r = req.send().promise.await.unwrap();
        assert_eq!(revision(Ok(r.get().unwrap().get_revision())), 2);
        // putEntity (which carries no ports) renames without dropping them.
        let mut req = c.get_entity_request();
        req.get().set_id("panel");
        let got = req.send().promise.await.unwrap();
        let mut panel = entity_read(got.get().unwrap().get_entity().unwrap()).unwrap();
        assert_eq!(panel.faceplate, Some(Faceplate::Panel));
        assert_eq!(panel.ports, ports);
        panel.name = "Main panel".into();
        panel.ports.clear();
        panel.faceplate = None;
        let mut req = c.put_entity_request();
        req.get().set_expected_revision(2);
        entity_write(req.get().init_entity(), &panel);
        req.send().promise.await.unwrap();
        // An endpoint names a port; it survives the round trip.
        let mut req = c.list_connections_request();
        req.get().set_entity("panel");
        let list = req.send().promise.await.unwrap();
        let mut circuit =
            connection_read(list.get().unwrap().get_connections().unwrap().get(0)).unwrap();
        circuit
            .endpoints
            .iter_mut()
            .find(|ep| ep.entity == "panel")
            .unwrap()
            .port = Some("3/5".into());
        let mut req = c.put_connection_request();
        req.get().set_expected_revision(3);
        connection_write(req.get().init_connection(), &circuit);
        req.send().promise.await.unwrap();
        let doc = c.get_document_request().send().promise.await.unwrap();
        let doc = document_read(doc.get().unwrap().get_document().unwrap()).unwrap();
        let panel = doc.entities.iter().find(|e| e.id == "panel").unwrap();
        assert_eq!((panel.name.as_str(), panel.ports.len()), ("Main panel", 3));
        let c2 = doc.connections.iter().find(|x| x.id == circuit.id).unwrap();
        assert_eq!(
            c2.endpoints
                .iter()
                .find(|ep| ep.entity == "panel")
                .unwrap()
                .port
                .as_deref(),
            Some("3/5")
        );
        // Removing a port an endpoint still names is refused.
        let mut req = c.put_ports_request();
        req.get().set_expected_revision(4);
        req.get().set_entity("panel");
        req.get().set_faceplate(wire::Faceplate::Panel);
        ports_write(req.get().init_ports(1), &ports[..1]);
        let err = req.send().promise.await.err().unwrap().to_string();
        assert!(err.contains("400") && err.contains("3/5"), "{err}");
    }
    #[tokio::test(flavor = "current_thread")]
    async fn entity_links_paging_validation_and_revision_conflicts() {
        let app = app();
        let c = client(&app, &["read", "write"]);
        let mut req = c.put_entity_request();
        req.get().set_expected_revision(1);
        entity_write(req.get().init_entity(), &entity("test"));
        assert_eq!(
            req.send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_revision(),
            2
        );
        let mut req = c.get_entity_request();
        req.get().set_id("test");
        let response = req.send().promise.await.unwrap();
        let e = entity_read(response.get().unwrap().get_entity().unwrap()).unwrap();
        assert_eq!(e.links[0].external_id, "light.test");
        assert_eq!(e.parent.as_deref(), Some("kitchen"));
        let mut req = c.list_entities_request();
        req.get().set_parent("kitchen");
        req.get().set_kind("device");
        req.get().set_limit(1);
        let response = req.send().promise.await.unwrap();
        let r = response.get().unwrap();
        assert_eq!(r.get_entities().unwrap().len(), 1);
        assert!(r.get_has_more());
        assert_eq!(r.get_next_offset(), 1);
        let mut req = c.put_entity_request();
        req.get().set_expected_revision(1);
        entity_write(req.get().init_entity(), &entity("stale"));
        assert!(
            req.send()
                .promise
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("409")
        );
        // A concurrent HTTP write uses the same revision and database.
        let mut s = load_snapshot(&app).unwrap();
        s.document.entities[0].name = "HTTP edit".into();
        store_snapshot(&app, s).unwrap();
        let mut req = c.delete_entity_request();
        req.get().set_expected_revision(2);
        req.get().set_id("test");
        assert!(
            req.send()
                .promise
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("409")
        );
        let mut bad = entity("bad");
        bad.parent = Some("bad".into());
        let mut req = c.put_entity_request();
        req.get().set_expected_revision(3);
        entity_write(req.get().init_entity(), &bad);
        assert!(req.send().promise.await.is_err());
        let mut req = c.delete_entity_request();
        req.get().set_expected_revision(3);
        req.get().set_id("kitchen");
        assert!(req.send().promise.await.is_err());
        assert_eq!(load_snapshot(&app).unwrap().revision, 3);
        let mut req = c.delete_entity_request();
        req.get().set_expected_revision(3);
        req.get().set_id("test");
        assert_eq!(
            req.send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_revision(),
            4
        );
    }
    #[tokio::test(flavor = "current_thread")]
    async fn connections_layers_views_and_full_document_share_storage() {
        let app = app();
        let c = client(&app, &["owner"]);
        let mut req = c.put_layer_request();
        req.get().set_expected_revision(1);
        layer_write(
            req.get().init_layer(),
            &Layer {
                id: "test".into(),
                name: "Test".into(),
                color: "#aabbcc".into(),
            },
        );
        req.send().promise.await.unwrap();
        let connection = Connection {
            id: "test".into(),
            name: "Directed".into(),
            layer: "test".into(),
            endpoints: vec![
                Endpoint {
                    entity: "lamp".into(),
                    role: Role::Target,
                    port: None,
                },
                Endpoint {
                    entity: "sensor".into(),
                    role: Role::Source,
                    port: None,
                },
            ],
        };
        let mut req = c.put_connection_request();
        req.get().set_expected_revision(2);
        connection_write(req.get().init_connection(), &connection);
        req.send().promise.await.unwrap();
        let mut req = c.list_connections_request();
        req.get().set_layer("test");
        req.get().set_entity("lamp");
        let response = req.send().promise.await.unwrap();
        let list = response.get().unwrap().get_connections().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            connection_read(list.get(0)).unwrap().endpoints[0].role,
            Role::Target
        );
        let mut req = c.delete_layer_request();
        req.get().set_expected_revision(3);
        req.get().set_id("test");
        assert!(req.send().promise.await.is_err());
        let mut req = c.delete_connection_request();
        req.get().set_expected_revision(3);
        req.get().set_id("test");
        req.send().promise.await.unwrap();
        let mut req = c.delete_layer_request();
        req.get().set_expected_revision(4);
        req.get().set_id("test");
        req.send().promise.await.unwrap();
        let mut req = c.put_view_request();
        view_write(
            req.get().init_view(),
            &SavedView {
                id: "test".into(),
                name: "Test".into(),
                view: stufftracker_core::View::default(),
            },
        )
        .unwrap();
        req.send().promise.await.unwrap();
        assert_eq!(load_views(&app).unwrap().len(), 1);
        let response = c.list_views_request().send().promise.await.unwrap();
        assert_eq!(response.get().unwrap().get_views().unwrap().len(), 1);
        let mut req = c.put_view_request();
        let mut v = req.get().init_view();
        v.set_id("bad");
        v.set_name("Bad");
        v.set_view_json("{bad}");
        assert!(req.send().promise.await.is_err());
        let mut req = c.delete_view_request();
        req.get().set_id("test");
        req.send().promise.await.unwrap();
        assert!(load_views(&app).unwrap().is_empty());
        let s = load_snapshot(&app).unwrap();
        let mut req = c.put_document_request();
        req.get().set_expected_revision(s.revision);
        document_write(req.get().init_document(), &s.document);
        assert_eq!(
            req.send()
                .promise
                .await
                .unwrap()
                .get()
                .unwrap()
                .get_revision(),
            6
        );
    }
}
