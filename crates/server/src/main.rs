mod rpc;
#[allow(dead_code, unused_parens, clippy::all)]
pub mod stufftracker_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/stufftracker_v1_capnp.rs"));
}
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::future::IntoFuture;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use stufftracker_core::{Document, LayoutRequest, View};
#[derive(Clone)]
struct App {
    db: Arc<Mutex<Connection>>,
    grain: bool,
}
type Error = (StatusCode, String);
fn internal(e: impl std::fmt::Display) -> Error {
    eprintln!("{e}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Storage operation failed".into(),
    )
}
#[derive(Serialize, Deserialize)]
struct Snapshot {
    revision: u64,
    document: Document,
}
#[derive(Serialize, Deserialize)]
struct SavedView {
    id: String,
    name: String,
    view: View,
}
fn writable(app: &App, h: &HeaderMap) -> bool {
    !app.grain
        || h.get("x-darude-permissions")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|v| v.split(',').any(|p| matches!(p.trim(), "write" | "owner")))
}
fn require_write(app: &App, h: &HeaderMap) -> Result<(), Error> {
    if writable(app, h) {
        Ok(())
    } else {
        Err((StatusCode::FORBIDDEN, "This share is read-only".into()))
    }
}
fn initialize(db: &mut Connection) -> rusqlite::Result<()> {
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS meta(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS entities(id TEXT PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS connections(id TEXT PRIMARY KEY,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS layers(id TEXT PRIMARY KEY,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS views(id TEXT PRIMARY KEY,name TEXT NOT NULL,data TEXT NOT NULL);")?;
    if db.query_row("SELECT count(*) FROM meta", [], |r| r.get::<_, u32>(0))? == 0 {
        let tx = db.transaction()?;
        write_document(&tx, &stufftracker_core::sample())?;
        tx.execute("INSERT INTO meta VALUES(1,1)", [])?;
        tx.commit()?;
    }
    Ok(())
}
fn write_document(db: &Connection, doc: &Document) -> rusqlite::Result<()> {
    db.execute_batch("DELETE FROM entities; DELETE FROM connections; DELETE FROM layers;")?;
    for e in &doc.entities {
        db.execute(
            "INSERT INTO entities VALUES(?1,?2)",
            params![e.id, serde_json::to_string(e).unwrap()],
        )?;
    }
    for c in &doc.connections {
        db.execute(
            "INSERT INTO connections VALUES(?1,?2)",
            params![c.id, serde_json::to_string(c).unwrap()],
        )?;
    }
    for l in &doc.layers {
        db.execute(
            "INSERT INTO layers VALUES(?1,?2)",
            params![l.id, serde_json::to_string(l).unwrap()],
        )?;
    }
    Ok(())
}
fn rows<T: serde::de::DeserializeOwned>(db: &Connection, table: &str) -> Result<Vec<T>, Error> {
    let mut stmt = db
        .prepare(&format!("SELECT data FROM {table} ORDER BY rowid"))
        .map_err(internal)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(internal)?;
    rows.map(|r| serde_json::from_str(&r.map_err(internal)?).map_err(internal))
        .collect()
}
fn load_snapshot(a: &App) -> Result<Snapshot, Error> {
    let db = a.db.lock().map_err(internal)?;
    let revision = db
        .query_row("SELECT revision FROM meta WHERE id=1", [], |r| r.get(0))
        .map_err(internal)?;
    Ok(Snapshot {
        revision,
        document: Document {
            entities: rows(&db, "entities")?,
            connections: rows(&db, "connections")?,
            layers: rows(&db, "layers")?,
        },
    })
}
async fn document(State(a): State<App>) -> Result<Json<Snapshot>, Error> {
    load_snapshot(&a).map(Json)
}
async fn put_document(
    State(a): State<App>,
    h: HeaderMap,
    Json(s): Json<Snapshot>,
) -> Result<Json<serde_json::Value>, Error> {
    require_write(&a, &h)?;
    let revision = store_snapshot(&a, s)?;
    Ok(Json(serde_json::json!({"revision":revision})))
}
fn store_snapshot(a: &App, s: Snapshot) -> Result<u64, Error> {
    stufftracker_core::validate(&s.document).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let mut db = a.db.lock().map_err(internal)?;
    let tx = db.transaction().map_err(internal)?;
    let revision: u64 = tx
        .query_row("SELECT revision FROM meta WHERE id=1", [], |r| r.get(0))
        .map_err(internal)?;
    if revision != s.revision {
        return Err((
            StatusCode::CONFLICT,
            "The document changed in another window. Reload before editing.".into(),
        ));
    }
    write_document(&tx, &s.document).map_err(internal)?;
    tx.execute("UPDATE meta SET revision=revision+1 WHERE id=1", [])
        .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(revision + 1)
}
fn load_views(a: &App) -> Result<Vec<SavedView>, Error> {
    let db = a.db.lock().map_err(internal)?;
    let mut q = db
        .prepare("SELECT id,name,data FROM views ORDER BY name")
        .map_err(internal)?;
    let result = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(internal)?
        .map(|row| {
            let (id, name, data) = row.map_err(internal)?;
            Ok(SavedView {
                id,
                name,
                view: serde_json::from_str(&data).map_err(internal)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(result)
}
async fn views(State(a): State<App>) -> Result<Json<Vec<SavedView>>, Error> {
    load_views(&a).map(Json)
}
async fn save_view(
    State(a): State<App>,
    h: HeaderMap,
    Json(v): Json<SavedView>,
) -> Result<StatusCode, Error> {
    require_write(&a, &h)?;
    store_view(&a, v)?;
    Ok(StatusCode::NO_CONTENT)
}
fn store_view(a: &App, v: SavedView) -> Result<(), Error> {
    stufftracker_core::validate_view(&v.view).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if v.name.trim().is_empty() || v.id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "A view needs an ID and name".into(),
        ));
    }
    a.db.lock().map_err(internal)?.execute("INSERT INTO views VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET name=excluded.name,data=excluded.data",params![v.id,v.name,serde_json::to_string(&v.view).map_err(internal)?]).map_err(internal)?;
    Ok(())
}
async fn delete_view(
    State(a): State<App>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, Error> {
    require_write(&a, &h)?;
    remove_view(&a, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
fn remove_view(a: &App, id: &str) -> Result<(), Error> {
    a.db.lock()
        .map_err(internal)?
        .execute("DELETE FROM views WHERE id=?1", [id])
        .map_err(internal)?;
    Ok(())
}
async fn layout(Json(r): Json<LayoutRequest>) -> Result<Json<stufftracker_core::Layout>, Error> {
    stufftracker_core::layout(&r)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}
async fn move_node(Json(r): Json<stufftracker_core::MoveRequest>) -> Result<Json<View>, Error> {
    stufftracker_core::move_entity(&r)
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))
}
fn router(a: App) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Html(include_str!("../../../dist/index.html")) }),
        )
        .route("/api/document", get(document).put(put_document))
        .route("/api/views", get(views).post(save_view))
        .route("/api/views/{id}", axum::routing::delete(delete_view))
        .route("/api/layout", post(layout))
        .route("/api/move", post(move_node))
        .route(
            "/api/session",
            get(|State(a): State<App>, h: HeaderMap| async move {
                Json(serde_json::json!({"writable":writable(&a,&h)}))
            }),
        )
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .with_state(a)
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let state = PathBuf::from(
        std::env::var("DARUDE_STATE_DIR")
            .or_else(|_| std::env::var("STUFFTRACKER_STATE_DIR"))
            .unwrap_or("data".into()),
    );
    std::fs::create_dir_all(&state)?;
    let mut db = Connection::open(state.join("stufftracker.sqlite3"))?;
    initialize(&mut db)?;
    let socket = std::env::var("DARUDE_HTTP_SOCKET").ok();
    let state = App {
        db: Arc::new(Mutex::new(db)),
        grain: socket.is_some() || std::env::var_os("DARUDE_PLATFORM_SOCKET").is_some(),
    };
    if std::env::var_os("DARUDE_PLATFORM_SOCKET").is_some() && socket.is_none() {
        return Err("DARUDE_PLATFORM_SOCKET requires DARUDE_HTTP_SOCKET".into());
    }
    let app = router(state.clone());
    if let Some(path) = socket {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        };
        let listener = tokio::net::UnixListener::bind(&path)?;
        eprintln!("StuffTracker HTTP bound on {path}");
        if std::env::var_os("DARUDE_PLATFORM_SOCKET").is_some() {
            let local = tokio::task::LocalSet::new();
            local.run_until(async move {
                tokio::select! {
                    result = axum::serve(listener, app).into_future() => result.map_err(anyhow::Error::from),
                    result = rpc::serve(state) => result,
                }
            }).await?;
        } else {
            axum::serve(listener, app).await?;
        }
    } else {
        let address = std::env::var("STUFFTRACKER_BIND").unwrap_or("127.0.0.1:8090".into());
        let listener = tokio::net::TcpListener::bind(&address).await?;
        eprintln!("StuffTracker: http://{address}");
        axum::serve(listener, app).await?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn database_survives_restart_without_reseed() {
        let mut db = Connection::open_in_memory().unwrap();
        initialize(&mut db).unwrap();
        db.execute("UPDATE meta SET revision=9", []).unwrap();
        initialize(&mut db).unwrap();
        assert_eq!(
            db.query_row("SELECT revision FROM meta", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            9
        );
        assert_eq!(
            rows::<stufftracker_core::Entity>(&db, "entities")
                .unwrap()
                .len(),
            17
        );
    }
    #[test]
    fn grain_permissions_fail_closed() {
        let a = App {
            db: Arc::new(Mutex::new(Connection::open_in_memory().unwrap())),
            grain: true,
        };
        let mut h = HeaderMap::new();
        assert!(!writable(&a, &h));
        h.insert("x-darude-permissions", "read".parse().unwrap());
        assert!(!writable(&a, &h));
        h.insert("x-darude-permissions", "read,write".parse().unwrap());
        assert!(writable(&a, &h));
    }
}
