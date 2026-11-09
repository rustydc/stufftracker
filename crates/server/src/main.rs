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
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS meta(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS entities(id TEXT PRIMARY KEY, data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS connections(id TEXT PRIMARY KEY,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS layers(id TEXT PRIMARY KEY,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS views(id TEXT PRIMARY KEY,name TEXT NOT NULL,data TEXT NOT NULL); CREATE TABLE IF NOT EXISTS attachments(id TEXT PRIMARY KEY, mime TEXT NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL, data BLOB NOT NULL, created_at INTEGER NOT NULL);")?;
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
    // Details may only name stored attachments; unreferenced uploads are
    // dropped once a day old (time to attach them after uploading).
    let referenced: std::collections::BTreeSet<&str> = s
        .document
        .entities
        .iter()
        .flat_map(|e| e.attachments())
        .collect();
    for id in &referenced {
        let known: bool = tx
            .query_row("SELECT count(*) FROM attachments WHERE id=?1", [id], |r| {
                r.get::<_, u32>(0).map(|n| n > 0)
            })
            .map_err(internal)?;
        if !known {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown attachment {id}; upload it first"),
            ));
        }
    }
    let stale: Vec<String> = {
        let mut q = tx
            .prepare("SELECT id FROM attachments WHERE created_at < ?1")
            .map_err(internal)?;
        q.query_map([now() - 86_400], |r| r.get(0))
            .map_err(internal)?
            .collect::<rusqlite::Result<_>>()
            .map_err(internal)?
    };
    for id in stale.iter().filter(|id| !referenced.contains(id.as_str())) {
        tx.execute("DELETE FROM attachments WHERE id=?1", [id])
            .map_err(internal)?;
    }
    write_document(&tx, &s.document).map_err(internal)?;
    tx.execute("UPDATE meta SET revision=revision+1 WHERE id=1", [])
        .map_err(internal)?;
    tx.commit().map_err(internal)?;
    Ok(revision + 1)
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
/// Largest attachment accepted (an image or a PDF manual).
const MAX_ATTACHMENT: usize = 25 * 1024 * 1024;
#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Attachment {
    attachment: String,
    name: String,
    mime: String,
    size: u64,
}
/// A plain `type/subtype` MIME type, lowercased; anything else is generic.
fn clean_mime(mime: &str) -> String {
    let m = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let ok = |p: &str| {
        !p.is_empty()
            && p.len() <= 60
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b))
    };
    match m.split_once('/') {
        Some((a, b)) if ok(a) && ok(b) => m,
        _ => "application/octet-stream".into(),
    }
}
fn clean_name(name: &str) -> String {
    let n: String = name
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .take(200)
        .collect();
    if n.trim().is_empty() {
        "attachment".into()
    } else {
        n.trim().into()
    }
}
/// Store bytes as an attachment, named by their SHA-256; storing the same
/// bytes again returns the same attachment.
fn store_attachment(a: &App, name: &str, mime: &str, data: &[u8]) -> Result<Attachment, Error> {
    use sha2::Digest;
    if data.is_empty() || data.len() > MAX_ATTACHMENT {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("Attachments must be 1 byte to {} MiB", MAX_ATTACHMENT >> 20),
        ));
    }
    let id: String = sha2::Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let (name, mime) = (clean_name(name), clean_mime(mime));
    a.db.lock()
        .map_err(internal)?
        .execute(
            "INSERT INTO attachments VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET created_at=excluded.created_at",
            params![id, mime, name, data.len() as i64, data, now()],
        )
        .map_err(internal)?;
    Ok(Attachment {
        attachment: id,
        name,
        mime,
        size: data.len() as u64,
    })
}
fn load_attachment(a: &App, id: &str) -> Result<(Attachment, Vec<u8>), Error> {
    let missing = || (StatusCode::NOT_FOUND, "No such attachment".to_string());
    if !stufftracker_core::is_attachment_id(id) {
        return Err(missing());
    }
    a.db.lock()
        .map_err(internal)?
        .query_row(
            "SELECT mime,name,size,data FROM attachments WHERE id=?1",
            [id],
            |r| {
                Ok((
                    Attachment {
                        attachment: id.into(),
                        mime: r.get(0)?,
                        name: r.get(1)?,
                        size: r.get::<_, i64>(2)? as u64,
                    },
                    r.get(3)?,
                ))
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => missing(),
            e => internal(e),
        })
}
/// Decode a percent-encoded header value (the browser sends file names
/// with encodeURIComponent).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        // Header values are visible ASCII, so byte slicing is safe.
        if let Some(v) = (b[i] == b'%' && i + 2 < b.len())
            .then(|| u8::from_str_radix(&s[i + 1..i + 3], 16).ok())
            .flatten()
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
async fn upload_attachment(
    State(a): State<App>,
    h: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<Attachment>, Error> {
    require_write(&a, &h)?;
    let header = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("");
    store_attachment(
        &a,
        &percent_decode(header("x-file-name")),
        header("content-type"),
        &body,
    )
    .map(Json)
}
/// Serve an attachment. Images (not SVG, which can carry script), PDFs and
/// plain text open in the browser; anything else downloads. Uploads are
/// untrusted, so nothing may sniff or script.
async fn get_attachment(
    State(a): State<App>,
    Path(id): Path<String>,
) -> Result<axum::response::Response, Error> {
    let (meta, data) = load_attachment(&a, &id)?;
    let inline = (meta.mime.starts_with("image/") && !meta.mime.contains("svg"))
        || meta.mime == "application/pdf"
        || meta.mime == "text/plain";
    let encoded: String = meta
        .name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let mut r = axum::response::Response::builder()
        .header(
            "content-type",
            if inline {
                meta.mime.as_str()
            } else {
                "application/octet-stream"
            },
        )
        .header(
            "content-disposition",
            format!(
                "{}; filename*=UTF-8''{encoded}",
                if inline { "inline" } else { "attachment" }
            ),
        )
        .header("x-content-type-options", "nosniff")
        .header("cache-control", "private, max-age=31536000, immutable");
    if meta.mime != "application/pdf" {
        // The browser's PDF viewer will not run in a sandboxed document.
        r = r.header(
            "content-security-policy",
            "sandbox; default-src 'none'; img-src 'self'",
        );
    }
    r.body(axum::body::Body::from(data)).map_err(internal)
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
            "/api/attachments",
            post(upload_attachment).layer(DefaultBodyLimit::max(MAX_ATTACHMENT)),
        )
        .route("/api/attachments/{id}", get(get_attachment))
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
    fn memory_app() -> App {
        let mut db = Connection::open_in_memory().unwrap();
        initialize(&mut db).unwrap();
        App {
            db: Arc::new(Mutex::new(db)),
            grain: false,
        }
    }
    #[test]
    fn attachments_are_content_addressed_sanitised_and_collected() {
        let a = memory_app();
        let one =
            store_attachment(&a, "../evil\nname.png", "IMAGE/PNG; x=1", b"png bytes").unwrap();
        assert_eq!(
            (one.name.as_str(), one.mime.as_str()),
            ("..evilname.png", "image/png")
        );
        assert_eq!(
            store_attachment(&a, "copy", "image/png", b"png bytes")
                .unwrap()
                .attachment,
            one.attachment
        );
        assert_eq!(
            store_attachment(&a, "x", "not a type", b"z").unwrap().mime,
            "application/octet-stream"
        );
        assert!(store_attachment(&a, "x", "a/b", b"").is_err());
        let (meta, data) = load_attachment(&a, &one.attachment).unwrap();
        assert_eq!((meta.size, data.as_slice()), (9, b"png bytes".as_slice()));
        assert_eq!(
            load_attachment(&a, "../../x").unwrap_err().0,
            StatusCode::NOT_FOUND
        );
        // Unreferenced uploads older than a day go on the next save.
        a.db.lock()
            .unwrap()
            .execute("UPDATE attachments SET created_at=0", [])
            .unwrap();
        let s = load_snapshot(&a).unwrap();
        store_snapshot(&a, s).unwrap();
        assert_eq!(
            load_attachment(&a, &one.attachment).unwrap_err().0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            percent_decode("US8%20manual%E2%9C%93.pdf"),
            "US8 manual✓.pdf"
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
