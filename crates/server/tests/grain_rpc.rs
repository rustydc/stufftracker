//! Real child process + Unix-socket Darude ABI v2 handshake and restart.
#[allow(dead_code, unused_parens, clippy::all)]
mod stufftracker_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/stufftracker_v1_capnp.rs"));
}
use capnp::capability::Promise;
use capnp_rpc::{RpcSystem, pry, rpc_twoparty_capnp, twoparty};
use darude_wire::grain_v2_capnp::{bootstrap, grain, grain_ctl};
use std::{cell::RefCell, rc::Rc};
use stufftracker_v1_capnp::stuff_tracker_v1 as api;
use tokio::sync::oneshot;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
struct Ctl {
    announced: Rc<RefCell<Option<oneshot::Sender<Vec<u8>>>>>,
}
impl grain_ctl::Server for Ctl {
    fn announce(
        &mut self,
        params: grain_ctl::AnnounceParams,
        _: grain_ctl::AnnounceResults,
    ) -> Promise<(), capnp::Error> {
        let p = pry!(params.get());
        assert_eq!(pry!(pry!(p.get_name()).to_str()), "stufftracker.v1");
        assert_eq!(
            p.get_interface_id(),
            <api::Client as capnp::traits::HasTypeId>::TYPE_ID
        );
        if let Some(tx) = self.announced.borrow_mut().take() {
            let _ = tx.send(pry!(p.get_schema()).to_vec());
        }
        Promise::ok(())
    }
}
struct Bootstrap {
    registered: Option<oneshot::Sender<grain::Client>>,
    ctl: grain_ctl::Client,
}
impl bootstrap::Server for Bootstrap {
    fn register_grain(
        &mut self,
        params: bootstrap::RegisterGrainParams,
        mut results: bootstrap::RegisterGrainResults,
    ) -> Promise<(), capnp::Error> {
        let grain = pry!(pry!(params.get()).get_grain());
        results.get().set_ctl(self.ctl.clone());
        results.get().init_slots(0);
        let _ = self.registered.take().unwrap().send(grain);
        Promise::ok(())
    }
}
struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
async fn session(grain: &grain::Client, rights: &[&str]) -> api::Client {
    let mut req = grain.init_session_request();
    req.get().init_principal().set_username("test");
    let mut list = req.get().init_rights(rights.len() as u32);
    for (i, right) in rights.iter().enumerate() {
        list.set(i as u32, *right);
    }
    let session = req
        .send()
        .promise
        .await
        .unwrap()
        .get()
        .unwrap()
        .get_session()
        .unwrap();
    let mut req = session.get_request();
    req.get().set_interface("stufftracker.v1");
    let response = req.send().promise.await.unwrap();
    let r = response.get().unwrap();
    assert!(r.get_found());
    r.get_cap().get_as_capability().unwrap()
}
#[tokio::test(flavor = "current_thread")]
async fn registers_announces_enforces_rights_and_survives_restart() {
    tokio::task::LocalSet::new()
        .run_until(async {
            tokio::time::timeout(std::time::Duration::from_secs(30), run_scenario())
                .await
                .expect("grain handshake timed out");
        })
        .await;
}
async fn run_scenario() {
    let root = std::env::temp_dir().join(format!("stufftracker-rpc-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let platform = root.join("platform.sock");
    let http = root.join("http.sock");
    let _ = std::fs::remove_file(&platform);
    let listener = tokio::net::UnixListener::bind(&platform).unwrap();
    for boot in 0..2 {
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_stufftracker"))
            .env("DARUDE_STATE_DIR", root.join("state"))
            .env("DARUDE_HTTP_SOCKET", &http)
            .env("DARUDE_PLATFORM_SOCKET", &platform)
            .spawn()
            .unwrap();
        let mut child = Child(child);
        let (stream, _) = listener.accept().await.unwrap();
        let (registered, rx) = oneshot::channel();
        let (announced, announce_rx) = oneshot::channel();
        let ctl: grain_ctl::Client = capnp_rpc::new_client(Ctl {
            announced: Rc::new(RefCell::new(Some(announced))),
        });
        let bootstrap: bootstrap::Client = capnp_rpc::new_client(Bootstrap {
            registered: Some(registered),
            ctl,
        });
        let (read, write) = stream.into_split();
        let network = twoparty::VatNetwork::new(
            futures::io::BufReader::new(read.compat()),
            futures::io::BufWriter::new(write.compat_write()),
            rpc_twoparty_capnp::Side::Server,
            Default::default(),
        );
        let driver =
            tokio::task::spawn_local(RpcSystem::new(Box::new(network), Some(bootstrap.client)));
        let grain = rx.await.unwrap();
        let schema = announce_rx.await.unwrap();
        let message =
            capnp::serialize::read_message(&mut schema.as_slice(), Default::default()).unwrap();
        let cgr = message
            .get_root::<capnp::schema_capnp::code_generator_request::Reader>()
            .unwrap();
        assert!(
            cgr.get_nodes()
                .unwrap()
                .iter()
                .any(|n| n.get_id() == <api::Client as capnp::traits::HasTypeId>::TYPE_ID)
        );
        assert!(
            !cgr.get_source_info().unwrap().is_empty(),
            "shipped schema contains caller-facing docs"
        );
        let read = session(&grain, &["read"]).await;
        let owner = session(&grain, &["owner"]).await;
        use capnp::capability::FromClientHook;
        use darude_runtime::introspect::{SchemaCache, Value, dynamic_call};
        let cache = SchemaCache::new();
        cache.load(&schema).unwrap();
        let id = <api::Client as capnp::traits::HasTypeId>::TYPE_ID;
        let interface = cache.interface(id).unwrap();
        assert_eq!(interface.methods.len(), 15);
        assert!(
            interface
                .methods
                .iter()
                .all(|m| m.doc.as_ref().is_some_and(|d| !d.is_empty()))
        );
        let method = interface.method("listEntities").unwrap();
        let hook = read.clone().into_client_hook();
        let value = dynamic_call(
            &*hook,
            id,
            method,
            cache.struct_schema(method.param_type_id).unwrap(),
            cache.struct_schema(method.result_type_id).unwrap(),
            &Value::Struct(vec![
                ("kind".into(), Value::Text("device".into())),
                ("limit".into(), Value::UInt(1)),
            ]),
        )
        .await
        .unwrap();
        assert!(matches!(value.get("entities"), Some(Value::List(items)) if items.len() == 1));
        let snapshot = read.get_document_request().send().promise.await.unwrap();
        let r = snapshot.get().unwrap();
        assert_eq!(r.get_revision(), if boot == 0 { 1 } else { 2 });
        let mut denied = read.delete_entity_request();
        denied.get().set_id("sensor");
        denied.get().set_expected_revision(r.get_revision());
        assert!(denied.send().promise.await.is_err());
        if boot == 0 {
            let mut req = owner.put_entity_request();
            req.get().set_expected_revision(1);
            let mut e = req.get().init_entity();
            e.set_id("rpc-test");
            e.set_name("RPC device");
            e.set_kind("device");
            e.set_parent("kitchen");
            let mut link = e.init_links(1).get(0);
            link.set_provider("home_assistant");
            link.set_kind("entity");
            link.set_external_id("light.rpc_test");
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
            let mut req = owner.put_view_request();
            let mut v = req.get().init_view();
            v.set_id("rpc-view");
            v.set_name("RPC saved view");
            v.set_view_json("{\"collapsed\":[\"kitchen\"]}");
            req.send().promise.await.unwrap();
        } else {
            let mut req = read.get_entity_request();
            req.get().set_id("rpc-test");
            let response = req.send().promise.await.unwrap();
            let e = response.get().unwrap().get_entity().unwrap();
            assert_eq!(
                e.get_links()
                    .unwrap()
                    .get(0)
                    .get_external_id()
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "light.rpc_test"
            );
            let response = read.list_views_request().send().promise.await.unwrap();
            assert_eq!(response.get().unwrap().get_views().unwrap().len(), 1);
        }
        // The HTTP surface is bound and sees the same persisted capability write.
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::UnixStream::connect(&http).await.unwrap();
        stream
            .write_all(
                b"GET /api/document HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        assert!(response.contains("rpc-test"));
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        driver.abort();
    }
    drop(listener);
    std::fs::remove_dir_all(&root).unwrap();
}
