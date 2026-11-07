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
    /// Branch points derived from the union of the final routed segments.
    pub junctions: Vec<Point>,
    pub label_position: Option<Point>,
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
        let shared_gates: BTreeMap<_, _> = gates
            .iter()
            .filter(|(space, _)| {
                eps.iter()
                    .filter(|(id, _)| router.ancestors(id).contains(space))
                    .count()
                    > 1
            })
            .map(|(id, gate)| (id.clone(), gate.boundary))
            .collect();
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
        // Boundary gateways are construction waypoints, not mandatory bends.
        // Straighten the complete route across levels while retaining shared
        // entries and actual joins, hierarchy constraints, and reserved tracks.
        if branches.iter().all(|b| !b.points.is_empty()) {
            let joins = route_junctions(&branches);
            for branch in &mut branches {
                branch.points = router.straighten(branch, common, &shared_gates, &joins);
            }
            router.align_runs(&mut branches, common, &shared_gates);
        }
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
        let junctions = route_junctions(&branches);
        let label_position = junctions.first().copied().or_else(|| {
            branches
                .iter()
                .flat_map(|b| b.points.windows(2))
                .max_by(|a, b| distance(a[0], a[1]).total_cmp(&distance(b[0], b[1])))
                .map(|p| Point {
                    x: (p[0].x + p[1].x) / 2.,
                    y: (p[0].y + p[1].y) / 2.,
                })
        });
        bundles.push(Bundle {
            layer,
            connections,
            branches,
            junctions,
            label_position,
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
/// Count geometric directions, not branch records: overlapping branch tails
/// contribute one arm each. A bend has two arms; a split/merge has three or four.
/// This is evaluated separately per bundle, so crossings of independent
/// connections never acquire a junction dot.
fn route_junctions(branches: &[Branch]) -> Vec<Point> {
    const EPS: f64 = 1e-7;
    let mut segments = Vec::new();
    for pair in branches.iter().flat_map(|b| b.points.windows(2)) {
        let track = Track::new(pair[0], pair[1]);
        if track.a != track.b && !segments.contains(&track) {
            segments.push(track);
        }
    }
    let mut candidates: Vec<_> = segments.iter().flat_map(|t| [t.a, t.b]).collect();
    for (i, a) in segments.iter().enumerate() {
        for b in &segments[i + 1..] {
            if a.horizontal() == b.horizontal() {
                continue;
            }
            let (h, v) = if a.horizontal() { (a, b) } else { (b, a) };
            let p = Point { x: v.a.x, y: h.a.y };
            if p.x >= h.a.x - EPS && p.x <= h.b.x + EPS && p.y >= v.a.y - EPS && p.y <= v.b.y + EPS
            {
                candidates.push(p);
            }
        }
    }
    candidates.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    candidates.dedup_by(|a, b| (a.x - b.x).abs() < EPS && (a.y - b.y).abs() < EPS);
    candidates
        .into_iter()
        .filter(|p| {
            let mut arms = 0_u8;
            for t in &segments {
                if t.horizontal()
                    && (p.y - t.a.y).abs() < EPS
                    && p.x >= t.a.x - EPS
                    && p.x <= t.b.x + EPS
                {
                    if p.x > t.a.x + EPS {
                        arms |= 1;
                    }
                    if p.x < t.b.x - EPS {
                        arms |= 2;
                    }
                } else if !t.horizontal()
                    && (p.x - t.a.x).abs() < EPS
                    && p.y >= t.a.y - EPS
                    && p.y <= t.b.y + EPS
                {
                    if p.y > t.a.y + EPS {
                        arms |= 4;
                    }
                    if p.y < t.b.y - EPS {
                        arms |= 8;
                    }
                }
            }
            arms.count_ones() >= 3
        })
        .collect()
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
const GEOMETRY_EPS: f64 = 1e-7;
fn path_contains(path: &[Point], point: Point) -> bool {
    path.windows(2).any(|p| {
        if p[0].x == p[1].x {
            (point.x - p[0].x).abs() < GEOMETRY_EPS
                && point.y >= p[0].y.min(p[1].y) - GEOMETRY_EPS
                && point.y <= p[0].y.max(p[1].y) + GEOMETRY_EPS
        } else {
            (point.y - p[0].y).abs() < GEOMETRY_EPS
                && point.x >= p[0].x.min(p[1].x) - GEOMETRY_EPS
                && point.x <= p[0].x.max(p[1].x) + GEOMETRY_EPS
        }
    })
}
fn crosses_interior(a: Point, b: Point, rect: Rect) -> bool {
    if a.x == b.x {
        a.x > rect.position.x + GEOMETRY_EPS
            && a.x < rect.position.x + rect.width - GEOMETRY_EPS
            && a.y.min(b.y) < rect.position.y + rect.height - GEOMETRY_EPS
            && a.y.max(b.y) > rect.position.y + GEOMETRY_EPS
    } else {
        a.y > rect.position.y + GEOMETRY_EPS
            && a.y < rect.position.y + rect.height - GEOMETRY_EPS
            && a.x.min(b.x) < rect.position.x + rect.width - GEOMETRY_EPS
            && a.x.max(b.x) > rect.position.x + GEOMETRY_EPS
    }
}
/// Reject leaving and then entering an ancestor again. Merely following its
/// boundary is allowed; crossing its interior after an exit is not.
fn single_exit(path: &[Point], rect: Rect) -> bool {
    let Some(first) = path.first() else {
        return false;
    };
    if !in_bounds(*first, rect) {
        return false;
    }
    let mut outside = false;
    for pair in path.windows(2) {
        if outside && crosses_interior(pair[0], pair[1], rect) {
            return false;
        }
        if !in_bounds(pair[1], rect) {
            outside = true;
        }
    }
    true
}
fn boundary_exit(path: &[Point], rect: Rect) -> Option<Point> {
    for pair in path.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if in_bounds(a, rect) && !in_bounds(b, rect) {
            return Some(if a.x == b.x {
                Point {
                    x: a.x,
                    y: if b.y < a.y {
                        rect.position.y
                    } else {
                        rect.position.y + rect.height
                    },
                }
            } else {
                Point {
                    x: if b.x < a.x {
                        rect.position.x
                    } else {
                        rect.position.x + rect.width
                    },
                    y: a.y,
                }
            });
        }
    }
    None
}
/// Geometry checks shared by individual shortcutting and coordinated track
/// alignment. Topology checks live at the connection level below.
struct RouteGuard<'a> {
    tracks: &'a [Track],
    own: Rect,
    solids: Vec<Rect>,
    headers: Vec<Rect>,
    boxes: Vec<(String, Rect)>,
    bounds: Option<Rect>,
    port: Point,
    outward: Point,
}
impl<'a> RouteGuard<'a> {
    fn new(router: &HierarchyRouter<'a>, branch: &Branch, common: Option<&str>) -> Self {
        let ancestors = router.ancestors(&branch.entity);
        let own = router.node(&branch.entity);
        Self {
            tracks: router.tracks,
            own: Rect {
                position: own.position,
                width: own.width,
                height: own.height,
            },
            solids: router
                .nodes
                .iter()
                .filter(|n| n.id != branch.entity && !ancestors.contains(&n.id))
                .map(|n| Rect {
                    position: n.position,
                    width: n.width,
                    height: n.height,
                })
                .collect(),
            headers: ancestors
                .iter()
                .map(|id| {
                    let n = router.node(id);
                    Rect {
                        position: n.position,
                        width: n.width,
                        height: 52.,
                    }
                })
                .collect(),
            boxes: ancestors
                .iter()
                .take_while(|id| Some(id.as_str()) != common)
                .map(|id| {
                    let n = router.node(id);
                    (
                        id.clone(),
                        Rect {
                            position: n.position,
                            width: n.width,
                            height: n.height,
                        },
                    )
                })
                .collect(),
            bounds: router.bounds(common),
            port: branch.points[0],
            outward: Point {
                x: branch.points[1].x - branch.points[0].x,
                y: branch.points[1].y - branch.points[0].y,
            },
        }
    }
    fn allows(&self, path: &[Point]) -> bool {
        if path.len() < 2 || distance(path[0], self.port) > GEOMETRY_EPS {
            return false;
        }
        let edge = Point {
            x: path[1].x - path[0].x,
            y: path[1].y - path[0].y,
        };
        if self.outward.x * edge.x + self.outward.y * edge.y <= 0.
            || self.outward.x * edge.y != self.outward.y * edge.x
        {
            return false;
        }
        if path
            .iter()
            .any(|p| self.bounds.is_some_and(|b| !in_bounds(*p, b)))
        {
            return false;
        }
        for (i, p) in path.windows(2).enumerate() {
            if (p[0].x != p[1].x && p[0].y != p[1].y)
                || blocked(p[0], p[1], &self.solids)
                || blocked(p[0], p[1], &self.headers)
                || track_conflict(p[0], p[1], self.tracks)
            {
                return false;
            }
            if i > 0 && blocked(p[0], p[1], &[self.own]) {
                return false;
            }
        }
        self.boxes
            .iter()
            .all(|(_, rect)| single_exit(path, *rect) && !in_bounds(*path.last().unwrap(), *rect))
    }
}
#[derive(Clone, Copy)]
struct ConnectedRun {
    horizontal: bool,
    axis: f64,
    lo: f64,
    hi: f64,
}
impl ConnectedRun {
    fn contains(self, p: Point) -> bool {
        let (axis, along) = if self.horizontal {
            (p.y, p.x)
        } else {
            (p.x, p.y)
        };
        (axis - self.axis).abs() < GEOMETRY_EPS
            && along >= self.lo - GEOMETRY_EPS
            && along <= self.hi + GEOMETRY_EPS
    }
    fn shifted(self, p: Point, axis: f64) -> Point {
        if !self.contains(p) {
            p
        } else if self.horizontal {
            Point { x: p.x, y: axis }
        } else {
            Point { x: axis, y: p.y }
        }
    }
}
fn connected_runs(branches: &[Branch]) -> Vec<ConnectedRun> {
    let mut runs: Vec<_> = branches
        .iter()
        .flat_map(|b| b.points.windows(2))
        .filter(|p| p[0] != p[1])
        .map(|p| {
            let t = Track::new(p[0], p[1]);
            if t.horizontal() {
                ConnectedRun {
                    horizontal: true,
                    axis: t.a.y,
                    lo: t.a.x,
                    hi: t.b.x,
                }
            } else {
                ConnectedRun {
                    horizontal: false,
                    axis: t.a.x,
                    lo: t.a.y,
                    hi: t.b.y,
                }
            }
        })
        .collect();
    runs.sort_by(|a, b| {
        a.horizontal
            .cmp(&b.horizontal)
            .then(a.axis.total_cmp(&b.axis))
            .then(a.lo.total_cmp(&b.lo))
    });
    let mut merged: Vec<ConnectedRun> = Vec::new();
    for run in runs {
        if let Some(previous) = merged.last_mut().filter(|p| {
            p.horizontal == run.horizontal
                && (p.axis - run.axis).abs() < GEOMETRY_EPS
                && run.lo <= p.hi + GEOMETRY_EPS
        }) {
            previous.hi = previous.hi.max(run.hi);
        } else {
            merged.push(run);
        }
    }
    merged
}
/// A coincident tail ending at the solver's anchor can become redundant when
/// two tracks align. Trim only a suffix shared by every endpoint branch.
fn trim_common_tail(paths: &mut [Vec<Point>]) {
    loop {
        if paths.iter().any(|p| p.len() < 2) {
            return;
        }
        let anchor = *paths[0].last().unwrap();
        if paths
            .iter()
            .any(|p| distance(*p.last().unwrap(), anchor) > GEOMETRY_EPS)
        {
            return;
        }
        let prev = paths[0][paths[0].len() - 2];
        let sign = |d: f64| {
            if d.abs() < GEOMETRY_EPS {
                0.
            } else {
                d.signum()
            }
        };
        let direction = Point {
            x: sign(prev.x - anchor.x),
            y: sign(prev.y - anchor.y),
        };
        if paths.iter().any(|p| {
            let prev = p[p.len() - 2];
            sign(prev.x - anchor.x) != direction.x || sign(prev.y - anchor.y) != direction.y
        }) {
            return;
        }
        let step = paths
            .iter()
            .map(|p| distance(p[p.len() - 2], anchor))
            .fold(f64::INFINITY, f64::min);
        if step <= GEOMETRY_EPS
            || paths
                .iter()
                .any(|p| p.len() == 2 && distance(p[0], anchor) <= step + GEOMETRY_EPS)
        {
            return;
        }
        let next = Point {
            x: anchor.x + direction.x * step,
            y: anchor.y + direction.y * step,
        };
        for path in &mut *paths {
            *path.last_mut().unwrap() = next;
            *path = simplify(std::mem::take(path));
        }
    }
}
impl HierarchyRouter<'_> {
    fn align_runs(
        &self,
        branches: &mut [Branch],
        common: Option<&str>,
        shared: &BTreeMap<String, Point>,
    ) {
        loop {
            // Simplification can expose a common solver tail even when there
            // are no further tracks to align. Remove that tail before searching.
            let old_joins = route_junctions(branches);
            let tail_guards: Vec<_> = branches
                .iter()
                .map(|b| RouteGuard::new(self, b, common))
                .collect();
            let mut trimmed: Vec<_> = branches.iter().map(|b| b.points.clone()).collect();
            trim_common_tail(&mut trimmed);
            if tail_guards.iter().zip(&trimmed).all(|(g, p)| g.allows(p))
                && old_joins.iter().all(|join| {
                    branches
                        .iter()
                        .zip(&trimmed)
                        .all(|(b, p)| !path_contains(&b.points, *join) || path_contains(p, *join))
                })
            {
                for (branch, path) in branches.iter_mut().zip(trimmed) {
                    branch.points = path;
                }
            }
            let runs = connected_runs(branches);
            let joins = route_junctions(branches);
            let guards: Vec<_> = branches
                .iter()
                .map(|b| RouteGuard::new(self, b, common))
                .collect();
            let baseline: usize = branches.iter().map(|b| b.points.len()).sum();
            let mut accepted = None;
            'search: for run in &runs {
                if branches.iter().any(|b| run.contains(b.points[0])) {
                    continue;
                }
                // Only align runs connected by a perpendicular leg. Sliding
                // arbitrary unrelated tracks can create a new accidental join.
                let mut axes: Vec<_> = runs
                    .iter()
                    .filter(|other| {
                        other.horizontal == run.horizontal
                            && (other.axis - run.axis).abs() > GEOMETRY_EPS
                    })
                    .filter(|other| {
                        branches.iter().flat_map(|b| b.points.windows(2)).any(|p| {
                            if run.horizontal {
                                p[0].x == p[1].x
                                    && p[0].x >= run.lo - GEOMETRY_EPS
                                    && p[0].x <= run.hi + GEOMETRY_EPS
                                    && p[0].x >= other.lo - GEOMETRY_EPS
                                    && p[0].x <= other.hi + GEOMETRY_EPS
                                    && p[0].y.min(p[1].y) <= run.axis.min(other.axis) + GEOMETRY_EPS
                                    && p[0].y.max(p[1].y) >= run.axis.max(other.axis) - GEOMETRY_EPS
                            } else {
                                p[0].y == p[1].y
                                    && p[0].y >= run.lo - GEOMETRY_EPS
                                    && p[0].y <= run.hi + GEOMETRY_EPS
                                    && p[0].y >= other.lo - GEOMETRY_EPS
                                    && p[0].y <= other.hi + GEOMETRY_EPS
                                    && p[0].x.min(p[1].x) <= run.axis.min(other.axis) + GEOMETRY_EPS
                                    && p[0].x.max(p[1].x) >= run.axis.max(other.axis) - GEOMETRY_EPS
                            }
                        })
                    })
                    .map(|other| other.axis)
                    .collect();
                axes.sort_by(|a, b| (a - run.axis).abs().total_cmp(&(b - run.axis).abs()));
                axes.dedup_by(|a, b| (*a - *b).abs() < GEOMETRY_EPS);
                'axis: for axis in axes {
                    let mut paths: Vec<_> = branches
                        .iter()
                        .map(|b| simplify(b.points.iter().map(|p| run.shifted(*p, axis)).collect()))
                        .collect();
                    trim_common_tail(&mut paths);
                    if paths.iter().map(Vec::len).sum::<usize>() >= baseline
                        || !guards.iter().zip(&paths).all(|(g, p)| g.allows(p))
                    {
                        continue;
                    }
                    // Every previously shared geometric join stays incident to
                    // the same branches; only its position may move as a unit.
                    if joins.iter().any(|join| {
                        branches.iter().zip(&paths).any(|(b, p)| {
                            path_contains(&b.points, *join)
                                && !path_contains(p, run.shifted(*join, axis))
                        })
                    }) {
                        continue;
                    }
                    for id in shared.keys() {
                        let mut expected = None;
                        for (guard, path) in guards.iter().zip(&paths) {
                            if let Some((_, rect)) =
                                guard.boxes.iter().find(|(space, _)| space == id)
                            {
                                let Some(exit) = boundary_exit(path, *rect) else {
                                    continue 'axis;
                                };
                                if expected.is_some_and(|p| distance(p, exit) > GEOMETRY_EPS) {
                                    continue 'axis;
                                }
                                expected = Some(exit);
                            }
                        }
                    }
                    accepted = Some(paths);
                    break 'search;
                }
            }
            let Some(paths) = accepted else {
                break;
            };
            for (branch, path) in branches.iter_mut().zip(paths) {
                branch.points = path;
            }
            let current_shared: BTreeMap<_, _> = shared
                .keys()
                .filter_map(|id| {
                    guards
                        .iter()
                        .zip(&*branches)
                        .find_map(|(guard, b)| {
                            guard
                                .boxes
                                .iter()
                                .find(|(space, _)| space == id)
                                .and_then(|(_, rect)| boundary_exit(&b.points, *rect))
                        })
                        .map(|point| (id.clone(), point))
                })
                .collect();
            let current_joins = route_junctions(branches);
            for b in &mut *branches {
                b.points = self.straighten(b, common, &current_shared, &current_joins);
            }
        }
    }
    fn straighten(
        &self,
        branch: &Branch,
        common: Option<&str>,
        shared: &BTreeMap<String, Point>,
        joins: &[Point],
    ) -> Vec<Point> {
        let original = &branch.points;
        if original.len() < 4 {
            return original.clone();
        }
        let guard = RouteGuard::new(self, branch, common);
        let locks: Vec<_> = joins
            .iter()
            .copied()
            .filter(|p| path_contains(original, *p))
            .collect();
        let valid = |path: &[Point]| {
            guard.allows(path)
                && guard.boxes.iter().all(|(id, rect)| {
                    shared.get(id).is_none_or(|gate| {
                        boundary_exit(path, *rect)
                            .is_some_and(|p| distance(p, *gate) <= GEOMETRY_EPS)
                    })
                })
                && locks.iter().all(|p| path_contains(path, *p))
        };
        let mut path = original.clone();
        loop {
            let mut improved = None;
            // Longest intervals first: a single shortcut can align entries
            // through several nested spaces instead of adding one elbow each.
            'search: for span in (2..path.len()).rev() {
                for start in 1..path.len() - span {
                    let end = start + span;
                    let a = path[start];
                    let b = path[end];
                    for elbow in [Point { x: b.x, y: a.y }, Point { x: a.x, y: b.y }] {
                        let mut candidate = path[..start].to_vec();
                        candidate.extend([a, elbow, b]);
                        candidate.extend_from_slice(&path[end + 1..]);
                        let candidate = simplify(candidate);
                        if candidate.len() < path.len() && valid(&candidate) {
                            improved = Some(candidate);
                            break 'search;
                        }
                    }
                }
            }
            if let Some(candidate) = improved {
                path = candidate;
            } else {
                break;
            }
        }
        path
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
    #[test]
    fn trimming_a_shared_anchor_tail_stays_axis_aligned() {
        let mut paths = vec![
            vec![
                Point { x: 0., y: 0. },
                Point { x: 40., y: 0. },
                Point { x: 40., y: 80. },
            ],
            vec![
                Point { x: 80., y: 40. },
                Point { x: 40., y: 40. },
                Point { x: 40., y: 80. },
            ],
            vec![
                Point { x: 0., y: 40. },
                Point { x: 40., y: 40. },
                Point { x: 40., y: 80. },
            ],
        ];
        trim_common_tail(&mut paths);
        for path in &paths {
            assert_eq!(path.last(), Some(&Point { x: 40., y: 40. }));
            assert!(
                path.windows(2)
                    .all(|p| p[0].x == p[1].x || p[0].y == p[1].y)
            );
        }
    }
    #[test]
    fn shared_run_alignment_reduces_bends_without_detaching_branches() {
        let mut doc = sample();
        doc.entities
            .retain(|e| ["house", "panel", "lamp", "sensor"].contains(&e.id.as_str()));
        doc.connections.clear();
        for e in &mut doc.entities {
            if e.id != "house" {
                e.parent = Some("house".into());
            }
        }
        let positions = BTreeMap::from([
            ("house".into(), Point { x: 0., y: 0. }),
            ("panel".into(), Point { x: 80., y: 80. }),
            ("lamp".into(), Point { x: 80., y: 200. }),
            ("sensor".into(), Point { x: 400., y: 200. }),
        ]);
        let result = layout(&LayoutRequest {
            document: doc.clone(),
            view: View {
                layout_version: 2,
                positions,
                ..View::default()
            },
        })
        .unwrap();
        let router = HierarchyRouter {
            document: &doc,
            nodes: &result.nodes,
            tracks: &[],
        };
        let make = |id: &str, points: &[(f64, f64)]| Branch {
            entity: id.into(),
            role: Role::Peer,
            points: points.iter().map(|(x, y)| Point { x: *x, y: *y }).collect(),
        };
        let mut branches = vec![
            make(
                "panel",
                &[
                    (280., 116.),
                    (312., 116.),
                    (312., 176.),
                    (340., 176.),
                    (340., 260.),
                ],
            ),
            make(
                "lamp",
                &[(280., 236.), (312., 236.), (312., 260.), (340., 260.)],
            ),
            make("sensor", &[(400., 236.), (340., 236.), (340., 260.)]),
        ];
        let guards: Vec<_> = branches
            .iter()
            .map(|b| RouteGuard::new(&router, b, Some("house")))
            .collect();
        let ports: Vec<_> = branches.iter().map(|b| b.points[0]).collect();
        let before: usize = branches.iter().map(|b| b.points.len()).sum();
        assert!(
            guards
                .iter()
                .zip(&branches)
                .all(|(g, b)| g.allows(&b.points)),
            "invalid fixture geometry"
        );
        router.align_runs(&mut branches, Some("house"), &BTreeMap::new());
        assert!(branches.iter().map(|b| b.points.len()).sum::<usize>() <= before - 4);
        for ((branch, guard), port) in branches.iter().zip(&guards).zip(ports) {
            assert_eq!(branch.points[0], port);
            assert!(guard.allows(&branch.points));
            assert_eq!(branch.points.last(), branches[0].points.last());
        }
        assert_eq!(route_junctions(&branches).len(), 1);
    }
    fn nested_staircase() -> (Document, Layout, Branch) {
        let mut doc = sample();
        doc.entities
            .retain(|e| ["house", "upstairs", "office", "desk"].contains(&e.id.as_str()));
        doc.connections.clear();
        let result = layout(&LayoutRequest {
            document: doc.clone(),
            view: View::default(),
        })
        .unwrap();
        let n = |id: &str| result.nodes.iter().find(|n| n.id == id).unwrap();
        let desk = n("desk");
        let office = n("office");
        let upstairs = n("upstairs");
        let house = n("house");
        let points = vec![
            Point {
                x: desk.position.x,
                y: desk.position.y + 36.,
            },
            Point {
                x: desk.position.x - 12.,
                y: desk.position.y + 36.,
            },
            Point {
                x: office.position.x + 12.,
                y: office.position.y + 64.,
            },
            Point {
                x: office.position.x - 12.,
                y: office.position.y + 64.,
            },
            Point {
                x: upstairs.position.x + 12.,
                y: upstairs.position.y + 64.,
            },
            Point {
                x: upstairs.position.x - 12.,
                y: upstairs.position.y + 64.,
            },
            Point {
                x: house.position.x + 12.,
                y: house.position.y + 64.,
            },
        ];
        let branch = Branch {
            entity: "desk".into(),
            role: Role::Target,
            points,
        };
        (doc, result, branch)
    }
    #[test]
    fn nested_boundary_staircase_straightens_into_one_bend() {
        let (doc, result, branch) = nested_staircase();
        let router = HierarchyRouter {
            document: &doc,
            nodes: &result.nodes,
            tracks: &[],
        };
        let path = router.straighten(&branch, Some("house"), &BTreeMap::new(), &[]);
        assert_eq!(
            path,
            vec![
                branch.points[0],
                Point {
                    x: branch.points.last().unwrap().x,
                    y: branch.points[0].y
                },
                *branch.points.last().unwrap()
            ]
        );
        for id in ["office", "upstairs"] {
            let n = router.node(id);
            let rect = Rect {
                position: n.position,
                width: n.width,
                height: n.height,
            };
            assert!(single_exit(&path, rect));
        }
    }
    #[test]
    fn straightening_retains_needed_offsets_for_track_padding() {
        let (doc, result, branch) = nested_staircase();
        let tracks = [Track::new(
            Point {
                x: branch.points.last().unwrap().x,
                y: branch.points[0].y,
            },
            branch.points[1],
        )];
        let router = HierarchyRouter {
            document: &doc,
            nodes: &result.nodes,
            tracks: &tracks,
        };
        let path = router.straighten(&branch, Some("house"), &BTreeMap::new(), &[]);
        assert!(path.len() > 3 && path.len() < branch.points.len());
        assert!(
            path.windows(2)
                .all(|p| !track_conflict(p[0], p[1], &tracks))
        );
    }
    #[test]
    fn straightening_preserves_shared_gateways_and_branch_joins() {
        let (doc, result, branch) = nested_staircase();
        let router = HierarchyRouter {
            document: &doc,
            nodes: &result.nodes,
            tracks: &[],
        };
        let office = router.node("office");
        let gate = Point {
            x: office.position.x,
            y: office.position.y + 64.,
        };
        let shared = BTreeMap::from([("office".into(), gate)]);
        let join = branch.points[3];
        let path = router.straighten(&branch, Some("house"), &shared, &[join]);
        assert!(path.len() < branch.points.len());
        assert!(path_contains(&path, join));
        assert_eq!(
            boundary_exit(
                &path,
                Rect {
                    position: office.position,
                    width: office.width,
                    height: office.height
                }
            ),
            Some(gate)
        );
    }
    fn wire(points: &[(f64, f64)]) -> Branch {
        Branch {
            entity: "test".into(),
            role: Role::Peer,
            points: points.iter().map(|(x, y)| Point { x: *x, y: *y }).collect(),
        }
    }
    #[test]
    fn junctions_follow_the_actual_merge_instead_of_the_solver_anchor() {
        // All three records terminate at the solver anchor (40,40). Their
        // overlapping tails make that point a straight run; the real T is above.
        let branches = vec![
            wire(&[(0., 10.), (40., 10.), (40., 40.)]),
            wire(&[(80., 10.), (40., 10.), (40., 40.)]),
            wire(&[(40., 80.), (40., 40.)]),
        ];
        assert_eq!(route_junctions(&branches), vec![Point { x: 40., y: 10. }]);
    }
    #[test]
    fn junctions_ignore_bends_endpoints_and_duplicate_collinear_tails() {
        let branches = vec![
            wire(&[(0., 0.), (20., 0.), (20., 20.)]),
            wire(&[(40., 20.), (20., 20.)]),
            wire(&[(40., 20.), (30., 20.), (20., 20.)]),
        ];
        assert!(route_junctions(&branches).is_empty());
    }
    #[test]
    fn junctions_detect_midsegment_splits_without_joining_independent_crossings() {
        let horizontal = vec![wire(&[(-20., 0.), (20., 0.)])];
        let vertical = vec![wire(&[(0., -20.), (0., 20.)])];
        // Separate bundles are evaluated independently, even when they cross.
        assert!(route_junctions(&horizontal).is_empty());
        assert!(route_junctions(&vertical).is_empty());
        let same_connection = vec![
            wire(&[(-20., 0.), (20., 0.)]),
            wire(&[(0., -20.), (0., 20.)]),
        ];
        assert_eq!(
            route_junctions(&same_connection),
            vec![Point { x: 0., y: 0. }]
        );
        let t = vec![
            wire(&[(-20., 0.), (20., 0.)]),
            wire(&[(0., -20.), (0., 0.)]),
        ];
        assert_eq!(route_junctions(&t), vec![Point { x: 0., y: 0. }]);
    }
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
            if reported {
                assert!(bundle.junctions.is_empty());
                assert!(bundle.label_position.is_none());
            }
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
