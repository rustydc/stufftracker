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
    pub unrouted_connections: Vec<String>,
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
    let mut reserved = Vec::new();
    let mut bundles = Vec::new();
    for (channel, ((layer, eps), connections)) in groups.into_iter().enumerate() {
        let router = HierarchyRouter {
            document: doc,
            nodes: &nodes,
            tracks: &reserved,
        };
        let first_path = router.ancestors(&eps[0].0);
        let common = first_path
            .iter()
            .find(|ancestor| {
                eps.iter()
                    .all(|(id, _)| router.ancestors(id).contains(ancestor))
            })
            .map(String::as_str);
        let centers: Vec<_> = eps.iter().map(|(id, _)| router.center(id)).collect();
        let junction = router.junction(common, &centers, channel);
        let mut gates: BTreeMap<String, Gate> = BTreeMap::new();
        let spaces: BTreeSet<_> = eps
            .iter()
            .flat_map(|(id, _)| {
                router
                    .ancestors(id)
                    .into_iter()
                    .take_while(|id| Some(id.as_str()) != common)
            })
            .collect();
        let mut spaces: Vec<_> = spaces.into_iter().collect();
        spaces.sort_by_key(|id| router.node(id).depth);
        // Choose outer gateways first. An inner gateway is scored against the
        // actual route to its parent's gateway, not a straight-line shortcut
        // through the parent's obstacles or header.
        for ancestor in spaces {
            let target = router
                .parent(&ancestor)
                .and_then(|id| gates.get(id))
                .map(|gate| gate.inside)
                .unwrap_or(junction);
            let members: Vec<_> = eps
                .iter()
                .filter(|(id, _)| router.ancestors(id).contains(&ancestor))
                .map(|(id, _)| router.center(id))
                .collect();
            if let Some(gate) = router.gate(&ancestor, target, &members) {
                gates.insert(ancestor, gate);
            }
        }
        let mut branches: Vec<Branch> = eps
            .into_iter()
            .map(|(id, role)| {
                let points = router.branch(&id, common, junction, &gates);
                Branch {
                    entity: id,
                    role,
                    points,
                }
            })
            .collect();
        // Do not display a partial hyperedge as though it were complete. Keep
        // hard spacing when a corridor is full and report the missing connection.
        if branches.iter().any(|branch| branch.points.is_empty()) {
            for branch in &mut branches {
                branch.points.clear();
            }
        }
        // Reserve complete bundles only: their own branches may share trunks,
        // but every subsequent connection must use a distinct parallel track.
        for branch in &branches {
            for pair in branch.points.windows(2) {
                let segment = Track::new(pair[0], pair[1]);
                if segment.a != segment.b && !reserved.contains(&segment) {
                    reserved.push(segment);
                }
            }
        }
        bundles.push(Bundle {
            layer,
            connections,
            branches,
            junction,
        });
    }
    let unrouted_connections = bundles
        .iter()
        .filter(|b| b.branches.iter().any(|branch| branch.points.is_empty()))
        .flat_map(|b| b.connections.clone())
        .collect();
    Ok(Layout {
        layout_version: 2,
        unrouted_connections,
        nodes,
        bundles,
        positions,
    })
}

/// Route one level at a time. Within a space, its immediate children are
/// solid obstacles. A branch crosses each ancestor through one shared gateway
/// and can never use an unrelated room (or re-enter an exited room) as a shortcut.
struct HierarchyRouter<'a> {
    document: &'a Document,
    nodes: &'a [Node],
    tracks: &'a [Track],
}
/// Centerline separation between independent parallel runs. With a 2-unit
/// stroke this leaves 8 units of visible padding. Opposing flows share the same
/// geometric rule; deliberately bundled branches are reserved as one connection.
pub const TRACK_SPACING: f64 = 10.;
#[derive(Clone, Copy, Debug, PartialEq)]
struct Track {
    a: Point,
    b: Point,
}
impl Track {
    fn new(a: Point, b: Point) -> Self {
        if a.x > b.x || (a.x == b.x && a.y > b.y) {
            Self { a: b, b: a }
        } else {
            Self { a, b }
        }
    }
    fn horizontal(&self) -> bool {
        self.a.y == self.b.y
    }
    fn distance_to(&self, p: Point) -> f64 {
        if self.horizontal() {
            (p.y - self.a.y).abs() + (self.a.x - p.x).max(0.) + (p.x - self.b.x).max(0.)
        } else {
            (p.x - self.a.x).abs() + (self.a.y - p.y).max(0.) + (p.y - self.b.y).max(0.)
        }
    }
}
fn track_conflict(a: Point, b: Point, tracks: &[Track]) -> bool {
    let current = Track::new(a, b);
    if a == b {
        return false;
    }
    tracks.iter().any(|t| {
        if current.horizontal() != t.horizontal() {
            return false;
        }
        if current.horizontal() {
            (current.a.y - t.a.y).abs() < TRACK_SPACING - 1e-6
                && current.a.x.max(t.a.x) < current.b.x.min(t.b.x) - 1e-6
        } else {
            (current.a.x - t.a.x).abs() < TRACK_SPACING - 1e-6
                && current.a.y.max(t.a.y) < current.b.y.min(t.b.y) - 1e-6
        }
    })
}
fn point_clear(p: Point, tracks: &[Track]) -> bool {
    tracks
        .iter()
        .all(|t| t.distance_to(p) >= TRACK_SPACING - 1e-6)
}
fn offsets(center: f64, min: f64, max: f64) -> Vec<f64> {
    let mut result = Vec::new();
    for i in [0., -1., 1., -2., 2., -3., 3., -4., 4.] {
        let p = (center + i * TRACK_SPACING).clamp(min, max);
        if !result.contains(&p) {
            result.push(p);
        }
    }
    result
}
#[derive(Clone, Copy)]
struct Gate {
    inside: Point,
    boundary: Point,
    outside: Point,
}
fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}
fn simplify(path: Vec<Point>) -> Vec<Point> {
    let mut result: Vec<Point> = Vec::new();
    for p in path {
        if result.last() == Some(&p) {
            continue;
        }
        if result.len() >= 2 {
            let a = result[result.len() - 2];
            let b = result[result.len() - 1];
            if (a.x == b.x && b.x == p.x) || (a.y == b.y && b.y == p.y) {
                result.pop();
            }
        }
        result.push(p);
    }
    result
}
impl HierarchyRouter<'_> {
    fn node(&self, id: &str) -> &Node {
        self.nodes.iter().find(|n| n.id == id).unwrap()
    }
    fn parent(&self, id: &str) -> Option<&str> {
        self.document
            .entities
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.parent.as_deref())
    }
    fn ancestors(&self, id: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut at = self.parent(id);
        while let Some(id) = at {
            result.push(id.into());
            at = self.parent(id);
        }
        result
    }
    fn center(&self, id: &str) -> Point {
        let n = self.node(id);
        Point {
            x: n.position.x + n.width / 2.,
            y: n.position.y
                + if n.container && !n.collapsed {
                    26.
                } else {
                    n.height / 2.
                },
        }
    }
    fn bounds(&self, domain: Option<&str>) -> Option<Rect> {
        domain.map(|id| {
            let n = self.node(id);
            Rect {
                position: Point {
                    x: n.position.x,
                    y: n.position.y + 60.,
                },
                width: n.width,
                height: n.height - 60.,
            }
        })
    }
    fn obstacles(&self, domain: Option<&str>) -> Vec<Rect> {
        // The immediate child boxes include every nested space in full. Routes
        // within a room see its device boxes; floor routes see solid room boxes.
        self.nodes
            .iter()
            .filter(|n| self.parent(&n.id) == domain)
            .map(|n| Rect {
                position: n.position,
                width: n.width,
                height: n.height,
            })
            .collect()
    }
    fn junction(&self, domain: Option<&str>, centers: &[Point], channel: usize) -> Point {
        let rects = self.obstacles(domain);
        let bounds = self.bounds(domain);
        let mut xs: Vec<_> = centers.iter().map(|p| p.x).collect();
        let mut ys: Vec<_> = centers.iter().map(|p| p.y).collect();
        let lane = (channel % 3) as f64 * 2.;
        for r in &rects {
            xs.extend([
                r.position.x - 12. - lane,
                r.position.x + r.width + 12. + lane,
            ]);
            ys.extend([
                r.position.y - 12. - lane,
                r.position.y + r.height + 12. + lane,
            ]);
        }
        if let Some(b) = bounds {
            xs.extend([b.position.x + 12., b.position.x + b.width - 12.]);
            ys.extend([b.position.y + 4., b.position.y + b.height - 12.]);
        }
        for track in self.tracks {
            if track.horizontal() {
                ys.extend([track.a.y - TRACK_SPACING, track.a.y + TRACK_SPACING]);
                xs.extend([track.a.x, track.b.x]);
            } else {
                xs.extend([track.a.x - TRACK_SPACING, track.a.x + TRACK_SPACING]);
                ys.extend([track.a.y, track.b.y]);
            }
        }
        if let Some(b) = bounds {
            xs.retain(|x| *x >= b.position.x && *x <= b.position.x + b.width);
            ys.retain(|y| *y >= b.position.y && *y <= b.position.y + b.height);
        }
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        ys.sort_by(f64::total_cmp);
        ys.dedup();
        let mut best = None;
        for x in &xs {
            for y in &ys {
                let p = Point { x: *x, y: *y };
                if bounds.is_some_and(|b| !in_bounds(p, b))
                    || rects.iter().any(|r| r.contains(p, 8.))
                    || !point_clear(p, self.tracks)
                {
                    continue;
                }
                let score = centers.iter().map(|q| distance(p, *q)).sum::<f64>();
                if best.is_none_or(|(cost, _)| score < cost) {
                    best = Some((score, p));
                }
            }
        }
        best.map(|(_, p)| p).unwrap_or_else(|| {
            let n = self.nodes.first().unwrap();
            Point {
                x: n.position.x + n.width + 24.,
                y: n.position.y + n.height + 24.,
            }
        })
    }
    fn gate(&self, id: &str, junction: Point, members: &[Point]) -> Option<Gate> {
        let n = self.node(id);
        let parent = self.parent(id);
        let inside_rects = self.obstacles(Some(id));
        let outside_rects = self.obstacles(parent);
        let mut mx: Vec<_> = members.iter().map(|p| p.x).collect();
        let mut my: Vec<_> = members.iter().map(|p| p.y).collect();
        mx.sort_by(f64::total_cmp);
        my.sort_by(f64::total_cmp);
        let median = Point {
            x: mx[mx.len() / 2],
            y: my[my.len() / 2],
        };
        let top = n.position.y + 64.;
        let bottom = (n.position.y + n.height - 12.).max(top);
        let left = n.position.x;
        let right = n.position.x + n.width;
        let mut candidates = Vec::new();
        let mut ys = Vec::new();
        for y in [junction.y, median.y, top, bottom] {
            ys.extend(offsets(y, top, bottom));
        }
        ys.sort_by(f64::total_cmp);
        ys.dedup();
        for y in ys {
            for (x, normal) in [(left, -1.), (right, 1.)] {
                candidates.push(Gate {
                    inside: Point {
                        x: x - normal * 12.,
                        y,
                    },
                    boundary: Point { x, y },
                    outside: Point {
                        x: x + normal * 12.,
                        y,
                    },
                });
            }
        }
        let mut xs = Vec::new();
        for x in [junction.x, median.x, (left + right) / 2.] {
            xs.extend(offsets(x, left + 12., right - 12.));
        }
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        for x in xs {
            let y = n.position.y + n.height;
            candidates.push(Gate {
                inside: Point { x, y: y - 12. },
                boundary: Point { x, y },
                outside: Point { x, y: y + 12. },
            });
        }
        let mut candidates: Vec<_> = candidates
            .into_iter()
            .filter(|g| {
                self.bounds(Some(id)).is_none_or(|b| in_bounds(g.inside, b))
                    && self.bounds(parent).is_none_or(|b| in_bounds(g.outside, b))
                    && !inside_rects.iter().any(|r| r.contains(g.inside, 8.))
                    && !outside_rects.iter().any(|r| r.contains(g.outside, 8.))
                    && point_clear(g.boundary, self.tracks)
                    && !track_conflict(g.inside, g.outside, self.tracks)
            })
            .map(|gate| {
                let inside_cost = members
                    .iter()
                    .map(|p| distance(*p, gate.inside))
                    .sum::<f64>();
                let bound = inside_cost
                    + distance(gate.outside, junction)
                    + if gate.outside == junction { 28. } else { 56. };
                (bound, inside_cost, gate)
            })
            .collect();
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best: Option<(f64, Gate)> = None;
        for (bound, inside_cost, gate) in candidates {
            if best.as_ref().is_some_and(|(cost, _)| bound >= *cost) {
                break;
            }
            let path = self.segment(gate.outside, junction, parent);
            if path.is_empty() {
                continue;
            }
            let cost = inside_cost
                + path.windows(2).map(|p| distance(p[0], p[1])).sum::<f64>()
                + path.len() as f64 * 28.;
            if best.as_ref().is_none_or(|(previous, _)| cost < *previous) {
                best = Some((cost, gate));
            }
        }
        best.map(|(_, gate)| gate)
    }
    fn segment(&self, start: Point, end: Point, domain: Option<&str>) -> Vec<Point> {
        route(
            start,
            end,
            &self.obstacles(domain),
            self.bounds(domain),
            self.tracks,
        )
    }
    fn branch(
        &self,
        id: &str,
        common: Option<&str>,
        junction: Point,
        gates: &BTreeMap<String, Gate>,
    ) -> Vec<Point> {
        let levels: Vec<_> = self
            .ancestors(id)
            .into_iter()
            .take_while(|a| Some(a.as_str()) != common)
            .collect();
        if levels.iter().any(|id| !gates.contains_key(id)) {
            return Vec::new();
        }
        let first = levels
            .first()
            .map(|id| gates[id].inside)
            .unwrap_or(junction);
        let n = self.node(id);
        let mut ports = Vec::new();
        let mid_y = self.center(id).y;
        let (min_y, max_y) = if n.container && !n.collapsed {
            (n.position.y + 12., n.position.y + 40.)
        } else {
            (n.position.y + 12., n.position.y + n.height - 12.)
        };
        for y in offsets(mid_y, min_y, max_y) {
            ports.extend([
                (
                    Point { x: n.position.x, y },
                    Point {
                        x: n.position.x - 12.,
                        y,
                    },
                ),
                (
                    Point {
                        x: n.position.x + n.width,
                        y,
                    },
                    Point {
                        x: n.position.x + n.width + 12.,
                        y,
                    },
                ),
            ]);
        }
        if !n.container || n.collapsed {
            for x in offsets(
                n.position.x + n.width / 2.,
                n.position.x + 16.,
                n.position.x + n.width - 16.,
            ) {
                ports.extend([
                    (
                        Point { x, y: n.position.y },
                        Point {
                            x,
                            y: n.position.y - 12.,
                        },
                    ),
                    (
                        Point {
                            x,
                            y: n.position.y + n.height,
                        },
                        Point {
                            x,
                            y: n.position.y + n.height + 12.,
                        },
                    ),
                ]);
            }
        }
        ports.sort_by(|a, b| distance(a.1, first).total_cmp(&distance(b.1, first)));
        let mut best: Option<(f64, Vec<Point>)> = None;
        for (port, lead) in ports {
            if !point_clear(port, self.tracks) || track_conflict(port, lead, self.tracks) {
                continue;
            }
            if best
                .as_ref()
                .is_some_and(|(cost, _)| distance(port, first) + 56. >= *cost)
            {
                continue;
            }
            let part = self.segment(lead, first, self.parent(id));
            if part.is_empty() {
                continue;
            }
            let mut path = vec![port];
            path.extend(part);
            let path = simplify(path);
            let score = path.windows(2).map(|p| distance(p[0], p[1])).sum::<f64>()
                + path.len() as f64 * 28.;
            if best.as_ref().is_none_or(|(cost, _)| score < *cost) {
                best = Some((score, path));
            }
        }
        let Some((_, mut path)) = best else {
            return Vec::new();
        };
        let mut at = first;
        for (i, level) in levels.iter().enumerate() {
            let gate = gates[level];
            if i > 0 {
                let part = self.segment(at, gate.inside, Some(level));
                if part.is_empty() {
                    return Vec::new();
                }
                path.extend(part);
            }
            path.extend([gate.boundary, gate.outside]);
            at = gate.outside;
        }
        if !levels.is_empty() {
            let part = self.segment(at, junction, common);
            if part.is_empty() {
                return Vec::new();
            }
            path.extend(part);
        }
        simplify(path)
    }
}
fn in_bounds(p: Point, b: Rect) -> bool {
    p.x >= b.position.x
        && p.x <= b.position.x + b.width
        && p.y >= b.position.y
        && p.y <= b.position.y + b.height
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
fn route(
    start: Point,
    end: Point,
    rects: &[Rect],
    bounds: Option<Rect>,
    tracks: &[Track],
) -> Vec<Point> {
    if bounds.is_some_and(|b| !in_bounds(start, b) || !in_bounds(end, b)) {
        return Vec::new();
    }
    // A clear L is already Manhattan-shortest with the minimum bend count.
    // Avoid constructing a visibility grid for the common straight/L cases.
    for elbow in [
        Point {
            x: end.x,
            y: start.y,
        },
        Point {
            x: start.x,
            y: end.y,
        },
    ] {
        let path = simplify(vec![start, elbow, end]);
        if path
            .windows(2)
            .all(|p| !blocked(p[0], p[1], rects) && !track_conflict(p[0], p[1], tracks))
        {
            return path;
        }
    }
    let relevant: Vec<_> = tracks
        .iter()
        .filter(|track| {
            !bounds.is_some_and(|b| {
                track.b.x + TRACK_SPACING < b.position.x
                    || track.a.x - TRACK_SPACING > b.position.x + b.width
                    || track.a.y.min(track.b.y) - TRACK_SPACING > b.position.y + b.height
                    || track.a.y.max(track.b.y) + TRACK_SPACING < b.position.y
            })
        })
        .copied()
        .collect();
    let tracks = &relevant[..];
    let mut xs = vec![start.x, end.x];
    let mut ys = vec![start.y, end.y];
    for r in rects {
        xs.extend([r.position.x - 12., r.position.x + r.width + 12.]);
        ys.extend([r.position.y - 12., r.position.y + r.height + 12.]);
    }
    for track in tracks {
        if bounds.is_some_and(|b| {
            track.b.x + TRACK_SPACING < b.position.x
                || track.a.x - TRACK_SPACING > b.position.x + b.width
                || track.a.y.min(track.b.y) - TRACK_SPACING > b.position.y + b.height
                || track.a.y.max(track.b.y) + TRACK_SPACING < b.position.y
        }) {
            continue;
        }
        if track.horizontal() {
            ys.extend([track.a.y - TRACK_SPACING, track.a.y + TRACK_SPACING]);
            xs.extend([
                track.a.x - TRACK_SPACING,
                track.a.x,
                track.b.x,
                track.b.x + TRACK_SPACING,
            ]);
        } else {
            xs.extend([track.a.x - TRACK_SPACING, track.a.x + TRACK_SPACING]);
            ys.extend([
                track.a.y - TRACK_SPACING,
                track.a.y,
                track.b.y,
                track.b.y + TRACK_SPACING,
            ]);
        }
    }
    if let Some(b) = bounds {
        xs.extend([b.position.x, b.position.x + b.width]);
        ys.extend([b.position.y, b.position.y + b.height]);
        xs.retain(|x| *x >= b.position.x && *x <= b.position.x + b.width);
        ys.retain(|y| *y >= b.position.y && *y <= b.position.y + b.height);
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
            if blocked(a, b, rects) || track_conflict(a, b, tracks) {
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
    fn assert_track_spacing(result: &Layout) {
        for (i, a) in result.bundles.iter().enumerate() {
            assert!(
                a.branches.iter().all(|b| !b.points.is_empty()),
                "unrouted {:?}",
                a.connections
            );
            let tracks: Vec<_> = a
                .branches
                .iter()
                .flat_map(|branch| {
                    branch
                        .points
                        .windows(2)
                        .map(|pair| Track::new(pair[0], pair[1]))
                })
                .collect();
            for b in &result.bundles[i + 1..] {
                for branch in &b.branches {
                    for pair in branch.points.windows(2) {
                        assert!(
                            !track_conflict(pair[0], pair[1], &tracks),
                            "parallel overlap between {:?} and {:?}: {:?}",
                            a.connections,
                            b.connections,
                            pair
                        );
                    }
                }
            }
            for b in &result.bundles[i + 1..] {
                for aa in &a.branches {
                    for bb in &b.branches {
                        if aa.entity == bb.entity {
                            assert!(
                                distance(aa.points[0], bb.points[0]) >= TRACK_SPACING - 1e-6,
                                "shared port on {}",
                                aa.entity
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn independent_connections_reserve_parallel_tracks_and_ports() {
        let doc = sample();
        let initial = layout(&LayoutRequest {
            document: doc.clone(),
            view: View::default(),
        })
        .unwrap();
        assert_track_spacing(&initial);
        for (dx, dy, collapsed) in [(0., 24., false), (20., 20., false), (0., 0., true)] {
            let mut view = View {
                layout_version: 2,
                positions: initial.positions.clone(),
                ..View::default()
            };
            let p = view.positions.get_mut("kitchen").unwrap();
            p.x += dx;
            p.y += dy;
            if collapsed {
                view.collapsed.insert("kitchen".into());
            }
            assert_track_spacing(
                &layout(&LayoutRequest {
                    document: doc.clone(),
                    view,
                })
                .unwrap(),
            );
        }
    }
    #[test]
    fn crowded_connections_between_the_same_devices_get_distinct_lanes() {
        let mut doc = sample();
        doc.entities
            .retain(|e| ["house", "panel", "lamp"].contains(&e.id.as_str()));
        for e in &mut doc.entities {
            if e.id != "house" {
                e.parent = Some("house".into());
            }
        }
        doc.connections.clear();
        for layer in &doc.layers {
            doc.connections.push(Connection {
                id: layer.id.clone(),
                name: layer.name.clone(),
                layer: layer.id.clone(),
                endpoints: vec![
                    Endpoint {
                        entity: "panel".into(),
                        role: Role::Source,
                    },
                    Endpoint {
                        entity: "lamp".into(),
                        role: Role::Target,
                    },
                ],
            });
        }
        let result = layout(&LayoutRequest {
            document: doc,
            view: View::default(),
        })
        .unwrap();
        assert_eq!(result.bundles.len(), 4);
        assert_track_spacing(&result);
    }
    #[test]
    fn exhausted_corridors_report_connections_instead_of_overlapping() {
        let mut doc = sample();
        doc.entities
            .retain(|e| ["house", "panel", "lamp"].contains(&e.id.as_str()));
        for e in &mut doc.entities {
            if e.id != "house" {
                e.parent = Some("house".into());
            }
        }
        doc.layers = (0..16)
            .map(|i| Layer {
                id: format!("run-{i}"),
                name: format!("Run {i}"),
                color: "#123456".into(),
            })
            .collect();
        doc.connections = doc
            .layers
            .iter()
            .map(|layer| Connection {
                id: layer.id.clone(),
                name: layer.name.clone(),
                layer: layer.id.clone(),
                endpoints: vec![
                    Endpoint {
                        entity: "panel".into(),
                        role: Role::Source,
                    },
                    Endpoint {
                        entity: "lamp".into(),
                        role: Role::Target,
                    },
                ],
            })
            .collect();
        let result = layout(&LayoutRequest {
            document: doc,
            view: View::default(),
        })
        .unwrap();
        assert!(!result.unrouted_connections.is_empty());
        let mut tracks = Vec::new();
        for bundle in &result.bundles {
            let reported = bundle
                .connections
                .iter()
                .all(|id| result.unrouted_connections.contains(id));
            assert!(
                bundle
                    .branches
                    .iter()
                    .all(|b| b.points.is_empty() == reported)
            );
            for branch in &bundle.branches {
                for pair in branch.points.windows(2) {
                    assert!(!track_conflict(pair[0], pair[1], &tracks));
                }
            }
            for branch in &bundle.branches {
                tracks.extend(branch.points.windows(2).map(|p| Track::new(p[0], p[1])));
            }
        }
    }
    #[test]
    fn tracks_allow_crossings_but_reject_parallel_runs_without_padding() {
        let tracks = [Track::new(
            Point { x: 0., y: 10. },
            Point { x: 100., y: 10. },
        )];
        assert!(track_conflict(
            Point { x: 80., y: 10. },
            Point { x: 20., y: 10. },
            &tracks
        ));
        assert!(track_conflict(
            Point { x: 20., y: 19. },
            Point { x: 80., y: 19. },
            &tracks
        ));
        assert!(!track_conflict(
            Point { x: 20., y: 20. },
            Point { x: 80., y: 20. },
            &tracks
        ));
        assert!(!track_conflict(
            Point { x: 50., y: 0. },
            Point { x: 50., y: 30. },
            &tracks
        ));
    }
    fn visits(path: &[Point], rect: Rect) -> Vec<(f64, f64)> {
        let mut intervals: Vec<(f64, f64)> = Vec::new();
        let mut offset = 0.;
        for pair in path.windows(2) {
            let a = pair[0];
            let b = pair[1];
            let length = distance(a, b);
            let clip = if a.y == b.y && a.y > rect.position.y && a.y < rect.position.y + rect.height
            {
                let lo = a.x.min(b.x).max(rect.position.x);
                let hi = a.x.max(b.x).min(rect.position.x + rect.width);
                (hi > lo).then_some((
                    (lo - a.x).abs().min((hi - a.x).abs()),
                    (lo - a.x).abs().max((hi - a.x).abs()),
                ))
            } else if a.x == b.x && a.x > rect.position.x && a.x < rect.position.x + rect.width {
                let lo = a.y.min(b.y).max(rect.position.y);
                let hi = a.y.max(b.y).min(rect.position.y + rect.height);
                (hi > lo).then_some((
                    (lo - a.y).abs().min((hi - a.y).abs()),
                    (lo - a.y).abs().max((hi - a.y).abs()),
                ))
            } else {
                None
            };
            if let Some((lo, hi)) = clip {
                let range = (offset + lo, offset + hi);
                if let Some(last) = intervals
                    .last_mut()
                    .filter(|last| (last.1 - range.0).abs() < 1e-5)
                {
                    last.1 = range.1;
                } else {
                    intervals.push(range);
                }
            }
            offset += length;
        }
        intervals
    }
    fn exit_point(path: &[Point], n: &Node) -> Point {
        let rect = Rect {
            position: n.position,
            width: n.width,
            height: n.height,
        };
        for pair in path.windows(2) {
            let a = pair[0];
            let b = pair[1];
            if in_bounds(a, rect) && !in_bounds(b, rect) {
                return if a.x == b.x {
                    Point {
                        x: a.x,
                        y: if b.y < a.y {
                            n.position.y
                        } else {
                            n.position.y + n.height
                        },
                    }
                } else {
                    Point {
                        x: if b.x < a.x {
                            n.position.x
                        } else {
                            n.position.x + n.width
                        },
                        y: a.y,
                    }
                };
            }
        }
        panic!("branch never leaves {}", n.id)
    }
    #[test]
    fn routes_only_visit_endpoint_ancestors_and_cross_each_once() {
        let doc = sample();
        let initial = layout(&LayoutRequest {
            document: doc.clone(),
            view: View::default(),
        })
        .unwrap();
        for (dx, dy, collapsed) in [
            (0., 0., false),
            (0., 24., false),
            (20., 20., false),
            (0., 0., true),
        ] {
            let mut view = View {
                layout_version: 2,
                positions: initial.positions.clone(),
                ..View::default()
            };
            let p = view.positions.get_mut("kitchen").unwrap();
            p.x += dx;
            p.y += dy;
            if collapsed {
                view.collapsed.insert("kitchen".into());
            }
            let result = layout(&LayoutRequest {
                document: doc.clone(),
                view,
            })
            .unwrap();
            let router = HierarchyRouter {
                document: &doc,
                nodes: &result.nodes,
                tracks: &[],
            };
            for bundle in &result.bundles {
                for branch in &bundle.branches {
                    assert!(
                        !branch.points.is_empty(),
                        "missing route {:?}: {} at {dx},{dy}",
                        bundle.connections,
                        branch.entity
                    );
                    let ancestors = router.ancestors(&branch.entity);
                    for n in &result.nodes {
                        if n.id == branch.entity {
                            continue;
                        }
                        let rect = Rect {
                            position: n.position,
                            width: n.width,
                            height: n.height,
                        };
                        if ancestors.contains(&n.id) {
                            let closed = Rect {
                                position: Point {
                                    x: rect.position.x - 1e-6,
                                    y: rect.position.y - 1e-6,
                                },
                                width: rect.width + 2e-6,
                                height: rect.height + 2e-6,
                            };
                            assert_eq!(
                                visits(&branch.points, closed).len(),
                                1,
                                "{} leaves then re-enters {}",
                                branch.entity,
                                n.id
                            );
                        } else {
                            assert!(
                                visits(&branch.points, rect).is_empty(),
                                "{} transits unrelated {} in {:?}",
                                branch.entity,
                                n.id,
                                bundle.connections
                            );
                        }
                    }
                    for pair in branch.points.windows(2) {
                        assert!(pair[0].x == pair[1].x || pair[0].y == pair[1].y);
                    }
                }
            }
            if !collapsed {
                let circuit = result
                    .bundles
                    .iter()
                    .find(|b| b.connections.contains(&"circuit".into()))
                    .unwrap();
                let kitchen = router.node("kitchen");
                let lamp = circuit
                    .branches
                    .iter()
                    .find(|b| b.entity == "lamp")
                    .unwrap();
                let sensor = circuit
                    .branches
                    .iter()
                    .find(|b| b.entity == "sensor")
                    .unwrap();
                assert_eq!(
                    exit_point(&lamp.points, kitchen),
                    exit_point(&sensor.points, kitchen)
                );
            }
        }
    }
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
        let path = route(
            Point { x: 0., y: 36. },
            Point { x: 400., y: 36. },
            &rects,
            None,
            &[],
        );
        assert!(path.len() >= 4);
        for p in path.windows(2) {
            assert!(p[0].x == p[1].x || p[0].y == p[1].y);
            assert!(!blocked(p[0], p[1], &rects));
        }
    }
}
