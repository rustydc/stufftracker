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
    /// How the physical layout view draws this thing's `ports`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faceplate: Option<Faceplate>,
    /// Physical attachment points (switch ports, panel breakers, controller
    /// terminals). A connection endpoint may name one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<Port>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Faceplate {
    /// Breaker panel: spaces numbered odd down the left column and even
    /// down the right, as panels are labelled.
    Panel,
    /// Network switch: one row per port, in port order.
    Switch,
    /// Terminal strip (zone controllers, relay boards): one row per terminal.
    Terminals,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Port {
    /// Unique within its entity; what an endpoint's `port` names, e.g.
    /// "14", "1/3", "5A", "eth3", "Z2".
    pub id: String,
    /// What is written beside it: "20 A kitchen", "PoE+ 2.5G", "Zone 2 gym".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// The space or port number it starts at. A panel breaker of N poles
    /// covers spaces position, position+2, … on the same side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    /// Spaces covered (panel breakers: 1, 2 or 3). Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poles: Option<u32>,
    /// "A" or "B": one half of a tandem (twin) breaker sharing its space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half: Option<String>,
    /// The half of each space it occupies, one per pole ("A", "B", or "" for
    /// the whole space), for linked breakers that change half from space to
    /// space: a quad's outer pair is ["A", "B"] over 1 and 3 (1A/3B), its
    /// inner pair ["B", "A"] (1B/3A). Use instead of `half`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub halves: Vec<String>,
    /// The thing's supply: a panel's main breaker or main lugs, a switch's
    /// uplink, a controller's line input. Drawn in its own row above the
    /// numbered positions, where an incoming feed lands. Takes no position.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub main: bool,
}
impl Port {
    fn poles(&self) -> u32 {
        self.poles.unwrap_or(1).max(1)
    }
    /// The half of the k-th space it covers, if only half.
    fn half_at(&self, k: u32) -> Option<&str> {
        match self.halves.get(k as usize) {
            Some(h) => Some(h.as_str()).filter(|h| !h.is_empty()),
            None => self.half.as_deref(),
        }
    }
    /// The spaces it covers, as printed: "14", "1/3", "5A", "7B/9A".
    fn spaces_label(&self, step: u32) -> String {
        let Some(start) = self.position else {
            return String::new();
        };
        let spaces = (0..self.poles()).map(|k| start + k * step);
        if self.halves.is_empty() {
            spaces.map(|s| s.to_string()).collect::<Vec<_>>().join("/")
                + self.half.as_deref().unwrap_or("")
        } else {
            spaces
                .zip(&self.halves)
                .map(|(s, h)| format!("{s}{h}"))
                .collect::<Vec<_>>()
                .join("/")
        }
    }
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
    /// One of the entity's `ports`, where known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
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
    #[serde(default)]
    pub attachment_hints: BTreeMap<String, Point>,
    /// Entities drawn with their physical layout (ports) instead of a card.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub faceplates: BTreeSet<String>,
    /// Show only part of the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<Focus>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Focus {
    /// The things to show; a space brings everything inside it.
    pub entities: Vec<String>,
    /// How many connections away to follow; none = everything reachable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hops: Option<u32>,
    #[serde(default)]
    pub direction: Direction,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Every connection, either way.
    #[default]
    Both,
    /// Toward sources: what feeds this.
    Upstream,
    /// Toward targets: what this feeds.
    Downstream,
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
    /// Present when the node is drawn as its physical layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faceplate: Option<FaceplateLayout>,
}
/// A faceplate's port geometry, relative to its node's origin.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaceplateLayout {
    pub kind: Faceplate,
    pub ports: Vec<PortBox>,
    /// Unoccupied spaces or positions, drawn as empty slots.
    pub empty: Vec<PortBox>,
    /// Row collecting this thing's connections with no port assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unassigned: Option<PortBox>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortBox {
    /// The port's ID; empty for an unused slot.
    pub id: String,
    pub label: String,
    /// The space numbers it covers, as printed: "14", "1/3", "5A".
    pub position: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Which card edge its wires leave from.
    pub side: Side,
    /// 0 for a port's first (or only) box. A linked breaker whose spaces
    /// are not adjacent (a quad's outer pair, 1A + 3B) has a box per space;
    /// the later ones are drawn tied to the first. Wires attach to box 0.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub segment: u32,
}
fn is_zero(n: &u32) -> bool {
    *n == 0
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Right,
    Either,
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
    pub attachment_hints: BTreeMap<String, Point>,
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
        validate_ports(e)?;
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
            if let Some(port) = &ep.port {
                let e = doc.entities.iter().find(|e| e.id == ep.entity).unwrap();
                if !e.ports.iter().any(|p| &p.id == port) {
                    return Err(format!("{} has no port {port:?}", e.name));
                }
            }
        }
    }
    Ok(())
}

/// Port IDs are unique and every occupied space or position is taken once
/// (tandem halves A and B may share a breaker space).
fn validate_ports(e: &Entity) -> Result<(), String> {
    if e.ports.len() > 256 {
        return Err(format!("{}: at most 256 ports", e.name));
    }
    let mut ids = BTreeSet::new();
    let mut taken: BTreeMap<u32, Option<&str>> = BTreeMap::new();
    for p in &e.ports {
        if p.id.trim().is_empty() || p.id.len() > 40 || !ids.insert(p.id.as_str()) {
            return Err(format!("{}: port IDs must be nonempty and unique", e.name));
        }
        if p.label.len() > 200 || !(1..=3).contains(&p.poles()) {
            return Err(format!("{}: port {} needs 1-3 poles", e.name, p.id));
        }
        if p.half.as_deref().is_some_and(|h| h != "A" && h != "B") {
            return Err(format!("{}: port {} half must be A or B", e.name, p.id));
        }
        if !p.halves.is_empty()
            && (p.half.is_some()
                || p.halves.len() != p.poles() as usize
                || p.halves
                    .iter()
                    .any(|h| !matches!(h.as_str(), "A" | "B" | "")))
        {
            return Err(format!(
                "{}: port {} halves needs one of A, B or \"\" per pole (and no half)",
                e.name, p.id
            ));
        }
        if p.main && p.position.is_some() {
            return Err(format!(
                "{}: main port {} takes no position (a back-fed main in spaces is an ordinary breaker)",
                e.name, p.id
            ));
        }
        let Some(start) = p.position else { continue };
        if start == 0 || start > 999 {
            return Err(format!("{}: port {} position must be 1-999", e.name, p.id));
        }
        let step = if e.faceplate == Some(Faceplate::Panel) {
            2
        } else {
            1
        };
        for (k, space) in (0..p.poles()).map(|k| (k, start + k * step)) {
            let half = p.half_at(k);
            match taken.get(&space) {
                // Two halves of one tandem space.
                Some(Some(other)) if half.is_some_and(|h| h != *other) => {
                    taken.insert(space, None);
                }
                Some(_) => {
                    return Err(format!(
                        "{}: position {space} is used twice (port {})",
                        e.name, p.id
                    ));
                }
                None => {
                    taken.insert(space, half);
                }
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
pub fn validate_view(view: &View) -> Result<(), String> {
    if view
        .positions
        .values()
        .chain(view.attachment_hints.values())
        .any(|p| !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e6 || p.y.abs() > 1e6)
    {
        return Err("Invalid coordinates".into());
    }
    if view.faceplates.len() > 500
        || view
            .focus
            .as_ref()
            .is_some_and(|f| f.entities.len() > 500 || f.hops.is_some_and(|h| h > 50))
    {
        return Err("View focus or faceplates out of range".into());
    }
    Ok(())
}

/// Measure the full hierarchy before applying collapse. Sibling slots remain
/// reserved, so folding a room never moves another room or loses hidden offsets.
/// Where a wire attaches to a faceplate node: the named port's row, or the
/// "not assigned" row for a connection without one. x encodes the edge
/// (-1 left, 1 right, 0 either); y is relative to the node.
fn faceplate_pin(n: &Node, port: Option<&str>) -> Option<Point> {
    let f = n.faceplate.as_ref()?;
    let b = match port {
        Some(id) => f.ports.iter().find(|b| b.id == id)?,
        None => f.unassigned.as_ref()?,
    };
    Some(Point {
        x: match b.side {
            Side::Left => -1.,
            Side::Right => 1.,
            Side::Either => 0.,
        },
        y: b.y + b.height / 2.,
    })
}
const FACE_HEADER: f64 = 56.;
const FACE_ROW: f64 = 24.;
/// Physical layouts for the entities the view shows as faceplates: leaves
/// (a space keeps its contents view) that have ports or a faceplate kind.
fn faceplate_layouts(doc: &Document, view: &View) -> BTreeMap<String, (Size, FaceplateLayout)> {
    let mut out = BTreeMap::new();
    for e in &doc.entities {
        if !view.faceplates.contains(&e.id)
            || e.kind == "space"
            || doc
                .entities
                .iter()
                .any(|c| c.parent.as_deref() == Some(e.id.as_str()))
            || (e.faceplate.is_none() && e.ports.is_empty())
        {
            continue;
        }
        let unassigned = doc.connections.iter().any(|c| {
            !view.hidden_layers.contains(&c.layer)
                && c.endpoints
                    .iter()
                    .any(|ep| ep.entity == e.id && ep.port.is_none())
        });
        out.insert(e.id.clone(), faceplate_layout(e, unassigned));
    }
    out
}
fn faceplate_layout(e: &Entity, unassigned: bool) -> (Size, FaceplateLayout) {
    let kind = e.faceplate.unwrap_or(Faceplate::Terminals);
    let row_box = |x: f64, row: f64, rows: f64, width: f64, side: Side| PortBox {
        id: String::new(),
        label: String::new(),
        position: String::new(),
        x,
        y: FACE_HEADER + row * FACE_ROW,
        width,
        height: rows * FACE_ROW - 4.,
        side,
        segment: 0,
    };
    let mut ports = Vec::new();
    let mut empty = Vec::new();
    let width = if kind == Faceplate::Panel { 320. } else { 240. };
    // Supplies first: a row each above the numbered positions.
    let mains: Vec<&Port> = e.ports.iter().filter(|p| p.main).collect();
    for (i, p) in mains.iter().enumerate() {
        let mut b = row_box(10., i as f64, 1., width - 20., Side::Either);
        b.id = p.id.clone();
        b.label = p.label.clone();
        b.position = "MAIN".into();
        ports.push(b);
    }
    let top = mains.len() as f64;
    let mut rows = top;
    let unpositioned: Vec<&Port> = e
        .ports
        .iter()
        .filter(|p| p.position.is_none() && !p.main)
        .collect();
    if kind == Faceplate::Panel {
        // Two columns: odd spaces down the left, even down the right.
        let col = (width - 30.) / 2.;
        let column = |space: u32| {
            if space % 2 == 1 {
                (10., Side::Left)
            } else {
                (20. + col, Side::Right)
            }
        };
        let mut used = BTreeSet::new();
        let row_of = |space: u32| top + ((space - 1) / 2) as f64;
        for p in e.ports.iter().filter(|p| p.position.is_some()) {
            let start = p.position.unwrap();
            let (x, side) = column(start);
            // Each space it covers, as a vertical span: a whole space, or
            // its top (A) or bottom (B) half. Adjacent spans merge into one
            // box (a 2-pole breaker, or 7B/9A); a gap starts a new segment.
            let half_h = (FACE_ROW - 4.) / 2. - 1.;
            let mut spans: Vec<(f64, f64, String)> = Vec::new();
            for k in 0..p.poles() {
                let space = start + 2 * k;
                used.insert(space);
                let y0 = FACE_HEADER + row_of(space) * FACE_ROW;
                let half = p.half_at(k);
                let (top_y, bottom_y) = match half {
                    Some("A") => (y0, y0 + half_h),
                    Some(_) => (y0 + half_h + 2., y0 + FACE_ROW - 4.),
                    None => (y0, y0 + FACE_ROW - 4.),
                };
                let label = format!("{space}{}", half.unwrap_or(""));
                let joins = k > 0 && p.half_at(k - 1) != Some("A") && half != Some("B");
                match spans.last_mut() {
                    Some(last) if joins => {
                        last.1 = bottom_y;
                        last.2 = format!("{}/{label}", last.2);
                    }
                    _ => spans.push((top_y, bottom_y, label)),
                }
                rows = rows.max(row_of(space) + 1.);
            }
            for (i, (top_y, bottom_y, label)) in spans.into_iter().enumerate() {
                let mut b = row_box(x, 0., 1., col, side);
                b.y = top_y;
                b.height = bottom_y - top_y;
                b.id = p.id.clone();
                b.segment = i as u32;
                b.position = label;
                if i == 0 {
                    b.label = p.label.clone();
                    b.position = p.spaces_label(2);
                }
                ports.push(b);
            }
        }
        let last = used.iter().max().copied().unwrap_or(0);
        for space in 1..=last + (last % 2) {
            if !used.contains(&space) {
                let (x, side) = column(space);
                let mut b = row_box(x, row_of(space), 1., col, side);
                b.position = space.to_string();
                empty.push(b);
                rows = rows.max(row_of(space) + 1.);
            }
        }
    } else {
        // One row per port, in port order; gaps in the numbering stay visible.
        let mut positioned: Vec<&Port> = e.ports.iter().filter(|p| p.position.is_some()).collect();
        positioned.sort_by_key(|p| p.position);
        let mut taken = BTreeSet::new();
        for p in &positioned {
            let at = p.position.unwrap();
            for k in 0..p.poles() {
                taken.insert(at + k);
            }
            let mut b = row_box(
                10.,
                top + (at - 1) as f64,
                p.poles() as f64,
                width - 20.,
                Side::Either,
            );
            b.id = p.id.clone();
            b.label = p.label.clone();
            b.position = at.to_string();
            rows = rows.max(top + (at - 1 + p.poles()) as f64);
            ports.push(b);
        }
        for at in 1..=(rows - top) as u32 {
            if !taken.contains(&at) {
                let mut b = row_box(10., top + (at - 1) as f64, 1., width - 20., Side::Either);
                b.position = at.to_string();
                empty.push(b);
            }
        }
    }
    // Ports with no recorded position follow the numbered ones.
    for p in unpositioned {
        let mut b = row_box(10., rows, p.poles() as f64, width - 20., Side::Either);
        b.id = p.id.clone();
        b.label = p.label.clone();
        rows += p.poles() as f64;
        ports.push(b);
    }
    let unassigned = unassigned.then(|| {
        let mut b = row_box(10., rows, 1., width - 20., Side::Either);
        b.label = "not assigned to a port".into();
        rows += 1.;
        b
    });
    let size = Size {
        width,
        height: FACE_HEADER + rows.max(1.) * FACE_ROW + 8.,
    };
    (
        size,
        FaceplateLayout {
            kind,
            ports,
            empty,
            unassigned,
        },
    )
}
fn measure(
    id: &str,
    depth: usize,
    doc: &Document,
    faces: &BTreeMap<String, (Size, FaceplateLayout)>,
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
        let size = faces.get(id).map(|(size, _)| *size).unwrap_or(Size {
            width: CARD_WIDTH,
            height: CARD_HEIGHT,
        });
        sizes.insert(id.into(), size);
        return size;
    }
    let child_sizes: Vec<_> = children
        .iter()
        .map(|e| measure(&e.id, depth + 1, doc, faces, positions, sizes))
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
    sizes.insert(id.into(), extent);
    extent
}

/// (layer, visible endpoints, pinned ports). Connections to different ports
/// of a faceplate are different wires, so the pins are part of the key.
type BundleKey = (String, Vec<(String, Role)>, Vec<(String, String)>);
/// Visible connections grouped into bundles: one per layer and set of
/// visible endpoints (an endpoint inside a collapsed space projects onto it).
fn bundle_groups(doc: &Document, view: &View) -> BTreeMap<BundleKey, Vec<String>> {
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
    let mut groups: BTreeMap<BundleKey, Vec<String>> = BTreeMap::new();
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
        // A port pins its wire only where the faceplate itself is drawn.
        let mut pins: Vec<(String, String)> = c
            .endpoints
            .iter()
            .filter(|ep| {
                view.faceplates.contains(&ep.entity) && visible_ancestor(&ep.entity) == ep.entity
            })
            .filter_map(|ep| ep.port.clone().map(|p| (ep.entity.clone(), p)))
            .collect();
        pins.sort();
        groups
            .entry((c.layer.clone(), eps, pins))
            .or_default()
            .push(c.id.clone());
    }
    groups
}
/// Box geometry without any routing: parent-relative positions, each
/// container's expanded size, and the visible nodes in world coordinates.
struct Placement {
    positions: BTreeMap<String, Point>,
    nodes: Vec<Node>,
}
fn place(doc: &Document, view: &View) -> Placement {
    // Old flat-card coordinates cannot describe containment. Rebuild their
    // geometry once; keep the view's fold/layer choices and the inventory.
    let mut positions = if view.layout_version >= 2 {
        view.positions.clone()
    } else {
        BTreeMap::new()
    };
    positions.retain(|id, _| doc.entities.iter().any(|e| &e.id == id));
    let mut sizes = BTreeMap::new();
    let faces = faceplate_layouts(doc, view);
    let roots: Vec<_> = doc.entities.iter().filter(|e| e.parent.is_none()).collect();
    let mut root_y = 40.;
    for root in &roots {
        let size = measure(&root.id, 0, doc, &faces, &mut positions, &mut sizes);
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
        (positions, sizes, faces): (
            &BTreeMap<String, Point>,
            &BTreeMap<String, Size>,
            &BTreeMap<String, (Size, FaceplateLayout)>,
        ),
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
            faceplate: faces.get(id).map(|(_, f)| f.clone()),
        });
        if !collapsed {
            for child in children {
                collect(
                    &child.id,
                    position,
                    depth + 1,
                    doc,
                    view,
                    (positions, sizes, faces),
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
            (&positions, &sizes, &faces),
            &mut nodes,
        );
    }
    Placement { positions, nodes }
}
/// The things a focus shows: its entities (a space brings its contents),
/// then whatever connections reach from them, `hops` steps at most, in the
/// focus direction. Hidden layers are not followed.
pub fn focus_set(doc: &Document, view: &View, focus: &Focus) -> BTreeSet<String> {
    let exists: BTreeSet<&str> = doc.entities.iter().map(|e| e.id.as_str()).collect();
    let mut reached: BTreeSet<String> = focus
        .entities
        .iter()
        .filter(|id| exists.contains(id.as_str()))
        .cloned()
        .collect();
    loop {
        let inside: Vec<String> = doc
            .entities
            .iter()
            .filter(|e| {
                e.parent.as_ref().is_some_and(|p| reached.contains(p)) && !reached.contains(&e.id)
            })
            .map(|e| e.id.clone())
            .collect();
        if inside.is_empty() {
            break;
        }
        reached.extend(inside);
    }
    let mut frontier = reached.clone();
    let mut depth = 0;
    while !frontier.is_empty() && focus.hops.is_none_or(|h| depth < h) {
        let mut next = BTreeSet::new();
        for c in doc
            .connections
            .iter()
            .filter(|c| !view.hidden_layers.contains(&c.layer))
        {
            for from in c
                .endpoints
                .iter()
                .filter(|ep| frontier.contains(&ep.entity))
            {
                for to in c.endpoints.iter().filter(|ep| ep.entity != from.entity) {
                    let follows = match focus.direction {
                        Direction::Both => true,
                        Direction::Upstream => {
                            (from.role == Role::Target && to.role == Role::Source)
                                || (from.role == Role::Peer && to.role == Role::Peer)
                        }
                        Direction::Downstream => {
                            (from.role == Role::Source && to.role == Role::Target)
                                || (from.role == Role::Peer && to.role == Role::Peer)
                        }
                    };
                    if follows && reached.insert(to.entity.clone()) {
                        next.insert(to.entity.clone());
                    }
                }
            }
        }
        frontier = next;
        depth += 1;
    }
    reached
}
/// The document and view a focused layout is computed from: only the
/// focused things, their enclosing spaces (expanded) and the connections
/// among them, laid out compactly from scratch. The caller's positions and
/// attachment hints are untouched; they belong to the full view.
fn focus_projection(doc: &Document, view: &View) -> Option<(Document, View)> {
    let focus = view.focus.as_ref()?;
    let shown = focus_set(doc, view, focus);
    let parent = |id: &str| {
        doc.entities
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.parent.clone())
    };
    let mut context = BTreeSet::new();
    for id in &shown {
        let mut at = parent(id);
        while let Some(p) = at {
            at = parent(&p);
            context.insert(p);
        }
    }
    let keep = |id: &String| shown.contains(id) || context.contains(id);
    let projected = Document {
        entities: doc
            .entities
            .iter()
            .filter(|e| keep(&e.id))
            .cloned()
            .collect(),
        connections: doc
            .connections
            .iter()
            .filter_map(|c| {
                let endpoints: Vec<_> = c
                    .endpoints
                    .iter()
                    .filter(|ep| shown.contains(&ep.entity))
                    .cloned()
                    .collect();
                (endpoints.len() >= 2).then(|| Connection {
                    endpoints,
                    ..c.clone()
                })
            })
            .collect(),
        layers: doc.layers.clone(),
    };
    let collapsed = view
        .collapsed
        .iter()
        .filter(|id| !context.contains(*id) && !focus.entities.contains(id))
        .cloned()
        .collect();
    Some((
        projected,
        View {
            layout_version: 2,
            positions: BTreeMap::new(),
            collapsed,
            hidden_layers: view.hidden_layers.clone(),
            attachment_hints: BTreeMap::new(),
            faceplates: view.faceplates.clone(),
            focus: None,
        },
    ))
}
fn focused(
    req: &LayoutRequest,
    lay: fn(&LayoutRequest) -> Result<Layout, String>,
) -> Option<Result<Layout, String>> {
    let (document, view) = focus_projection(&req.document, &req.view)?;
    Some(lay(&LayoutRequest { document, view }).map(|mut l| {
        l.positions = req.view.positions.clone();
        l.attachment_hints = req.view.attachment_hints.clone();
        l
    }))
}
pub fn layout(req: &LayoutRequest) -> Result<Layout, String> {
    validate(&req.document)?;
    validate_view(&req.view)?;
    if let Some(result) = focused(req, layout) {
        return result;
    }
    let doc = &req.document;
    let view = &req.view;
    let Placement {
        positions, nodes, ..
    } = place(doc, view);
    let groups = bundle_groups(doc, view);
    let mut attachment_hints = view.attachment_hints.clone();
    let mut reserved = Vec::new();
    let mut bundles = Vec::new();
    for (channel, ((layer, eps, pins), connections)) in groups.into_iter().enumerate() {
        let mut router = HierarchyRouter {
            document: doc,
            nodes: &nodes,
            tracks: &reserved,
            preferences: BTreeMap::new(),
            attachments: BTreeMap::new(),
            routes: Default::default(),
            obstacle_sets: Default::default(),
        };
        let hint_prefix = format!("{}|", serde_json::to_string(&connections).unwrap());
        router.attachments = view
            .attachment_hints
            .iter()
            .filter_map(|(k, v)| k.strip_prefix(&hint_prefix).map(|k| (k.to_owned(), *v)))
            .collect();
        // Faceplate endpoints attach at their port's row (see branch()).
        for (id, _) in &eps {
            let port = pins.iter().find(|(e, _)| e == id).map(|(_, p)| p.as_str());
            if let Some(pin) = router
                .nodes
                .iter()
                .find(|n| &n.id == id)
                .and_then(|n| faceplate_pin(n, port))
            {
                router.attachments.insert(format!("pin:{id}"), pin);
            }
        }
        let first_path = router.ancestors(&eps[0].0);
        let common = first_path
            .iter()
            .find(|ancestor| {
                eps.iter()
                    .all(|(id, _)| router.ancestors(id).contains(ancestor))
            })
            .map(String::as_str);
        router.plan_corridors(common, &eps);
        let mut branches = router.bundle(&eps, common, channel);
        // Do not display a partial hyperedge as though it were complete. Keep
        // hard spacing when a corridor is full and report the missing connection.
        if branches.iter().any(|branch| branch.points.is_empty()) {
            for branch in &mut branches {
                branch.points.clear();
            }
        }
        for branch in &branches {
            if let Some(port) = branch.points.first() {
                let n = router.node(&branch.entity);
                attachment_hints.insert(
                    format!("{}port:{}", hint_prefix, branch.entity),
                    Point {
                        x: if port.x == n.position.x { -1. } else { 1. },
                        y: port.y - n.position.y,
                    },
                );
                for id in router
                    .ancestors(&branch.entity)
                    .iter()
                    .take_while(|id| Some(id.as_str()) != common)
                {
                    let n = router.node(id);
                    for pair in branch.points.windows(2).filter(|p| p[0].y == p[1].y) {
                        for (x, side) in [(n.position.x, -1.), (n.position.x + n.width, 1.)] {
                            if pair[0].x.min(pair[1].x) < x
                                && pair[0].x.max(pair[1].x) > x
                                && pair[0].y >= n.position.y + 60.
                                && pair[0].y <= n.position.y + n.height
                            {
                                attachment_hints.insert(
                                    format!("{}gate:{}", hint_prefix, id),
                                    Point {
                                        x: side,
                                        y: pair[0].y - n.position.y,
                                    },
                                );
                            }
                        }
                    }
                }
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
        attachment_hints,
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
    preferences: BTreeMap<String, Vec<f64>>,
    attachments: BTreeMap<String, Point>,
    /// Segments already routed by this router. A router's inputs are fixed
    /// for its lifetime — the reserved tracks are borrowed immutably, and
    /// lane preferences only change in `plan_corridors`, which clears this —
    /// so a segment is a pure function of (start, end, domain). Junction
    /// candidates and port alternatives ask for the same segments many
    /// times; this answers the repeats without re-running the search.
    routes: std::cell::RefCell<std::collections::HashMap<RouteKey, Vec<Point>>>,
    /// Each domain's obstacle boxes (its immediate children), built once:
    /// the nodes are fixed for the router's lifetime.
    obstacle_sets: std::cell::RefCell<std::collections::HashMap<Option<String>, Vec<Rect>>>,
}
type RouteKey = ([u64; 4], Option<String>);
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
#[derive(Clone)]
struct LanePool {
    center: f64,
    lanes: Vec<f64>,
}
fn vertical_affinity(a: Point, b: Point, lanes: &[f64]) -> f64 {
    if a.x != b.x || lanes.is_empty() {
        return 0.;
    }
    let offset = lanes
        .iter()
        .map(|x| (a.x - x).abs())
        .fold(f64::INFINITY, f64::min);
    (a.y - b.y).abs() * offset * 0.01
}
fn routing_cost(path: &[Point], lanes: &[f64]) -> f64 {
    path.windows(2)
        .map(|p| distance(p[0], p[1]) + vertical_affinity(p[0], p[1], lanes))
        .sum::<f64>()
        + path.len() as f64 * 28.
}
impl HierarchyRouter<'_> {
    fn preferred(&self, domain: Option<&str>) -> &[f64] {
        self.preferences
            .get(domain.unwrap_or(""))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
    /// Stable lane pools come from the expanded sibling footprints, so folding
    /// a space does not change which gap a connection family uses.
    fn lane_pools(&self, domain: Option<&str>) -> Vec<LanePool> {
        let mut occupied: Vec<_> = self
            .nodes
            .iter()
            .filter(|n| self.parent(&n.id) == domain)
            .map(|n| (n.position.x, n.position.x + n.expanded_width))
            .collect();
        if occupied.is_empty() {
            return Vec::new();
        }
        occupied.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (left, right) = self
            .bounds(domain)
            .map(|b| (b.position.x, b.position.x + b.width))
            .unwrap_or((
                occupied[0].0 - 32.,
                occupied
                    .iter()
                    .map(|p| p.1)
                    .fold(f64::NEG_INFINITY, f64::max)
                    + 32.,
            ));
        let mut gaps = Vec::new();
        let mut cursor = left;
        for (lo, hi) in occupied {
            let lo = lo.clamp(left, right);
            let hi = hi.clamp(left, right);
            if lo > cursor {
                gaps.push((cursor, lo));
            }
            cursor = cursor.max(hi);
        }
        if cursor < right {
            gaps.push((cursor, right));
        }
        gaps.into_iter()
            .filter_map(|(left, right)| {
                let (lo, hi) = (left + 8., right - 8.);
                if hi < lo {
                    return None;
                }
                let count = (((hi - lo) / TRACK_SPACING).floor() as usize + 1).min(9);
                let center = (left + right) / 2.;
                Some(LanePool {
                    center,
                    lanes: (0..count)
                        .map(|i| center + (i as f64 - (count - 1) as f64 / 2.) * TRACK_SPACING)
                        .collect(),
                })
            })
            .collect()
    }
    fn projected_child(&self, id: &str, domain: Option<&str>) -> Option<String> {
        let mut at = id;
        loop {
            let parent = self.parent(at);
            if parent == domain {
                return Some(at.into());
            }
            at = parent?;
        }
    }
    fn plan_corridors(&mut self, common: Option<&str>, eps: &[(String, Role)]) {
        let pools = self.lane_pools(common);
        if pools.is_empty() {
            return;
        }
        let all: BTreeSet<_> = eps
            .iter()
            .filter_map(|(id, _)| self.projected_child(id, common))
            .collect();
        let mut sources: BTreeSet<_> = eps
            .iter()
            .filter(|(_, role)| *role == Role::Source)
            .filter_map(|(id, _)| self.projected_child(id, common))
            .collect();
        if sources.is_empty() {
            if let Some(id) = all
                .iter()
                .min_by(|a, b| self.node(a).position.x.total_cmp(&self.node(b).position.x))
            {
                sources.insert(id.clone());
            }
        }
        let mut targets: Vec<_> = all.iter().filter(|id| !sources.contains(*id)).collect();
        if targets.is_empty() {
            targets = all.iter().collect();
        }
        let source_left = sources
            .iter()
            .map(|id| self.node(id).position.x)
            .fold(f64::INFINITY, f64::min);
        let source_right = sources
            .iter()
            .map(|id| {
                let n = self.node(id);
                n.position.x + n.expanded_width
            })
            .fold(f64::NEG_INFINITY, f64::max);
        let target_left = targets
            .iter()
            .map(|id| self.node(id).position.x)
            .fold(f64::INFINITY, f64::min);
        let target_right = targets
            .iter()
            .map(|id| {
                let n = self.node(id);
                n.position.x + n.expanded_width
            })
            .fold(f64::NEG_INFINITY, f64::max);
        let middle = all
            .iter()
            .map(|id| {
                let n = self.node(id);
                n.position.x + n.expanded_width / 2.
            })
            .sum::<f64>()
            / all.len().max(1) as f64;
        let selected = pools
            .iter()
            .filter(|p| {
                if target_left >= source_right {
                    p.center > source_right
                } else if target_right <= source_left {
                    p.center < source_left
                } else {
                    true
                }
            })
            .min_by(|a, b| {
                let score = |p: &LanePool| {
                    if target_left >= source_right {
                        p.center - source_right
                    } else if target_right <= source_left {
                        source_left - p.center
                    } else {
                        (p.center - middle).abs()
                    }
                };
                score(a).total_cmp(&score(b))
            })
            .unwrap_or_else(|| {
                pools
                    .iter()
                    .min_by(|a, b| {
                        (a.center - middle)
                            .abs()
                            .total_cmp(&(b.center - middle).abs())
                    })
                    .unwrap()
            });
        self.preferences
            .insert(common.unwrap_or("").into(), selected.lanes.clone());
        self.routes.get_mut().clear();
        let spaces: BTreeSet<_> = eps
            .iter()
            .flat_map(|(id, _)| {
                self.ancestors(id)
                    .into_iter()
                    .take_while(|id| Some(id.as_str()) != common)
            })
            .collect();
        let mut spaces: Vec<_> = spaces.into_iter().collect();
        spaces.sort_by_key(|id| self.node(id).depth);
        for id in spaces {
            let lanes = self.preferred(self.parent(&id));
            if lanes.is_empty() {
                continue;
            }
            let goal = (lanes[0] + lanes[lanes.len() - 1]) / 2.;
            let pools = self.lane_pools(Some(&id));
            if let Some(pool) = pools
                .iter()
                .min_by(|a, b| (a.center - goal).abs().total_cmp(&(b.center - goal).abs()))
            {
                self.preferences.insert(id, pool.lanes.clone());
                self.routes.get_mut().clear();
            }
        }
    }
    /// Score complete routes in the corridor of each hierarchy level. This
    /// keeps bend removal from undoing a useful central dogleg.
    fn whole_cost(&self, entity: &str, path: &[Point]) -> f64 {
        let ancestors = self.ancestors(entity);
        let mut cost = path.len() as f64 * 28.;
        for pair in path.windows(2) {
            let a = pair[0];
            let b = pair[1];
            let length = distance(a, b);
            if length == 0. {
                continue;
            }
            let mut cuts = vec![0., 1.];
            for id in &ancestors {
                let n = self.node(id);
                for edge in if a.x == b.x {
                    [n.position.y, n.position.y + n.height]
                } else {
                    [n.position.x, n.position.x + n.width]
                } {
                    let t = if a.x == b.x {
                        (edge - a.y) / (b.y - a.y)
                    } else {
                        (edge - a.x) / (b.x - a.x)
                    };
                    if t > 0. && t < 1. {
                        cuts.push(t);
                    }
                }
            }
            cuts.sort_by(f64::total_cmp);
            cuts.dedup();
            for span in cuts.windows(2) {
                let point = |t: f64| Point {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                };
                let mid = point((span[0] + span[1]) / 2.);
                let domain = ancestors
                    .iter()
                    .find(|id| {
                        let n = self.node(id);
                        Rect {
                            position: n.position,
                            width: n.width,
                            height: n.height,
                        }
                        .contains(mid, 0.)
                    })
                    .map(String::as_str);
                let from = point(span[0]);
                let to = point(span[1]);
                cost += distance(from, to) + vertical_affinity(from, to, self.preferred(domain));
            }
        }
        cost
    }
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
        let key = domain.map(str::to_owned);
        if let Some(hit) = self.obstacle_sets.borrow().get(&key) {
            return hit.clone();
        }
        // The immediate child boxes include every nested space in full. Routes
        // within a room see its device boxes; floor routes see solid room boxes.
        let set: Vec<Rect> = self
            .nodes
            .iter()
            .filter(|n| self.parent(&n.id) == domain)
            .map(|n| Rect {
                position: n.position,
                width: n.width,
                height: n.height,
            })
            .collect();
        self.obstacle_sets.borrow_mut().insert(key, set.clone());
        set
    }
    fn bundle(&self, eps: &[(String, Role)], common: Option<&str>, channel: usize) -> Vec<Branch> {
        let centers: Vec<_> = eps.iter().map(|(id, _)| self.center(id)).collect();
        for point in self.junction_options(common, &centers, channel) {
            let branches = self.via_junction(eps, common, point);
            if branches.iter().all(|b| !b.points.is_empty()) {
                return branches;
            }
        }
        // Corridor affinity is a preference. Relax it only after trying the
        // distinct free-space regions, never the side-port or padding rules.
        if !self.preferences.is_empty() {
            let fallback = HierarchyRouter {
                document: self.document,
                nodes: self.nodes,
                tracks: self.tracks,
                preferences: BTreeMap::new(),
                attachments: self.attachments.clone(),
                routes: Default::default(),
                obstacle_sets: Default::default(),
            };
            for point in fallback.junction_options(common, &centers, channel) {
                let branches = fallback.via_junction(eps, common, point);
                if branches.iter().all(|b| !b.points.is_empty()) {
                    return branches;
                }
            }
        }
        eps.iter()
            .map(|(id, role)| Branch {
                entity: id.clone(),
                role: role.clone(),
                points: Vec::new(),
            })
            .collect()
    }
    fn via_junction(
        &self,
        eps: &[(String, Role)],
        common: Option<&str>,
        junction: Point,
    ) -> Vec<Branch> {
        let router = self;
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
            .iter()
            .cloned()
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
        branches
    }
    fn junction_options(
        &self,
        domain: Option<&str>,
        centers: &[Point],
        channel: usize,
    ) -> Vec<Point> {
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
        xs.extend(self.preferred(domain));
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
        let mut choices = Vec::new();
        for x in &xs {
            for y in &ys {
                let p = Point { x: *x, y: *y };
                if bounds.is_some_and(|b| !in_bounds(p, b))
                    || rects.iter().any(|r| r.contains(p, 8.))
                    || !point_clear(p, self.tracks)
                {
                    continue;
                }
                let affinity = self
                    .preferred(domain)
                    .iter()
                    .map(|lane| (p.x - lane).abs())
                    .fold(f64::INFINITY, f64::min);
                let score = centers.iter().map(|q| distance(p, *q)).sum::<f64>()
                    + if affinity.is_finite() {
                        affinity * 2.
                    } else {
                        0.
                    };
                choices.push((score, p));
            }
        }
        choices.sort_by(|a, b| {
            a.0.total_cmp(&b.0)
                .then(a.1.x.total_cmp(&b.1.x))
                .then(a.1.y.total_cmp(&b.1.y))
        });
        // Try different topological cells instead of repeatedly shifting a hub
        // within the same blocked strip. This bounds retries in dense views.
        let mut x_edges: Vec<_> = rects
            .iter()
            .flat_map(|r| [r.position.x, r.position.x + r.width])
            .collect();
        let mut y_edges: Vec<_> = rects
            .iter()
            .flat_map(|r| [r.position.y, r.position.y + r.height])
            .collect();
        x_edges.sort_by(f64::total_cmp);
        x_edges.dedup();
        y_edges.sort_by(f64::total_cmp);
        y_edges.dedup();
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for (_, point) in &choices {
            let cell = (
                x_edges.partition_point(|x| *x < point.x),
                y_edges.partition_point(|y| *y < point.y),
            );
            if seen.insert(cell) {
                result.push(*point);
                if result.len() == 24 {
                    break;
                }
            }
        }
        // A few local alternatives can release a crowded port without changing
        // which corridor is used.
        for (_, point) in choices.into_iter().take(6) {
            if !result.contains(&point) {
                result.push(point);
            }
        }
        result
    }
    fn attachment_penalty(&self, key: &str, n: &Node, p: Point) -> f64 {
        self.attachments.get(key).map_or(0., |hint| {
            let side = if p.x == n.position.x { -1. } else { 1. };
            (if side != hint.x { 160. } else { 0. }) + (p.y - n.position.y - hint.y).abs() * 2.
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
        if let Some(hint) = self.attachments.get(&format!("gate:{id}")) {
            ys.push((n.position.y + hint.y).clamp(top, bottom));
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
                let inside_cost =
                    inside_cost + self.attachment_penalty(&format!("gate:{id}"), n, gate.boundary);
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
            let cost = inside_cost + routing_cost(&path, self.preferred(parent));
            if best.as_ref().is_none_or(|(previous, _)| cost < *previous) {
                best = Some((cost, gate));
            }
        }
        best.map(|(_, gate)| gate)
    }
    fn segment(&self, start: Point, end: Point, domain: Option<&str>) -> Vec<Point> {
        let key = (
            [
                start.x.to_bits(),
                start.y.to_bits(),
                end.x.to_bits(),
                end.y.to_bits(),
            ],
            domain.map(str::to_owned),
        );
        if let Some(hit) = self.routes.borrow().get(&key) {
            return hit.clone();
        }
        let path = route(
            start,
            end,
            &self.obstacles(domain),
            self.bounds(domain),
            self.tracks,
            self.preferred(domain),
        );
        self.routes.borrow_mut().insert(key, path.clone());
        path
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
        let mut ys = offsets(mid_y, min_y, max_y);
        if let Some(hint) = self.attachments.get(&format!("port:{id}")) {
            ys.insert(0, (n.position.y + hint.y).clamp(min_y, max_y));
        }
        // A faceplate pins the wire to its port's row and edge.
        let pin = self.attachments.get(&format!("pin:{id}")).copied();
        if let Some(pin) = pin {
            ys = vec![n.position.y + pin.y];
        }
        for y in ys {
            if pin.is_none_or(|p| p.x <= 0.) {
                ports.push((
                    Point { x: n.position.x, y },
                    Point {
                        x: n.position.x - 12.,
                        y,
                    },
                ));
            }
            if pin.is_none_or(|p| p.x >= 0.) {
                ports.push((
                    Point {
                        x: n.position.x + n.width,
                        y,
                    },
                    Point {
                        x: n.position.x + n.width + 12.,
                        y,
                    },
                ));
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
            let score = routing_cost(&path, self.preferred(self.parent(id)))
                + self.attachment_penalty(&format!("port:{id}"), n, port);
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
fn side_exit(path: &[Point], rect: Rect) -> bool {
    path.windows(2)
        .find(|p| in_bounds(p[0], rect) && !in_bounds(p[1], rect))
        .is_some_and(|p| {
            p[0].y == p[1].y
                && p[0].y >= rect.position.y + 60. - GEOMETRY_EPS
                && p[0].y <= rect.position.y + rect.height - 8. + GEOMETRY_EPS
        })
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
        self.boxes.iter().all(|(_, rect)| {
            single_exit(path, *rect)
                && side_exit(path, *rect)
                && !in_bounds(*path.last().unwrap(), *rect)
        })
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
            let baseline_cost: f64 = branches
                .iter()
                .map(|b| self.whole_cost(&b.entity, &b.points))
                .sum();
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
                        || branches
                            .iter()
                            .zip(&paths)
                            .map(|(b, p)| self.whole_cost(&b.entity, p))
                            .sum::<f64>()
                            > baseline_cost + GEOMETRY_EPS
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
            let baseline_cost = self.whole_cost(&branch.entity, &path);
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
                        if candidate.len() < path.len()
                            && valid(&candidate)
                            && self.whole_cost(&branch.entity, &candidate)
                                <= baseline_cost + GEOMETRY_EPS
                        {
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
    /// An intermediate drag frame rather than the drop.
    #[serde(default)]
    pub dragging: bool,
}
/// Covering this share of the smaller box (or its centre) swaps two siblings.
const SWAP_SHARE: f64 = 1. / 3.;
fn overlap_area(a: Rect, b: Rect) -> f64 {
    let w = (a.position.x + a.width).min(b.position.x + b.width) - a.position.x.max(b.position.x);
    let h = (a.position.y + a.height).min(b.position.y + b.height) - a.position.y.max(b.position.y);
    w.max(0.) * h.max(0.)
}
/// Move in world coordinates while retaining parent-relative offsets. Dragging
/// a space moves its hidden and visible descendants without rewriting them.
///
/// Siblings make room instead of overlapping. If the dragged box's centre is
/// over a sibling, the two swap: the sibling takes the slot the dragged box
/// left. Any remaining overlap pushes siblings aside (cascading), and a
/// container that grows pushes its own siblings in turn, up to the roots.
/// Everything is computed from `view` as given, so a drag that sends its
/// starting view every frame lets pushed boxes spring back as it moves on.
pub fn move_entity(req: &MoveRequest) -> Result<View, String> {
    if !req.position.x.is_finite()
        || !req.position.y.is_finite()
        || req.position.x.abs() > 1e6
        || req.position.y.abs() > 1e6
    {
        return Err("Invalid coordinates".into());
    }
    validate(&req.document)?;
    validate_view(&req.view)?;
    let doc = &req.document;
    let base = place(doc, &req.view);
    let visible: BTreeMap<&str, &Node> = base.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    visible
        .get(req.entity.as_str())
        .ok_or("Move a visible entity")?;
    let parent_of = |id: &str| {
        doc.entities
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.parent.clone())
    };
    let mut target = req.position;
    if let Some(parent) = parent_of(&req.entity) {
        let parent = visible.get(parent.as_str()).ok_or("Parent is hidden")?;
        target.x -= parent.position.x;
        target.y -= parent.position.y;
    }
    // Visible footprint: a collapsed space occupies a card, not its contents.
    let mut sizes: BTreeMap<String, Size> = base
        .nodes
        .iter()
        .map(|n| {
            (
                n.id.clone(),
                Size {
                    width: n.width,
                    height: n.height,
                },
            )
        })
        .collect();
    let mut positions = base.positions.clone();
    let mut moved = req.entity.clone();
    let mut home = Some(base.positions[&req.entity]);
    loop {
        let parent = parent_of(&moved);
        let siblings: Vec<String> = doc
            .entities
            .iter()
            .filter(|e| e.parent == parent && visible.contains_key(e.id.as_str()))
            .map(|e| e.id.clone())
            .collect();
        positions.insert(moved.clone(), target);
        let mut fixed = vec![moved.clone()];
        if let Some(home) = home.take() {
            let size = sizes[&moved];
            let dragged = Rect {
                position: target,
                width: size.width,
                height: size.height,
            };
            let centre = Point {
                x: target.x + size.width / 2.,
                y: target.y + size.height / 2.,
            };
            // How much of the smaller box the drag covers, per sibling (at
            // its starting position, so a held box cannot slide away).
            let mut covered: Vec<(f64, &String)> = siblings
                .iter()
                .filter(|id| **id != moved)
                .filter_map(|id| {
                    let r = Rect {
                        position: base.positions[id],
                        width: sizes[id].width,
                        height: sizes[id].height,
                    };
                    let area = overlap_area(dragged, r);
                    (area > 0.).then(|| {
                        let smaller = (r.width * r.height).min(size.width * size.height);
                        let share = if r.contains(centre, 0.) {
                            1.
                        } else {
                            area / smaller
                        };
                        (share, id)
                    })
                })
                .collect();
            covered.sort_by(|a, b| b.0.total_cmp(&a.0));
            if let Some((share, other)) = covered.first() {
                // A box that has not moved (re-dropped in place to settle a
                // size change) pushes what it covers; it never swaps.
                if *share >= SWAP_SHARE && target != home {
                    positions.insert((*other).clone(), home);
                    fixed.push((*other).clone());
                    covered.remove(0);
                }
            }
            // Mid-drag, a box being moved onto stays put until the swap
            // threshold; dropping short of it pushes the box aside instead.
            if req.dragging {
                fixed.extend(covered.into_iter().map(|(_, id)| id.clone()));
            }
        }
        separate(&siblings, &fixed, &mut positions, &sizes);
        let Some(parent) = parent else { break };
        // Grow toward the top/left by moving the container origin and
        // compensating its children, so their world positions hold.
        let dx = siblings
            .iter()
            .map(|id| PADDING - positions[id].x)
            .fold(0., f64::max);
        let dy = siblings
            .iter()
            .map(|id| CONTENT_Y - positions[id].y)
            .fold(0., f64::max);
        for id in &siblings {
            let p = positions.get_mut(id).unwrap();
            p.x += dx;
            p.y += dy;
        }
        let origin = positions.get_mut(&parent).unwrap();
        origin.x -= dx;
        origin.y -= dy;
        // The container's new extent, as `measure` will compute it.
        let mut extent = Size {
            width: 272.,
            height: 148.,
        };
        for id in &siblings {
            extent.width = extent
                .width
                .max(positions[id].x + sizes[id].width + PADDING);
            extent.height = extent
                .height
                .max(positions[id].y + sizes[id].height + PADDING);
        }
        sizes.insert(parent.clone(), extent);
        target = positions[&parent];
        moved = parent;
    }
    let mut view = req.view.clone();
    view.layout_version = 2;
    view.positions = positions;
    Ok(view)
}
/// Push apart overlapping boxes among `ids` (parent-relative positions),
/// keeping at least `GAP` between them. `fixed` boxes never move; the rest
/// yield to boxes nearer the first fixed one, along the axis that needs the
/// smaller move, away from the box pushing them.
fn separate(
    ids: &[String],
    fixed: &[String],
    positions: &mut BTreeMap<String, Point>,
    sizes: &BTreeMap<String, Size>,
) {
    let centre = |positions: &BTreeMap<String, Point>, id: &str| {
        let (p, s) = (positions[id], sizes[id]);
        Point {
            x: p.x + s.width / 2.,
            y: p.y + s.height / 2.,
        }
    };
    let anchor = centre(positions, &fixed[0]);
    let mut rest: Vec<&String> = ids.iter().filter(|id| !fixed.contains(id)).collect();
    rest.sort_by(|a, b| {
        distance(centre(positions, a), anchor).total_cmp(&distance(centre(positions, b), anchor))
    });
    let order: Vec<&String> = fixed.iter().chain(rest).collect();
    for _ in 0..64 {
        let mut changed = false;
        for i in 0..order.len() {
            for j in (i + 1).max(fixed.len())..order.len() {
                let (a, b) = (order[i], order[j]);
                let (pa, sa, pb, sb) = (positions[a], sizes[a], positions[b], sizes[b]);
                let right = pa.x + sa.width + GAP - pb.x;
                let left = pb.x + sb.width + GAP - pa.x;
                let down = pa.y + sa.height + GAP - pb.y;
                let up = pb.y + sb.height + GAP - pa.y;
                if right.min(left) <= GEOMETRY_EPS || down.min(up) <= GEOMETRY_EPS {
                    continue;
                }
                let (ca, cb) = (centre(positions, a), centre(positions, b));
                let p = positions.get_mut(b).unwrap();
                if right.min(left) <= down.min(up) {
                    p.x += if cb.x >= ca.x { right } else { -left };
                } else {
                    p.y += if cb.y >= ca.y { down } else { -up };
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}
/// A routing-free layout for interactive drags: box geometry is exact, and
/// each bundle is drawn as plain elbows from its endpoints' facing sides to
/// the endpoints' centroid. The full `layout` replaces it on drop.
pub fn preview_layout(req: &LayoutRequest) -> Result<Layout, String> {
    validate(&req.document)?;
    validate_view(&req.view)?;
    if let Some(result) = focused(req, preview_layout) {
        return result;
    }
    let Placement {
        positions, nodes, ..
    } = place(&req.document, &req.view);
    let by_id: BTreeMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let centre = |n: &Node| Point {
        x: n.position.x + n.width / 2.,
        y: n.position.y + n.height / 2.,
    };
    let mut bundles = Vec::new();
    for ((layer, eps, pins), connections) in bundle_groups(&req.document, &req.view) {
        let ends: Vec<_> = eps
            .iter()
            .filter_map(|(id, role)| by_id.get(id.as_str()).map(|n| (*n, role)))
            .collect();
        if ends.len() < 2 {
            continue;
        }
        let hub = Point {
            x: ends.iter().map(|(n, _)| centre(n).x).sum::<f64>() / ends.len() as f64,
            y: ends.iter().map(|(n, _)| centre(n).y).sum::<f64>() / ends.len() as f64,
        };
        let branches = ends
            .iter()
            .map(|(n, role)| {
                let mut c = centre(n);
                let port = pins
                    .iter()
                    .find(|(e, _)| *e == n.id)
                    .map(|(_, p)| p.as_str());
                let pin = faceplate_pin(n, port);
                if let Some(pin) = pin {
                    c.y = n.position.y + pin.y;
                }
                let right = match pin.map(|p| p.x) {
                    Some(x) if x < 0. => false,
                    Some(x) if x > 0. => true,
                    _ => hub.x >= c.x,
                };
                let side = if right {
                    n.position.x + n.width
                } else {
                    n.position.x
                };
                Branch {
                    entity: n.id.clone(),
                    role: (*role).clone(),
                    points: simplify(vec![
                        Point { x: side, y: c.y },
                        Point { x: hub.x, y: c.y },
                        hub,
                    ]),
                }
            })
            .collect();
        bundles.push(Bundle {
            layer,
            connections,
            branches,
            junctions: if ends.len() > 2 {
                vec![hub]
            } else {
                Vec::new()
            },
            label_position: Some(hub),
        });
    }
    Ok(Layout {
        attachment_hints: req.view.attachment_hints.clone(),
        layout_version: 2,
        unrouted_connections: Vec::new(),
        nodes,
        bundles,
        positions,
    })
}
#[wasm_bindgen]
pub fn preview(input: &str) -> Result<String, JsError> {
    let request: LayoutRequest = serde_json::from_str(input)?;
    let result = preview_layout(&request).map_err(|e| JsError::new(&e))?;
    Ok(serde_json::to_string(&result)?)
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
    preferred: &[f64],
) -> Vec<Point> {
    if bounds.is_some_and(|b| !in_bounds(start, b) || !in_bounds(end, b)) {
        return Vec::new();
    }
    let mut candidates = vec![
        simplify(vec![
            start,
            Point {
                x: end.x,
                y: start.y,
            },
            end,
        ]),
        simplify(vec![
            start,
            Point {
                x: start.x,
                y: end.y,
            },
            end,
        ]),
    ];
    for lane in preferred {
        candidates.push(simplify(vec![
            start,
            Point {
                x: *lane,
                y: start.y,
            },
            Point { x: *lane, y: end.y },
            end,
        ]));
    }
    let fast = candidates
        .into_iter()
        .filter(|path| {
            path.iter().all(|p| bounds.is_none_or(|b| in_bounds(*p, b)))
                && path
                    .windows(2)
                    .all(|p| !blocked(p[0], p[1], rects) && !track_conflict(p[0], p[1], tracks))
        })
        .min_by(|a, b| routing_cost(a, preferred).total_cmp(&routing_cost(b, preferred)));
    // Every remaining alternative has at least two bends. Return immediately
    // when a clear candidate already meets that lower bound.
    if let Some(path) = &fast {
        if routing_cost(path, preferred) <= distance(start, end) + 112. + GEOMETRY_EPS {
            return path.clone();
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
    xs.extend(preferred);
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
    // The edge test is the search's inner loop: every relaxation checks a
    // grid edge against every obstacle and track. Only obstacles whose padded
    // span covers a row (column) can block a horizontal (vertical) edge in it,
    // and only same-orientation tracks within spacing can conflict, so gather
    // those per row and per column once; each edge's verdict is the same
    // `blocked` / `track_conflict` test run on that short list, memoized
    // (an edge is reached from several direction states).
    let row_rects: Vec<Vec<Rect>> = ys
        .iter()
        .map(|&y| {
            rects
                .iter()
                .filter(|r| y > r.position.y - 8. && y < r.position.y + r.height + 8.)
                .copied()
                .collect()
        })
        .collect();
    let col_rects: Vec<Vec<Rect>> = xs
        .iter()
        .map(|&x| {
            rects
                .iter()
                .filter(|r| x > r.position.x - 8. && x < r.position.x + r.width + 8.)
                .copied()
                .collect()
        })
        .collect();
    let row_tracks: Vec<Vec<Track>> = ys
        .iter()
        .map(|&y| {
            tracks
                .iter()
                .filter(|t| t.horizontal() && (y - t.a.y).abs() < TRACK_SPACING - 1e-6)
                .copied()
                .collect()
        })
        .collect();
    let col_tracks: Vec<Vec<Track>> = xs
        .iter()
        .map(|&x| {
            tracks
                .iter()
                .filter(|t| !t.horizontal() && (x - t.a.x).abs() < TRACK_SPACING - 1e-6)
                .copied()
                .collect()
        })
        .collect();
    // 0 = not yet tested, 1 = open, 2 = closed; indexed by the edge's
    // lower-indexed cell and its axis.
    let mut edge_state = vec![0u8; w * h * 2];
    let mut edge_open = |i: usize, j: usize, axis: usize| -> bool {
        let k = i.min(j) * 2 + axis - 1;
        if edge_state[k] == 0 {
            let (a, b) = (point(i), point(j));
            let (rs, ts) = if axis == 1 {
                (&row_rects[i / w], &row_tracks[i / w])
            } else {
                (&col_rects[i % w], &col_tracks[i % w])
            };
            edge_state[k] = if blocked(a, b, rs) || track_conflict(a, b, ts) {
                2
            } else {
                1
            };
        }
        edge_state[k] == 1
    };
    let mut dist = vec![u64::MAX; w * h * 3];
    let mut prev = vec![usize::MAX; dist.len()];
    let mut heap = BinaryHeap::new();
    // A*: order by cost-so-far plus a lower bound on the rest. Every step
    // costs at least 1000x its length minus 0.5 (rounding), so Manhattan
    // distance x1000, less 0.5 per grid cell a path could visit, never
    // overestimates — the first time `end` is reached its cost is optimal.
    let slack = (w * h) as u64 / 2 + 1;
    let estimate = |i: usize| -> u64 {
        let p = point(i);
        ((((p.x - end.x).abs() + (p.y - end.y).abs()) * 1000.).floor() as u64).saturating_sub(slack)
    };
    dist[s * 3] = 0;
    heap.push(std::cmp::Reverse((estimate(s), s * 3)));
    let mut finish = None;
    while let Some(std::cmp::Reverse((f, state))) = heap.pop() {
        let cost = dist[state];
        if cost.saturating_add(estimate(state / 3)) != f {
            continue;
        }
        let i = state / 3;
        let dir = state % 3;
        if i == t {
            finish = Some(state);
            break;
        }
        let neighbors = [
            (i % w > 0).then(|| (i.wrapping_sub(1), 1)),
            (i % w + 1 < w).then_some((i + 1, 1)),
            (i >= w).then(|| (i.wrapping_sub(w), 2)),
            (i + w < w * h).then_some((i + w, 2)),
        ];
        for (j, d) in neighbors.into_iter().flatten() {
            if !edge_open(i, j, d) {
                continue;
            }
            let a = point(i);
            let b = point(j);
            let c = cost
                + ((distance(a, b) + vertical_affinity(a, b, preferred)) * 1000.).round() as u64
                + if dir != 0 && dir != d { 28000 } else { 0 };
            let next = j * 3 + d;
            if c < dist[next] {
                dist[next] = c;
                prev[next] = state;
                heap.push(std::cmp::Reverse((c + estimate(j), next)));
            }
        }
    }
    let Some(mut at) = finish else {
        return fast.unwrap_or_default();
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
    if let Some(candidate) = fast {
        if routing_cost(&candidate, preferred) < routing_cost(&simple, preferred) {
            return candidate;
        }
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
    fn all_device_ports_and_space_crossings_are_left_or_right() {
        let doc = sample();
        for collapsed in [
            BTreeSet::new(),
            BTreeSet::from(["kitchen".into(), "living".into()]),
        ] {
            let result = layout(&LayoutRequest {
                document: doc.clone(),
                view: View {
                    collapsed,
                    ..View::default()
                },
            })
            .unwrap();
            assert!(result.unrouted_connections.is_empty());
            for bundle in &result.bundles {
                for branch in &bundle.branches {
                    let n = result.nodes.iter().find(|n| n.id == branch.entity).unwrap();
                    let port = branch.points[0];
                    assert!(port.x == n.position.x || port.x == n.position.x + n.width);
                    assert!(port.y > n.position.y && port.y < n.position.y + n.height);
                    assert_eq!(port.y, branch.points[1].y);
                    let mut at = doc
                        .entities
                        .iter()
                        .find(|e| e.id == branch.entity)
                        .unwrap()
                        .parent
                        .as_deref();
                    while let Some(id) = at {
                        let n = result.nodes.iter().find(|n| n.id == id).unwrap();
                        let rect = Rect {
                            position: n.position,
                            width: n.width,
                            height: n.height,
                        };
                        if !in_bounds(*branch.points.last().unwrap(), rect) {
                            let crossing = branch
                                .points
                                .windows(2)
                                .find(|p| in_bounds(p[0], rect) && !in_bounds(p[1], rect))
                                .unwrap();
                            assert_eq!(
                                crossing[0].y, crossing[1].y,
                                "vertical boundary crossing in {}",
                                id
                            );
                            let exit = boundary_exit(&branch.points, rect).unwrap();
                            assert!(
                                exit.x == rect.position.x || exit.x == rect.position.x + rect.width
                            );
                        }
                        at = doc
                            .entities
                            .iter()
                            .find(|e| e.id == id)
                            .unwrap()
                            .parent
                            .as_deref();
                    }
                }
            }
        }
    }
    #[test]
    fn utility_connection_families_share_stable_centered_lane_pools() {
        let doc = sample();
        let expanded = layout(&LayoutRequest {
            document: doc.clone(),
            view: View::default(),
        })
        .unwrap();
        let folded = layout(&LayoutRequest {
            document: doc.clone(),
            view: View {
                layout_version: 2,
                positions: expanded.positions.clone(),
                collapsed: BTreeSet::from(["kitchen".into(), "living".into()]),
                ..View::default()
            },
        })
        .unwrap();
        let mut expected = None;
        for nodes in [&expanded.nodes, &folded.nodes] {
            for target in ["kitchen", "living"] {
                let mut router = HierarchyRouter {
                    document: &doc,
                    nodes,
                    tracks: &[],
                    preferences: BTreeMap::new(),
                    attachments: BTreeMap::new(),
                    routes: Default::default(),
                    obstacle_sets: Default::default(),
                };
                router.plan_corridors(
                    Some("ground"),
                    &[
                        ("panel".into(), Role::Source),
                        (target.into(), Role::Target),
                    ],
                );
                let lanes = router.preferred(Some("ground"));
                assert_eq!(lanes.len(), 2);
                assert!((lanes[1] - lanes[0] - TRACK_SPACING).abs() < GEOMETRY_EPS);
                let utility = router.node("utility");
                let kitchen = router.node("kitchen");
                let midpoint =
                    (utility.position.x + utility.expanded_width + kitchen.position.x) / 2.;
                assert!(((lanes[0] + lanes[1]) / 2. - midpoint).abs() < GEOMETRY_EPS);
                if let Some(previous) = &expected {
                    assert_eq!(lanes, previous);
                } else {
                    expected = Some(lanes.to_vec());
                }
            }
        }
    }
    #[test]
    fn corridor_affinity_centers_long_vertical_legs_but_keeps_direct_links_straight() {
        let path = route(
            Point { x: 0., y: 0. },
            Point { x: 200., y: 160. },
            &[],
            None,
            &[],
            &[100.],
        );
        assert_eq!(
            path,
            vec![
                Point { x: 0., y: 0. },
                Point { x: 100., y: 0. },
                Point { x: 100., y: 160. },
                Point { x: 200., y: 160. }
            ]
        );
        assert_eq!(
            route(
                Point { x: 0., y: 0. },
                Point { x: 200., y: 0. },
                &[],
                None,
                &[],
                &[100.]
            ),
            vec![Point { x: 0., y: 0. }, Point { x: 200., y: 0. }]
        );
    }
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
            preferences: BTreeMap::new(),
            attachments: BTreeMap::new(),
            routes: Default::default(),
            obstacle_sets: Default::default(),
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
            preferences: BTreeMap::new(),
            attachments: BTreeMap::new(),
            routes: Default::default(),
            obstacle_sets: Default::default(),
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
            preferences: BTreeMap::new(),
            attachments: BTreeMap::new(),
            routes: Default::default(),
            obstacle_sets: Default::default(),
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
            preferences: BTreeMap::new(),
            attachments: BTreeMap::new(),
            routes: Default::default(),
            obstacle_sets: Default::default(),
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
                        port: None,
                    },
                    Endpoint {
                        entity: "lamp".into(),
                        role: Role::Target,
                        port: None,
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
                        port: None,
                    },
                    Endpoint {
                        entity: "lamp".into(),
                        role: Role::Target,
                        port: None,
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
                preferences: BTreeMap::new(),
                attachments: BTreeMap::new(),
                routes: Default::default(),
                obstacle_sets: Default::default(),
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
    fn containers_shrink_after_a_child_returns_and_ports_resist_small_moves() {
        let doc = sample();
        let first = layout(&LayoutRequest {
            document: doc.clone(),
            view: View::default(),
        })
        .unwrap();
        let mut view = View {
            layout_version: 2,
            positions: first.positions.clone(),
            attachment_hints: first.attachment_hints.clone(),
            ..View::default()
        };
        let lamp = first.nodes.iter().find(|n| n.id == "lamp").unwrap();
        for delta in [1., 2., 1., 0., -1., 0.] {
            view = move_entity(&MoveRequest {
                dragging: false,
                document: doc.clone(),
                view,
                entity: "lamp".into(),
                position: Point {
                    x: lamp.position.x + delta,
                    y: lamp.position.y,
                },
            })
            .unwrap();
            let next = layout(&LayoutRequest {
                document: doc.clone(),
                view: view.clone(),
            })
            .unwrap();
            assert!(next.unrouted_connections.is_empty());
            for (key, hint) in &first.attachment_hints {
                if key.contains("port:lamp") {
                    assert_eq!(next.attachment_hints.get(key), Some(hint));
                }
            }
            view.attachment_hints = next.attachment_hints;
        }
        // One drag out and back: every frame starts from the drag's first
        // view, so the rooms the far position pushed return with it.
        let start = view.clone();
        let before = layout(&LayoutRequest {
            document: doc.clone(),
            view: start.clone(),
        })
        .unwrap();
        view = move_entity(&MoveRequest {
            dragging: true,
            document: doc.clone(),
            view: start.clone(),
            entity: "lamp".into(),
            position: Point {
                x: lamp.position.x + 500.,
                y: lamp.position.y + 500.,
            },
        })
        .unwrap();
        let grown = layout(&LayoutRequest {
            document: doc.clone(),
            view: view.clone(),
        })
        .unwrap();
        view = move_entity(&MoveRequest {
            dragging: false,
            document: doc.clone(),
            view: start,
            entity: "lamp".into(),
            position: lamp.position,
        })
        .unwrap();
        let restored = layout(&LayoutRequest {
            document: doc,
            view,
        })
        .unwrap();
        for id in ["kitchen", "ground", "house"] {
            if let Some(original) = before.nodes.iter().find(|n| n.id == id) {
                let large = grown.nodes.iter().find(|n| n.id == id).unwrap();
                let small = restored.nodes.iter().find(|n| n.id == id).unwrap();
                assert!(large.width > original.width || large.height > original.height);
                assert_eq!(
                    (small.width, small.height),
                    (original.width, original.height)
                );
            }
        }
    }
    /// Visible siblings keep at least `GAP` between them at every level.
    fn assert_separated(doc: &Document, result: &Layout) {
        for a in &result.nodes {
            for b in &result.nodes {
                let parent = |id: &str| {
                    doc.entities
                        .iter()
                        .find(|e| e.id == id)
                        .unwrap()
                        .parent
                        .clone()
                };
                if a.id >= b.id || parent(&a.id) != parent(&b.id) {
                    continue;
                }
                let apart = a.position.x + a.width + GAP <= b.position.x + 1e-6
                    || b.position.x + b.width + GAP <= a.position.x + 1e-6
                    || a.position.y + a.height + GAP <= b.position.y + 1e-6
                    || b.position.y + b.height + GAP <= a.position.y + 1e-6;
                assert!(apart, "{} and {} are closer than GAP", a.id, b.id);
            }
        }
    }
    fn settled(doc: &Document) -> (View, Layout) {
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
        (view, first)
    }
    fn node<'a>(l: &'a Layout, id: &str) -> &'a Node {
        l.nodes.iter().find(|n| n.id == id).unwrap()
    }
    fn drag(doc: &Document, view: &View, id: &str, to: Point, dragging: bool) -> Layout {
        let view = move_entity(&MoveRequest {
            dragging,
            document: doc.clone(),
            view: view.clone(),
            entity: id.into(),
            position: to,
        })
        .unwrap();
        preview_layout(&LayoutRequest {
            document: doc.clone(),
            view,
        })
        .unwrap()
    }
    fn port(id: &str, position: u32, poles: u32, half: Option<&str>) -> Port {
        Port {
            id: id.into(),
            label: format!("{id} label"),
            position: Some(position),
            poles: Some(poles),
            half: half.map(Into::into),
            halves: Vec::new(),
            main: false,
        }
    }
    /// The sample with a breaker schedule on its panel: 1 kitchen, 2 living,
    /// a 2-pole breaker over spaces 3 and 5, and a tandem in space 6.
    fn panel_doc() -> Document {
        let mut doc = sample();
        let panel = doc.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        panel.faceplate = Some(Faceplate::Panel);
        panel.ports = vec![
            port("1", 1, 1, None),
            port("2", 2, 1, None),
            port("3/5", 3, 2, None),
            port("6A", 6, 1, Some("A")),
            port("6B", 6, 1, Some("B")),
        ];
        for c in &mut doc.connections {
            let at = match c.id.as_str() {
                "circuit" => "1",
                "living-power" => "2",
                _ => continue,
            };
            c.endpoints[0].port = Some(at.into());
        }
        doc
    }
    #[test]
    fn ports_validate_spaces_poles_tandems_and_endpoint_names() {
        let doc = panel_doc();
        validate(&doc).unwrap();
        let with = |f: &dyn Fn(&mut Document)| {
            let mut d = panel_doc();
            f(&mut d);
            validate(&d)
        };
        // A 2-pole breaker at 3 covers space 5: nothing else may start there.
        assert!(
            with(&|d| d
                .entities
                .iter_mut()
                .find(|e| e.id == "panel")
                .unwrap()
                .ports
                .push(port("5", 5, 1, None)))
            .is_err()
        );
        // A third half, or a full breaker, in the tandem's space.
        assert!(
            with(&|d| d
                .entities
                .iter_mut()
                .find(|e| e.id == "panel")
                .unwrap()
                .ports
                .push(port("6", 6, 1, None)))
            .is_err()
        );
        assert!(
            with(&|d| d
                .entities
                .iter_mut()
                .find(|e| e.id == "panel")
                .unwrap()
                .ports
                .push(port("6C", 6, 1, Some("A"))))
            .is_err()
        );
        // Duplicate IDs, bad halves, too many poles.
        assert!(
            with(&|d| d
                .entities
                .iter_mut()
                .find(|e| e.id == "panel")
                .unwrap()
                .ports
                .push(port("1", 9, 1, None)))
            .is_err()
        );
        assert!(
            with(&|d| d
                .entities
                .iter_mut()
                .find(|e| e.id == "panel")
                .unwrap()
                .ports
                .push(port("9", 9, 1, Some("C"))))
            .is_err()
        );
        assert!(
            with(&|d| d
                .entities
                .iter_mut()
                .find(|e| e.id == "panel")
                .unwrap()
                .ports
                .push(port("9", 9, 4, None)))
            .is_err()
        );
        // An endpoint may only name one of its entity's ports.
        assert!(with(&|d| d.connections[0].endpoints[1].port = Some("1".into())).is_err());
        assert!(with(&|d| d.connections[0].endpoints[0].port = Some("99".into())).is_err());
        // On a switch, positions count one apiece: 1 (2 wide) then 3 is fine.
        assert!(
            with(&|d| {
                let p = d.entities.iter_mut().find(|e| e.id == "panel").unwrap();
                p.faceplate = Some(Faceplate::Switch);
                p.ports = vec![port("a", 1, 2, None), port("b", 3, 1, None)];
                for c in &mut d.connections {
                    for ep in &mut c.endpoints {
                        ep.port = None;
                    }
                }
            })
            .is_ok()
        );
    }
    #[test]
    fn panel_faceplate_numbers_columns_spans_poles_and_splits_tandems() {
        let doc = panel_doc();
        let panel = doc.entities.iter().find(|e| e.id == "panel").unwrap();
        let (size, face) = faceplate_layout(panel, true);
        let b = |id: &str| face.ports.iter().find(|b| b.id == id).unwrap();
        // Odd spaces down the left, even down the right, row by row.
        assert_eq!(b("1").side, Side::Left);
        assert_eq!(b("2").side, Side::Right);
        assert_eq!(b("1").y, b("2").y);
        assert!(b("1").x < b("2").x);
        // The 2-pole breaker spans rows 2 and 3 (spaces 3 and 5).
        assert_eq!(b("3/5").position, "3/5");
        assert_eq!(b("3/5").y, b("1").y + FACE_ROW);
        assert!(b("3/5").height > FACE_ROW);
        // Tandem halves share space 6, A above B.
        assert_eq!(b("6A").position, "6A");
        assert!(b("6A").y < b("6B").y && b("6B").y < b("6A").y + FACE_ROW);
        // Space 4 is empty and drawn as a slot; the grid ends at row 3.
        assert_eq!(
            face.empty
                .iter()
                .map(|e| e.position.as_str())
                .collect::<Vec<_>>(),
            ["4"]
        );
        let unassigned = face.unassigned.unwrap();
        assert_eq!(unassigned.y, FACE_HEADER + 3. * FACE_ROW);
        assert_eq!(size.height, FACE_HEADER + 4. * FACE_ROW + 8.);
    }
    fn linked(id: &str, position: u32, halves: [&str; 2]) -> Port {
        Port {
            halves: halves.map(String::from).to_vec(),
            ..port(id, position, 2, None)
        }
    }
    /// D's quad breaker and the barn apartment's linked pairs: a pole may
    /// sit in a different half of each space it covers.
    #[test]
    fn linked_breakers_take_a_half_per_space_and_mains_sit_on_top() {
        let mut doc = panel_doc();
        let panel = doc.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        panel.ports = vec![
            Port {
                main: true,
                ..port("MAIN", 1, 2, None)
            },
            linked("1A/3B", 1, ["A", "B"]),
            linked("1B/3A", 1, ["B", "A"]),
            linked("7B/9A", 7, ["B", "A"]),
            port("7A", 7, 1, Some("A")),
            port("9B", 9, 1, Some("B")),
            port("2", 2, 1, None),
        ];
        panel.ports[0].position = None;
        for c in &mut doc.connections {
            for ep in &mut c.endpoints {
                ep.port = None;
            }
        }
        validate(&doc).unwrap();
        let panel = doc.entities.iter().find(|e| e.id == "panel").unwrap();
        let (_, face) = faceplate_layout(panel, false);
        let boxes = |id: &str| face.ports.iter().filter(|b| b.id == id).collect::<Vec<_>>();
        // The main breaker spans the top, above space 1.
        let main = boxes("MAIN")[0];
        assert_eq!((main.position.as_str(), main.y), ("MAIN", FACE_HEADER));
        assert!(main.width > 280.);
        let first_row = FACE_HEADER + FACE_ROW;
        // Quad outer pair: 1A and 3B are not adjacent, so two tied segments.
        let outer = boxes("1A/3B");
        assert_eq!(outer.len(), 2);
        assert_eq!((outer[0].segment, outer[0].position.as_str()), (0, "1A/3B"));
        assert_eq!((outer[1].segment, outer[1].position.as_str()), (1, "3B"));
        assert_eq!(outer[0].y, first_row);
        // 3B: the lower half of the next row (space 3).
        assert!(outer[1].y > first_row + FACE_ROW + FACE_ROW / 4.);
        assert!(outer[1].y + outer[1].height <= first_row + 2. * FACE_ROW);
        // Inner pair 1B/3A is one box between them.
        let inner = boxes("1B/3A");
        assert_eq!(inner.len(), 1);
        assert!(inner[0].y > outer[0].y + outer[0].height);
        assert!(inner[0].y + inner[0].height < outer[1].y);
        // 7B/9A is one box from 7's lower half to 9's upper half, between 7A and 9B.
        let (b79, a7, b9) = (boxes("7B/9A")[0], boxes("7A")[0], boxes("9B")[0]);
        assert!(a7.y + a7.height < b79.y && b79.y + b79.height < b9.y);
        // Wires attach to a port's first segment.
        let n = Node {
            id: "panel".into(),
            position: Point::default(),
            children: 0,
            width: 320.,
            height: 400.,
            expanded_width: 320.,
            expanded_height: 400.,
            depth: 0,
            container: false,
            collapsed: false,
            faceplate: Some(face.clone()),
        };
        let pin = faceplate_pin(&n, Some("1A/3B")).unwrap();
        assert_eq!(pin.y, outer[0].y + outer[0].height / 2.);

        // A space can't take the same half twice, or a whole and a half.
        let mut bad = doc.clone();
        let p = bad.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        p.ports.push(port("3A", 3, 1, Some("A")));
        assert!(validate(&bad).is_err(), "3A is the inner pair's");
        let mut bad = doc.clone();
        let p = bad.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        p.ports[1].halves = vec!["A".into()];
        assert!(validate(&bad).is_err(), "one half per pole");
        let mut bad = doc.clone();
        let p = bad.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        p.ports[1].half = Some("A".into());
        assert!(validate(&bad).is_err(), "halves replaces half");
        let mut bad = doc.clone();
        let p = bad.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        p.ports[0].position = Some(11);
        assert!(validate(&bad).is_err(), "a main takes no position");
    }
    #[test]
    fn faceplate_wires_attach_at_their_ports() {
        let doc = panel_doc();
        let mut view = View::default();
        view.faceplates.insert("panel".into());
        let result = layout(&LayoutRequest {
            document: doc.clone(),
            view: view.clone(),
        })
        .unwrap();
        let n = result.nodes.iter().find(|n| n.id == "panel").unwrap();
        let face = n.faceplate.as_ref().expect("drawn as a faceplate");
        assert!(n.height > CARD_HEIGHT && n.width > CARD_WIDTH);
        assert!(result.unrouted_connections.is_empty());
        for (connection, port) in [("circuit", "1"), ("living-power", "2")] {
            let bundle = result
                .bundles
                .iter()
                .find(|b| b.connections.contains(&connection.to_string()))
                .unwrap();
            let start = bundle
                .branches
                .iter()
                .find(|b| b.entity == "panel")
                .unwrap()
                .points[0];
            let b = face.ports.iter().find(|b| b.id == port).unwrap();
            assert_eq!(start.y, n.position.y + b.y + b.height / 2., "{connection}");
            let edge = if b.side == Side::Left {
                n.position.x
            } else {
                n.position.x + n.width
            };
            assert_eq!(start.x, edge, "{connection} leaves its column's edge");
        }
        // Without the faceplate shown, the panel is an ordinary card.
        let plain = layout(&LayoutRequest {
            document: doc,
            view: View::default(),
        })
        .unwrap();
        let n = plain.nodes.iter().find(|n| n.id == "panel").unwrap();
        assert!(n.faceplate.is_none() && n.height == CARD_HEIGHT);
    }
    #[test]
    fn settling_a_new_faceplate_pushes_its_neighbours_aside() {
        let doc = panel_doc();
        let (mut view, _) = settled(&doc);
        view.faceplates.insert("panel".into());
        // Taller than the card it replaced, the faceplate now overlaps the
        // things below it; a move to where it already is settles that.
        let grown = preview_layout(&LayoutRequest {
            document: doc.clone(),
            view: view.clone(),
        })
        .unwrap();
        let at = node(&grown, "panel").position;
        let view = move_entity(&MoveRequest {
            dragging: false,
            document: doc.clone(),
            view,
            entity: "panel".into(),
            position: at,
        })
        .unwrap();
        let after = preview_layout(&LayoutRequest {
            document: doc.clone(),
            view,
        })
        .unwrap();
        assert_eq!(node(&after, "panel").position, at);
        assert_separated(&doc, &after);
    }
    #[test]
    fn different_ports_are_different_wires() {
        let mut doc = panel_doc();
        // A second kitchen connection on breaker 2 to the same endpoints.
        let mut twin = doc.connections[0].clone();
        twin.id = "circuit-2".into();
        twin.endpoints[0].port = Some("2".into());
        doc.connections.push(twin);
        let mut view = View::default();
        let merged = layout(&LayoutRequest {
            document: doc.clone(),
            view: view.clone(),
        })
        .unwrap();
        assert!(merged.bundles.iter().any(|b| b.connections.len() == 2));
        view.faceplates.insert("panel".into());
        let split = layout(&LayoutRequest {
            document: doc,
            view,
        })
        .unwrap();
        assert!(
            split
                .bundles
                .iter()
                .all(|b| !(b.connections.contains(&"circuit".to_string())
                    && b.connections.contains(&"circuit-2".to_string())))
        );
    }
    #[test]
    fn focus_follows_hops_and_direction_and_keeps_the_full_view() {
        let doc = sample();
        let (view, _) = settled(&doc);
        let shown = |focus: Focus| {
            let mut v = view.clone();
            v.focus = Some(focus.clone());
            let l = layout(&LayoutRequest {
                document: doc.clone(),
                view: v.clone(),
            })
            .unwrap();
            assert_eq!(
                l.positions, view.positions,
                "focus never rewrites the full view"
            );
            (focus_set(&doc, &v, &focus), l)
        };
        // One hop from the lamp: its circuit's panel and sensor, and HA.
        let (set, l) = shown(Focus {
            entities: vec!["lamp".into()],
            hops: Some(1),
            direction: Direction::Both,
        });
        assert_eq!(
            set,
            ["lamp", "panel", "sensor", "ha"]
                .map(String::from)
                .into_iter()
                .collect()
        );
        // Shown: those plus the spaces around them, nothing else.
        let ids: BTreeSet<_> = l.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains("kitchen") && ids.contains("utility") && ids.contains("house"));
        assert!(!ids.contains("tv") && !ids.contains("office") && !ids.contains("desk"));
        assert!(
            l.bundles
                .iter()
                .all(|b| b.branches.iter().all(|br| set.contains(&br.entity)))
        );
        // Upstream from the TV: what feeds it (panel, router), not its peers.
        let (set, _) = shown(Focus {
            entities: vec!["tv".into()],
            hops: None,
            direction: Direction::Upstream,
        });
        assert_eq!(
            set,
            ["tv", "panel", "router"]
                .map(String::from)
                .into_iter()
                .collect()
        );
        // Downstream from the router, everything it reaches.
        let (set, _) = shown(Focus {
            entities: vec!["router".into()],
            hops: None,
            direction: Direction::Downstream,
        });
        assert_eq!(
            set,
            ["router", "tv", "desk", "ha", "sensor", "speaker", "lamp"]
                .map(String::from)
                .into_iter()
                .collect()
        );
        // A space brings its contents.
        let (set, _) = shown(Focus {
            entities: vec!["kitchen".into()],
            hops: Some(0),
            direction: Direction::Both,
        });
        assert_eq!(
            set,
            ["kitchen", "lamp", "sensor", "tap"]
                .map(String::from)
                .into_iter()
                .collect()
        );
    }
    #[test]
    fn dragging_onto_a_sibling_swaps_their_places() {
        let doc = sample();
        let (view, first) = settled(&doc);
        let (lamp, sensor) = (node(&first, "lamp"), node(&first, "sensor"));
        let to = Point {
            x: sensor.position.x + 10.,
            y: sensor.position.y + 10.,
        };
        for dragging in [true, false] {
            let after = drag(&doc, &view, "lamp", to, dragging);
            assert_eq!(node(&after, "lamp").position, to);
            assert_eq!(node(&after, "sensor").position, lamp.position);
            assert_separated(&doc, &after);
        }
    }
    #[test]
    fn a_box_moved_onto_holds_mid_drag_and_yields_on_drop() {
        let doc = sample();
        let (view, first) = settled(&doc);
        let sensor = node(&first, "sensor");
        // Cover a sliver of the sensor: below the swap threshold.
        let to = Point {
            x: sensor.position.x,
            y: sensor.position.y - CARD_HEIGHT + 12.,
        };
        let held = drag(&doc, &view, "lamp", to, true);
        assert_eq!(node(&held, "sensor").position, sensor.position);
        let dropped = drag(&doc, &view, "lamp", to, false);
        assert_eq!(node(&dropped, "lamp").position, to);
        assert_ne!(node(&dropped, "sensor").position, sensor.position);
        assert_separated(&doc, &dropped);
    }
    #[test]
    fn pushes_cascade_and_spring_back_within_a_drag() {
        let doc = sample();
        let (view, first) = settled(&doc);
        let kitchen = node(&first, "kitchen");
        // Drop the lamp below the kitchen's contents: the kitchen grows into
        // its neighbours, which make room, and so on up the hierarchy.
        let far = Point {
            x: kitchen.position.x + 40.,
            y: kitchen.position.y + kitchen.height + 300.,
        };
        let pushed = drag(&doc, &view, "lamp", far, true);
        assert_separated(&doc, &pushed);
        assert!(node(&pushed, "kitchen").height > kitchen.height);
        let back = drag(&doc, &view, "lamp", node(&first, "lamp").position, true);
        for n in &first.nodes {
            assert_eq!(node(&back, &n.id).position, n.position, "{}", n.id);
            assert_eq!(
                (node(&back, &n.id).width, node(&back, &n.id).height),
                (n.width, n.height)
            );
        }
    }
    #[test]
    fn preview_draws_every_bundle_without_routing() {
        let doc = sample();
        let (view, first) = settled(&doc);
        let preview = preview_layout(&LayoutRequest {
            document: doc,
            view,
        })
        .unwrap();
        assert_eq!(preview.bundles.len(), first.bundles.len());
        assert!(preview.unrouted_connections.is_empty());
        for b in &preview.bundles {
            for branch in &b.branches {
                assert!(branch.points.len() >= 2);
                for w in branch.points.windows(2) {
                    assert!(w[0].x == w[1].x || w[0].y == w[1].y);
                }
            }
        }
    }
    #[test]
    fn moving_a_space_translates_its_subtree_and_expands_parent() {
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
            dragging: false,
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
        // The kitchen's subtree moves rigidly; its rooms' neighbours only ever
        // move to keep their distance, never into it.
        for id in ["kitchen", "lamp", "sensor", "tap"] {
            let before = first.nodes.iter().find(|n| n.id == id).unwrap();
            let after = result.nodes.iter().find(|n| n.id == id).unwrap();
            assert_eq!(
                after.position,
                Point {
                    x: before.position.x + 20.,
                    y: before.position.y + 20.
                }
            );
        }
        assert_separated(&doc, &result);
        assert_eq!(moved.positions["lamp"], view.positions["lamp"]);
        let clamped = move_entity(&MoveRequest {
            dragging: false,
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
        let expanded = layout(&LayoutRequest {
            document: doc,
            view: clamped,
        })
        .unwrap();
        let lamp = expanded.nodes.iter().find(|n| n.id == "lamp").unwrap();
        assert_eq!(lamp.position, Point { x: -999., y: -999. });
        for node in &result.nodes {
            if node.id == "sensor" || node.id == "tap" {
                assert_eq!(
                    expanded
                        .nodes
                        .iter()
                        .find(|n| n.id == node.id)
                        .unwrap()
                        .position,
                    node.position
                );
            }
        }
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
            &[],
        );
        assert!(path.len() >= 4);
        for p in path.windows(2) {
            assert!(p[0].x == p[1].x || p[0].y == p[1].y);
            assert!(!blocked(p[0], p[1], &rects));
        }
    }
}
