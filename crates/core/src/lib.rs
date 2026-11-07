//! Portable graph projection and routing: identical implementation in native Rust and WASM.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use wasm_bindgen::prelude::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub links: Vec<ExternalLink>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalLink {
    pub provider: String,
    pub kind: String,
    pub external_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Source,
    Target,
    Peer,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Endpoint {
    pub entity: String,
    pub role: Role,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub layer: String,
    pub endpoints: Vec<Endpoint>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub color: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub entities: Vec<Entity>,
    pub connections: Vec<Connection>,
    pub layers: Vec<Layer>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct View {
    #[serde(default)]
    pub positions: BTreeMap<String, Point>,
    #[serde(default)]
    pub collapsed: BTreeSet<String>,
    #[serde(default)]
    pub hidden_layers: BTreeSet<String>,
}
#[derive(Serialize, Deserialize)]
pub struct LayoutRequest {
    pub document: Document,
    pub view: View,
}
#[derive(Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub position: Point,
    pub children: usize,
}
#[derive(Serialize, Deserialize)]
pub struct Branch {
    pub entity: String,
    pub role: Role,
    pub points: Vec<Point>,
}
#[derive(Serialize, Deserialize)]
pub struct Bundle {
    pub layer: String,
    pub connections: Vec<String>,
    pub branches: Vec<Branch>,
    pub junction: Point,
}
#[derive(Serialize, Deserialize)]
pub struct Layout {
    pub nodes: Vec<Node>,
    pub bundles: Vec<Bundle>,
    pub positions: BTreeMap<String, Point>,
}

pub fn validate(doc: &Document) -> Result<(), String> {
    if doc.entities.len() > 500 || doc.connections.len() > 2000 {
        return Err("Prototype limit: 500 entities and 2,000 connections".into());
    }
    let ids: BTreeSet<_> = doc.entities.iter().map(|e| e.id.as_str()).collect();
    if ids.len() != doc.entities.len() || ids.contains("") {
        return Err("Entity IDs must be nonempty and unique".into());
    }
    let layers: BTreeSet<_> = doc.layers.iter().map(|l| l.id.as_str()).collect();
    if layers.len() != doc.layers.len() {
        return Err("Duplicate layer IDs".into());
    }
    for layer in &doc.layers {
        if layer.id.is_empty()
            || !layer
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            || layer.color.len() != 7
            || !layer.color.starts_with('#')
            || !layer.color[1..].bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Layers need a simple ID and a six-digit hex color".into());
        }
    }
    let mut connection_ids = BTreeSet::new();
    for e in &doc.entities {
        if e.name.trim().is_empty() {
            return Err("An entity needs a name".into());
        }
        let mut seen = BTreeSet::new();
        let mut current = Some(e.id.as_str());
        while let Some(id) = current {
            if !seen.insert(id) {
                return Err("Hierarchy contains a cycle".into());
            }
            let entity = doc
                .entities
                .iter()
                .find(|n| n.id == id)
                .ok_or("Unknown parent")?;
            current = entity.parent.as_deref();
        }
    }
    for c in &doc.connections {
        if c.id.is_empty() || !connection_ids.insert(&c.id) {
            return Err("Connection IDs must be unique".into());
        }
        if !layers.contains(c.layer.as_str()) || c.endpoints.len() < 2 {
            return Err("Connection needs a layer and at least two endpoints".into());
        }
        let mut endpoints = BTreeSet::new();
        for ep in &c.endpoints {
            if !ids.contains(ep.entity.as_str()) || !endpoints.insert(&ep.entity) {
                return Err("Unknown or duplicate endpoint".into());
            }
        }
    }
    Ok(())
}

pub fn layout(req: &LayoutRequest) -> Result<Layout, String> {
    validate(&req.document)?;
    if req
        .view
        .positions
        .values()
        .any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e6 || p.y.abs() > 1e6)
    {
        return Err("Invalid coordinates".into());
    }
    let doc = &req.document;
    let view = &req.view;
    let mut positions = view.positions.clone();
    // Allocate all nodes before projection. Collapse/filters therefore never shift neighbors.
    fn place(
        parent: Option<&str>,
        depth: usize,
        doc: &Document,
        positions: &mut BTreeMap<String, Point>,
        rows: &mut BTreeMap<usize, usize>,
    ) {
        for e in doc
            .entities
            .iter()
            .filter(|e| e.parent.as_deref() == parent)
        {
            let row = rows.entry(depth).or_default();
            let mut p = Point {
                x: 60. + depth as f64 * 280.,
                y: 60. + *row as f64 * 116.,
            };
            if !positions.contains_key(&e.id) {
                while positions
                    .values()
                    .any(|q| (q.x - p.x).abs() < 200. && (q.y - p.y).abs() < 90.)
                {
                    p.y += 116.;
                }
                positions.insert(e.id.clone(), p);
            }
            *row += 1;
            place(Some(&e.id), depth + 1, doc, positions, rows);
        }
    }
    place(None, 0, doc, &mut positions, &mut BTreeMap::new());
    let visible_ancestor = |id: &str| -> String {
        let mut result = id.to_string();
        let mut current = doc
            .entities
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.parent.as_deref());
        while let Some(p) = current {
            if view.collapsed.contains(p) {
                result = p.into();
            }
            current = doc
                .entities
                .iter()
                .find(|e| e.id == p)
                .and_then(|e| e.parent.as_deref());
        }
        result
    };
    let nodes: Vec<_> = doc
        .entities
        .iter()
        .filter(|e| visible_ancestor(&e.id) == e.id)
        .map(|e| Node {
            id: e.id.clone(),
            position: positions[&e.id],
            children: doc
                .entities
                .iter()
                .filter(|n| n.parent.as_deref() == Some(&e.id))
                .count(),
        })
        .collect();
    let obstacles: Vec<_> = nodes.iter().map(|n| n.position).collect();
    type Key = (String, Vec<(String, Role)>);
    let mut groups: BTreeMap<Key, Vec<String>> = BTreeMap::new();
    for c in &doc.connections {
        if view.hidden_layers.contains(&c.layer) {
            continue;
        }
        let mut eps: Vec<_> = c
            .endpoints
            .iter()
            .map(|ep| (visible_ancestor(&ep.entity), ep.role.clone()))
            .collect();
        eps.sort();
        eps.dedup();
        if eps.iter().map(|(id, _)| id).collect::<BTreeSet<_>>().len() < 2 {
            continue;
        }
        groups
            .entry((c.layer.clone(), eps))
            .or_default()
            .push(c.id.clone());
    }
    let mut bundles = Vec::new();
    for (channel, ((layer, eps), connections)) in groups.into_iter().enumerate() {
        let mut xs: Vec<_> = eps.iter().map(|(id, _)| positions[id].x + 100.).collect();
        xs.sort_by(f64::total_cmp);
        let mut ys: Vec<_> = eps.iter().map(|(id, _)| positions[id].y + 36.).collect();
        ys.sort_by(f64::total_cmp);
        let mut junction = Point {
            x: xs[xs.len() - 1] + 140. + channel as f64 * 20.,
            y: ys[ys.len() / 2],
        };
        while obstacles.iter().any(|p| inside(junction, *p, 12.)) {
            junction.x += 32.;
        }
        let branches = eps
            .into_iter()
            .map(|(id, role)| {
                let p = positions[&id];
                let start = Point {
                    x: if junction.x >= p.x + 100. {
                        p.x + 200.
                    } else {
                        p.x
                    },
                    y: p.y + 36.,
                };
                let lead = Point {
                    x: start.x + if start.x > p.x { 16. } else { -16. },
                    y: start.y,
                };
                // Prefer this bundle's own trunk lane, avoiding the visual lie of
                // unrelated connections sharing the same vertical bus. Fall back
                // to obstacle routing when a card obstructs the direct approach.
                let elbow = Point {
                    x: junction.x,
                    y: lead.y,
                };
                let mut points =
                    if !blocked(lead, elbow, &obstacles) && !blocked(elbow, junction, &obstacles) {
                        let mut path = vec![lead, elbow, junction];
                        path.dedup();
                        path
                    } else {
                        route(lead, junction, &obstacles)
                    };
                if !points.is_empty() {
                    points.insert(0, start);
                }
                Branch {
                    entity: id,
                    role,
                    points,
                }
            })
            .collect();
        bundles.push(Bundle {
            layer,
            connections,
            branches,
            junction,
        });
    }
    Ok(Layout {
        nodes,
        bundles,
        positions,
    })
}
fn inside(p: Point, r: Point, pad: f64) -> bool {
    p.x > r.x - pad && p.x < r.x + 200. + pad && p.y > r.y - pad && p.y < r.y + 72. + pad
}
fn blocked(a: Point, b: Point, rects: &[Point]) -> bool {
    rects.iter().any(|r| {
        if a.x == b.x {
            a.x > r.x - 10.
                && a.x < r.x + 210.
                && a.y.min(b.y) < r.y + 82.
                && a.y.max(b.y) > r.y - 10.
        } else {
            a.y > r.y - 10.
                && a.y < r.y + 82.
                && a.x.min(b.x) < r.x + 210.
                && a.x.max(b.x) > r.x - 10.
        }
    })
}
/// Rectilinear visibility grid + Dijkstra with a bend penalty. Obstacles are cards.
fn route(start: Point, end: Point, rects: &[Point]) -> Vec<Point> {
    let mut xs = vec![start.x, end.x];
    let mut ys = vec![start.y, end.y];
    for r in rects {
        xs.extend([r.x - 16., r.x + 216.]);
        ys.extend([r.y - 16., r.y + 88.]);
    }
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let w = xs.len();
    let h = ys.len();
    let point = |i: usize| Point {
        x: xs[i % w],
        y: ys[i / w],
    };
    let index = |p: Point| {
        ys.iter().position(|v| *v == p.y).unwrap() * w + xs.iter().position(|v| *v == p.x).unwrap()
    };
    let s = index(start);
    let t = index(end);
    let mut dist = vec![u64::MAX; w * h * 3];
    let mut prev = vec![usize::MAX; dist.len()];
    let mut heap = BinaryHeap::new();
    dist[s * 3] = 0;
    heap.push(std::cmp::Reverse((0u64, s * 3)));
    let mut finish = None;
    while let Some(std::cmp::Reverse((cost, state))) = heap.pop() {
        if dist[state] != cost {
            continue;
        }
        let i = state / 3;
        let dir = state % 3;
        if i == t {
            finish = Some(state);
            break;
        }
        let mut neighbors = Vec::new();
        if i % w > 0 {
            neighbors.push((i - 1, 1));
        }
        if i % w + 1 < w {
            neighbors.push((i + 1, 1));
        }
        if i >= w {
            neighbors.push((i - w, 2));
        }
        if i + w < w * h {
            neighbors.push((i + w, 2));
        }
        for (j, d) in neighbors {
            let a = point(i);
            let b = point(j);
            if blocked(a, b, rects) {
                continue;
            }
            let c = cost
                + ((a.x - b.x).abs() + (a.y - b.y).abs()).round() as u64
                + if dir != 0 && dir != d { 28 } else { 0 };
            let next = j * 3 + d;
            if c < dist[next] {
                dist[next] = c;
                prev[next] = state;
                heap.push(std::cmp::Reverse((c, next)));
            }
        }
    }
    let Some(mut at) = finish else {
        return Vec::new();
    };
    let mut path = Vec::new();
    loop {
        path.push(point(at / 3));
        if prev[at] == usize::MAX {
            break;
        }
        at = prev[at];
    }
    path.reverse();
    let mut simple: Vec<Point> = Vec::new();
    for p in path {
        if simple.len() >= 2 {
            let a = simple[simple.len() - 2];
            let b = simple[simple.len() - 1];
            if (a.x == b.x && b.x == p.x) || (a.y == b.y && b.y == p.y) {
                simple.pop();
            }
        }
        simple.push(p);
    }
    simple
}
#[wasm_bindgen]
pub fn compute(input: &str) -> Result<String, JsError> {
    let request: LayoutRequest = serde_json::from_str(input)?;
    let result = layout(&request).map_err(|e| JsError::new(&e))?;
    Ok(serde_json::to_string(&result)?)
}

pub fn sample() -> Document {
    serde_json::from_str(include_str!("../../../sample.json")).expect("sample")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parallel_connections_bundle_after_collapse_and_keep_roles() {
        let mut doc = sample();
        let mut duplicate = doc.connections[0].clone();
        duplicate.id = "second-circuit".into();
        doc.connections.push(duplicate);
        let mut view = View::default();
        view.collapsed.insert("kitchen".into());
        let result = layout(&LayoutRequest {
            document: doc,
            view,
        })
        .unwrap();
        let bundle = result
            .bundles
            .iter()
            .find(|b| b.connections.len() == 2)
            .unwrap();
        assert_eq!(bundle.branches.len(), 2);
        assert!(
            bundle
                .branches
                .iter()
                .any(|b| b.entity == "panel" && b.role == Role::Source)
        );
        assert!(
            bundle
                .branches
                .iter()
                .any(|b| b.entity == "kitchen" && b.role == Role::Target)
        );
        assert!(
            result
                .bundles
                .iter()
                .flat_map(|b| &b.branches)
                .all(|b| b.points.len() >= 2)
        );
    }
    #[test]
    fn collapse_and_filters_preserve_positions_and_project_bundles() {
        let doc = sample();
        let mut view = View::default();
        let first = layout(&LayoutRequest {
            document: doc.clone(),
            view: view.clone(),
        })
        .unwrap();
        view.positions = first.positions.clone();
        view.collapsed.insert("kitchen".into());
        let collapsed = layout(&LayoutRequest {
            document: doc.clone(),
            view: view.clone(),
        })
        .unwrap();
        assert_eq!(first.positions, collapsed.positions);
        assert!(!collapsed.nodes.iter().any(|n| n.id == "lamp"));
        assert!(
            collapsed
                .bundles
                .iter()
                .any(|b| b.branches.iter().any(|ep| ep.entity == "kitchen"))
        );
        view.hidden_layers.insert("power".into());
        assert!(
            !layout(&LayoutRequest {
                document: doc,
                view
            })
            .unwrap()
            .bundles
            .iter()
            .any(|b| b.layer == "power")
        );
    }
    #[test]
    fn rejects_cycles_and_unknown_endpoints() {
        let mut doc = sample();
        doc.entities[0].parent = Some("lamp".into());
        assert!(validate(&doc).is_err());
        doc = sample();
        doc.connections[0].endpoints[0].entity = "missing".into();
        assert!(validate(&doc).is_err());
    }
    #[test]
    fn routing_avoids_obstacles_and_is_orthogonal() {
        let rects = [Point { x: 100., y: 0. }];
        let path = route(Point { x: 0., y: 36. }, Point { x: 400., y: 36. }, &rects);
        assert!(path.len() >= 4);
        for p in path.windows(2) {
            assert!(p[0].x == p[1].x || p[0].y == p[1].y);
            assert!(!blocked(p[0], p[1], &rects));
        }
    }
}
