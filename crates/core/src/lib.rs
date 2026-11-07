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
    /// Version 2 stores positions relative to the immediate parent.
    #[serde(default)]
    pub layout_version: u32,
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
    pub width: f64,
    pub height: f64,
    pub expanded_width: f64,
    pub expanded_height: f64,
    pub depth: usize,
    pub container: bool,
    pub collapsed: bool,
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
    pub layout_version: u32,
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

const PADDING: f64 = 24.;
const CONTENT_Y: f64 = 76.;
const GAP: f64 = 32.;
const CARD_WIDTH: f64 = 200.;
const CARD_HEIGHT: f64 = 72.;

#[derive(Clone, Copy, Debug)]
struct Size {
    width: f64,
    height: f64,
}
#[derive(Clone, Copy, Debug)]
struct Rect {
    position: Point,
    width: f64,
    height: f64,
}
impl Rect {
    fn contains(&self, p: Point, pad: f64) -> bool {
        p.x > self.position.x - pad
            && p.x < self.position.x + self.width + pad
            && p.y > self.position.y - pad
            && p.y < self.position.y + self.height + pad
    }
}
fn validate_view(view: &View) -> Result<(), String> {
    if view
        .positions
        .values()
        .any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e6 || p.y.abs() > 1e6)
    {
        return Err("Invalid coordinates".into());
    }
    Ok(())
}

/// Measure the full hierarchy before applying collapse. Sibling slots remain
/// reserved, so folding a room never moves another room or loses hidden offsets.
fn measure(
    id: &str,
    depth: usize,
    doc: &Document,
    positions: &mut BTreeMap<String, Point>,
    sizes: &mut BTreeMap<String, Size>,
) -> Size {
    let entity = doc.entities.iter().find(|e| e.id == id).unwrap();
    let children: Vec<_> = doc
        .entities
        .iter()
        .filter(|e| e.parent.as_deref() == Some(id))
        .collect();
    if children.is_empty() && entity.kind != "space" {
        let size = Size {
            width: CARD_WIDTH,
            height: CARD_HEIGHT,
        };
        sizes.insert(id.into(), size);
        return size;
    }
    let child_sizes: Vec<_> = children
        .iter()
        .map(|e| measure(&e.id, depth + 1, doc, positions, sizes))
        .collect();
    let has_containers = children.iter().any(|e| {
        e.kind == "space"
            || doc
                .entities
                .iter()
                .any(|c| c.parent.as_deref() == Some(&e.id))
    });
    let columns = if depth == 0 {
        1
    } else if has_containers {
        children.len().clamp(1, 3)
    } else if children.len() > 5 {
        2
    } else {
        1
    };
    let mut widths = vec![0_f64; columns];
    let mut heights = vec![0_f64; children.len().div_ceil(columns)];
    for (i, size) in child_sizes.iter().enumerate() {
        widths[i % columns] = widths[i % columns].max(size.width);
        heights[i / columns] = heights[i / columns].max(size.height);
    }
    let mut extent = Size {
        width: 272.,
        height: 148.,
    };
    let mut occupied: Vec<Rect> = Vec::new();
    // Reserve existing children before placing newly added entities.
    for (e, size) in children.iter().zip(&child_sizes) {
        if let Some(p) = positions.get_mut(&e.id) {
            p.x = p.x.max(PADDING);
            p.y = p.y.max(CONTENT_Y);
            occupied.push(Rect {
                position: *p,
                width: size.width,
                height: size.height,
            });
        }
    }
    for (i, (e, size)) in children.iter().zip(&child_sizes).enumerate() {
        let mut position = Point {
            x: PADDING + widths[..i % columns].iter().map(|w| w + GAP).sum::<f64>(),
            y: CONTENT_Y + heights[..i / columns].iter().map(|h| h + GAP).sum::<f64>(),
        };
        if let Some(p) = positions.get(&e.id) {
            position = *p;
        } else {
            while occupied.iter().any(|r| {
                position.x < r.position.x + r.width + 16.
                    && position.x + size.width + 16. > r.position.x
                    && position.y < r.position.y + r.height + 16.
                    && position.y + size.height + 16. > r.position.y
            }) {
                position.y += size.height + GAP;
            }
            occupied.push(Rect {
                position,
                width: size.width,
                height: size.height,
            });
            positions.insert(e.id.clone(), position);
        }
        extent.width = extent.width.max(position.x + size.width + PADDING);
        extent.height = extent.height.max(position.y + size.height + PADDING);
    }
    // Keep the initial expanded footprint when a child is dragged inward.
    extent.width = extent
        .width
        .max(PADDING * 2. + widths.iter().sum::<f64>() + GAP * (columns - 1) as f64);
    extent.height = extent.height.max(
        CONTENT_Y
            + heights.iter().sum::<f64>()
            + GAP * heights.len().saturating_sub(1) as f64
            + PADDING,
    );
    sizes.insert(id.into(), extent);
    extent
}

pub fn layout(req: &LayoutRequest) -> Result<Layout, String> {
    validate(&req.document)?;
    validate_view(&req.view)?;
    let doc = &req.document;
    let view = &req.view;
    // Old flat-card coordinates cannot describe containment. Rebuild their
    // geometry once; keep the view's fold/layer choices and the inventory.
    let mut positions = if view.layout_version >= 2 {
        view.positions.clone()
    } else {
        BTreeMap::new()
    };
    positions.retain(|id, _| doc.entities.iter().any(|e| &e.id == id));
    let mut sizes = BTreeMap::new();
    let roots: Vec<_> = doc.entities.iter().filter(|e| e.parent.is_none()).collect();
    let mut root_y = 40.;
    for root in &roots {
        let size = measure(&root.id, 0, doc, &mut positions, &mut sizes);
        let p = positions
            .entry(root.id.clone())
            .or_insert(Point { x: 40., y: root_y });
        root_y = p.y + size.height + GAP;
    }
    fn collect(
        id: &str,
        origin: Point,
        depth: usize,
        doc: &Document,
        view: &View,
        positions: &BTreeMap<String, Point>,
        sizes: &BTreeMap<String, Size>,
        nodes: &mut Vec<Node>,
    ) {
        let e = doc.entities.iter().find(|e| e.id == id).unwrap();
        let local = positions[id];
        let position = Point {
            x: origin.x + local.x,
            y: origin.y + local.y,
        };
        let children: Vec<_> = doc
            .entities
            .iter()
            .filter(|e| e.parent.as_deref() == Some(id))
            .collect();
        let container = e.kind == "space" || !children.is_empty();
        let collapsed = container && view.collapsed.contains(id);
        let full = sizes[id];
        nodes.push(Node {
            id: id.into(),
            position,
            children: children.len(),
            width: if collapsed { CARD_WIDTH } else { full.width },
            height: if collapsed { CARD_HEIGHT } else { full.height },
            expanded_width: full.width,
            expanded_height: full.height,
            depth,
            container,
            collapsed,
        });
        if !collapsed {
            for child in children {
                collect(
                    &child.id,
                    position,
                    depth + 1,
                    doc,
                    view,
                    positions,
                    sizes,
                    nodes,
                );
            }
        }
    }
    let mut nodes = Vec::new();
    for root in roots {
        collect(
            &root.id,
            Point::default(),
            0,
            doc,
            view,
            &positions,
            &sizes,
            &mut nodes,
        );
    }
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
    // Expanded space interiors are traversable. Headers, devices and collapsed
    // spaces are obstacles; crossing a space boundary is allowed, never its title.
    let obstacles: Vec<_> = nodes
        .iter()
        .map(|n| Rect {
            position: n.position,
            width: n.width,
            height: if n.container && !n.collapsed {
                52.
            } else {
                n.height
            },
        })
        .collect();
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
    let node_for = |id: &str| nodes.iter().find(|n| n.id == id).unwrap();
    let ancestors = |id: &str| {
        let mut path = Vec::new();
        let mut at = doc
            .entities
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.parent.as_deref());
        while let Some(id) = at {
            path.push(id.to_string());
            at = doc
                .entities
                .iter()
                .find(|e| e.id == id)
                .and_then(|e| e.parent.as_deref());
        }
        path
    };
    let mut bundles = Vec::new();
    for (channel, ((layer, eps), connections)) in groups.into_iter().enumerate() {
        let first_path = ancestors(&eps[0].0);
        let common = first_path
            .iter()
            .find(|ancestor| eps.iter().all(|(id, _)| ancestors(id).contains(ancestor)))
            .map(|id| node_for(id));
        let mut ys: Vec<_> = eps
            .iter()
            .map(|(id, _)| {
                let n = node_for(id);
                n.position.y
                    + if n.container && !n.collapsed {
                        26.
                    } else {
                        36.
                    }
            })
            .collect();
        ys.sort_by(f64::total_cmp);
        let mut junction = if let Some(parent) = common {
            Point {
                x: parent.position.x + parent.width - 12. - (channel % 3) as f64 * 3.,
                y: ys[ys.len() / 2],
            }
        } else {
            Point {
                x: eps
                    .iter()
                    .map(|(id, _)| {
                        let n = node_for(id);
                        n.position.x + n.width
                    })
                    .fold(f64::NEG_INFINITY, f64::max)
                    + 40.
                    + channel as f64 * 20.,
                y: ys[ys.len() / 2],
            }
        };
        while obstacles.iter().any(|r| r.contains(junction, 8.)) {
            junction.y += 16.;
        }
        let branches = eps
            .into_iter()
            .map(|(id, role)| {
                let n = node_for(&id);
                let center = n.position.x + n.width / 2.;
                let start = Point {
                    x: if junction.x >= center {
                        n.position.x + n.width
                    } else {
                        n.position.x
                    },
                    y: n.position.y
                        + if n.container && !n.collapsed {
                            26.
                        } else {
                            36.
                        },
                };
                let lead = Point {
                    x: start.x + if junction.x >= center { 12. } else { -12. },
                    y: start.y,
                };
                let elbow = Point {
                    x: junction.x,
                    y: lead.y,
                };
                let mut points =
                    if !blocked(lead, elbow, &obstacles) && !blocked(elbow, junction, &obstacles) {
                        let mut p = vec![lead, elbow, junction];
                        p.dedup();
                        p
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
        layout_version: 2,
        nodes,
        bundles,
        positions,
    })
}

#[derive(Serialize, Deserialize)]
pub struct MoveRequest {
    pub document: Document,
    pub view: View,
    pub entity: String,
    pub position: Point,
}
/// Move in world coordinates while retaining parent-relative offsets. Dragging
/// a space moves its hidden and visible descendants without rewriting them.
pub fn move_entity(req: &MoveRequest) -> Result<View, String> {
    if !req.position.x.is_finite()
        || !req.position.y.is_finite()
        || req.position.x.abs() > 1e6
        || req.position.y.abs() > 1e6
    {
        return Err("Invalid coordinates".into());
    }
    let layout = layout(&LayoutRequest {
        document: req.document.clone(),
        view: req.view.clone(),
    })?;
    let node = layout
        .nodes
        .iter()
        .find(|n| n.id == req.entity)
        .ok_or("Move a visible entity")?;
    let e = req
        .document
        .entities
        .iter()
        .find(|e| e.id == req.entity)
        .unwrap();
    let mut p = req.position;
    if let Some(parent) = &e.parent {
        let parent = layout
            .nodes
            .iter()
            .find(|n| &n.id == parent)
            .ok_or("Parent is hidden")?;
        p.x = (p.x - parent.position.x).clamp(
            PADDING,
            (parent.width - node.expanded_width - PADDING).max(PADDING),
        );
        p.y = (p.y - parent.position.y).clamp(
            CONTENT_Y,
            (parent.height - node.expanded_height - PADDING).max(CONTENT_Y),
        );
    }
    let mut view = req.view.clone();
    view.layout_version = 2;
    view.positions = layout.positions;
    view.positions.insert(req.entity.clone(), p);
    Ok(view)
}
#[wasm_bindgen]
pub fn move_node(input: &str) -> Result<String, JsError> {
    let request: MoveRequest = serde_json::from_str(input)?;
    let result = move_entity(&request).map_err(|e| JsError::new(&e))?;
    Ok(serde_json::to_string(&result)?)
}

fn blocked(a: Point, b: Point, rects: &[Rect]) -> bool {
    rects.iter().any(|r| {
        if a.x == b.x {
            a.x > r.position.x - 8.
                && a.x < r.position.x + r.width + 8.
                && a.y.min(b.y) < r.position.y + r.height + 8.
                && a.y.max(b.y) > r.position.y - 8.
        } else {
            a.y > r.position.y - 8.
                && a.y < r.position.y + r.height + 8.
                && a.x.min(b.x) < r.position.x + r.width + 8.
                && a.x.max(b.x) > r.position.x - 8.
        }
    })
}
/// Rectilinear visibility grid + Dijkstra with a bend penalty. Obstacles are cards.
fn route(start: Point, end: Point, rects: &[Rect]) -> Vec<Point> {
    let mut xs = vec![start.x, end.x];
    let mut ys = vec![start.y, end.y];
    for r in rects {
        xs.extend([r.position.x - 12., r.position.x + r.width + 12.]);
        ys.extend([r.position.y - 12., r.position.y + r.height + 12.]);
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
    fn hierarchy_is_contained_and_siblings_do_not_overlap() {
        let doc = sample();
        let result = layout(&LayoutRequest {
            document: doc.clone(),
            view: View::default(),
        })
        .unwrap();
        for node in &result.nodes {
            let entity = doc.entities.iter().find(|e| e.id == node.id).unwrap();
            if let Some(parent) = &entity.parent {
                let parent = result.nodes.iter().find(|n| &n.id == parent).unwrap();
                assert!(node.position.x >= parent.position.x + PADDING);
                assert!(node.position.y >= parent.position.y + CONTENT_Y);
                assert!(node.position.x + node.width <= parent.position.x + parent.width - PADDING);
                assert!(
                    node.position.y + node.height <= parent.position.y + parent.height - PADDING
                );
            }
            for other in &result.nodes {
                let other_entity = doc.entities.iter().find(|e| e.id == other.id).unwrap();
                if node.id != other.id && entity.parent == other_entity.parent {
                    assert!(
                        node.position.x + node.width <= other.position.x
                            || other.position.x + other.width <= node.position.x
                            || node.position.y + node.height <= other.position.y
                            || other.position.y + other.height <= node.position.y
                    );
                }
            }
        }
        assert!(
            result
                .bundles
                .iter()
                .flat_map(|b| &b.branches)
                .all(|b| !b.points.is_empty())
        );
    }
    #[test]
    fn moving_a_space_translates_its_subtree_and_clamps_to_parent() {
        let doc = sample();
        let first = layout(&LayoutRequest {
            document: doc.clone(),
            view: View::default(),
        })
        .unwrap();
        let view = View {
            layout_version: 2,
            positions: first.positions.clone(),
            ..View::default()
        };
        let kitchen = first.nodes.iter().find(|n| n.id == "kitchen").unwrap();
        let moved = move_entity(&MoveRequest {
            document: doc.clone(),
            view: view.clone(),
            entity: "kitchen".into(),
            position: Point {
                x: kitchen.position.x + 20.,
                y: kitchen.position.y + 20.,
            },
        })
        .unwrap();
        let result = layout(&LayoutRequest {
            document: doc.clone(),
            view: moved.clone(),
        })
        .unwrap();
        for node in &first.nodes {
            let after = result.nodes.iter().find(|n| n.id == node.id).unwrap();
            let delta = if ["kitchen", "lamp", "sensor", "tap"].contains(&node.id.as_str()) {
                20.
            } else {
                0.
            };
            assert_eq!(
                after.position,
                Point {
                    x: node.position.x + delta,
                    y: node.position.y + delta
                }
            );
        }
        assert_eq!(moved.positions["lamp"], view.positions["lamp"]);
        let clamped = move_entity(&MoveRequest {
            document: doc.clone(),
            view: moved,
            entity: "lamp".into(),
            position: Point { x: -999., y: -999. },
        })
        .unwrap();
        assert_eq!(
            clamped.positions["lamp"],
            Point {
                x: PADDING,
                y: CONTENT_Y
            }
        );
    }
    #[test]
    fn flat_views_migrate_without_losing_fold_or_layer_choices() {
        let mut old = View::default();
        old.positions
            .insert("lamp".into(), Point { x: 9999., y: 9999. });
        old.collapsed.insert("kitchen".into());
        old.hidden_layers.insert("water".into());
        let result = layout(&LayoutRequest {
            document: sample(),
            view: old,
        })
        .unwrap();
        assert_eq!(result.layout_version, 2);
        assert!(result.positions["lamp"].x < 9999.);
        assert!(!result.nodes.iter().any(|n| n.id == "lamp"));
        assert!(!result.bundles.iter().any(|b| b.layer == "water"));
    }
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
        view.layout_version = 2;
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
        let rects = [Rect {
            position: Point { x: 100., y: 0. },
            width: 200.,
            height: 72.,
        }];
        let path = route(Point { x: 0., y: 36. }, Point { x: 400., y: 36. }, &rects);
        assert!(path.len() >= 4);
        for p in path.windows(2) {
            assert!(p[0].x == p[1].x || p[0].y == p[1].y);
            assert!(!blocked(p[0], p[1], &rects));
        }
    }
}
