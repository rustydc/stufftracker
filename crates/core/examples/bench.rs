//! Time a full layout of an inventory:
//!   cargo run --release -p stufftracker-core --example bench -- inventory.json
//! The file may be a bare Document, a GET /api/document response, or a dump
//! keyed by "/api/document". Set N for the number of timed runs (default 5).
use std::time::Instant;
use stufftracker_core::*;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: bench <inventory.json>");
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let doc = raw
        .get("/api/document")
        .unwrap_or(&raw)
        .get("document")
        .unwrap_or(raw.get("/api/document").unwrap_or(&raw));
    let doc: Document = serde_json::from_value(doc.clone()).unwrap();
    let empty: View = serde_json::from_value(
        serde_json::json!({"positions": {}, "collapsed": [], "hidden_layers": []}),
    )
    .unwrap();
    // Lay out once to settle positions, then time layouts of the settled view.
    let first = layout(&LayoutRequest {
        document: doc.clone(),
        view: empty.clone(),
    })
    .unwrap();
    let req = LayoutRequest {
        document: doc,
        view: View {
            positions: first.positions,
            attachment_hints: first.attachment_hints,
            layout_version: 2,
            ..empty
        },
    };
    let n: usize = std::env::var("N").map(|v| v.parse().unwrap()).unwrap_or(5);
    let t = Instant::now();
    for _ in 0..n {
        layout(&req).unwrap();
    }
    let l = layout(&req).unwrap();
    println!(
        "layout {:.1}ms nodes {} bundles {} unrouted {}",
        t.elapsed().as_secs_f64() * 1000. / n.max(1) as f64,
        l.nodes.len(),
        l.bundles.len(),
        l.unrouted_connections.len()
    );
}
