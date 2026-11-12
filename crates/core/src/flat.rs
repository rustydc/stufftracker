//! Flat layout: the diagram without the area hierarchy.
//!
//! With areas off (the default), every thing is its own card on one plane;
//! a space appears only where a connection ends on it. Things nobody has
//! placed yet are arranged by flow (what feeds what, left to right), and
//! wires are routed by a cost search over a sparse orthogonal grid, then
//! spread so parallel runs and the pins on a card's side never overlap.
use super::*;
use std::cmp::Ordering;

/// Kind given to a space drawn as a plain card, so nothing treats it as a
/// container. (The viewer shows the entity's real kind.)
const AREA_CARD: &str = "area";

/// The flat document and view: no parents, spaces only where connected,
/// positions from `view.flat_positions`, new things arranged by flow.
pub(crate) fn projection(doc: &Document, view: &View) -> (Document, View) {
    let shown_connections: Vec<&Connection> = doc
        .connections
        .iter()
        .filter(|c| !view.hidden_layers.contains(&c.layer))
        .collect();
    let connected_space = |id: &str| {
        shown_connections
            .iter()
            .any(|c| c.endpoints.iter().any(|ep| ep.entity == id))
    };
    let keep: BTreeSet<String> = doc
        .entities
        .iter()
        .filter(|e| e.kind != "space" || connected_space(&e.id))
        .map(|e| e.id.clone())
        .collect();
    let area_of = |id: &str| -> String {
        let mut names = Vec::new();
        let mut at = doc
            .entities
            .iter()
            .find(|e| e.id == id)
            .and_then(|e| e.parent.clone());
        while let Some(p) = at {
            let Some(e) = doc.entities.iter().find(|e| e.id == p) else {
                break;
            };
            if names.len() > 20 {
                break;
            }
            names.push(e.name.clone());
            at = e.parent.clone();
        }
        names.reverse();
        names.join(" / ")
    };
    let entities: Vec<Entity> = doc
        .entities
        .iter()
        .filter(|e| keep.contains(&e.id))
        .map(|e| {
            let mut e = e.clone();
            e.parent = None;
            if e.kind == "space" {
                e.kind = AREA_CARD.into();
            }
            e
        })
        .collect();
    let connections: Vec<Connection> = doc
        .connections
        .iter()
        .filter_map(|c| {
            let endpoints: Vec<_> = c
                .endpoints
                .iter()
                .filter(|ep| keep.contains(&ep.entity))
                .cloned()
                .collect();
            let distinct: BTreeSet<_> = endpoints.iter().map(|ep| &ep.entity).collect();
            (distinct.len() >= 2).then(|| Connection {
                endpoints,
                ..c.clone()
            })
        })
        .collect();
    let projected = Document {
        entities,
        connections,
        layers: doc.layers.clone(),
    };
    let mut flat_view = View {
        layout_version: 2,
        positions: BTreeMap::new(),
        collapsed: BTreeSet::new(),
        hidden_layers: view.hidden_layers.clone(),
        attachment_hints: BTreeMap::new(),
        faceplates: view.faceplates.clone(),
        focus: None,
        areas: true,
        flat_positions: BTreeMap::new(),
    };
    let faces = faceplate_layouts(&projected, &flat_view);
    let sizes: BTreeMap<String, Size> = projected
        .entities
        .iter()
        .map(|e| {
            let size = faces.get(&e.id).map_or(
                Size {
                    width: CARD_WIDTH,
                    height: CARD_HEIGHT,
                },
                |(s, _)| *s,
            );
            (e.id.clone(), size)
        })
        .collect();
    let mut positions: BTreeMap<String, Point> = view
        .flat_positions
        .iter()
        .filter(|(id, _)| keep.contains(*id))
        .map(|(id, p)| (id.clone(), *p))
        .collect();
    let missing: Vec<String> = projected
        .entities
        .iter()
        .filter(|e| !positions.contains_key(&e.id))
        .map(|e| e.id.clone())
        .collect();
    if !missing.is_empty() {
        let areas: BTreeMap<String, String> =
            missing.iter().map(|id| (id.clone(), area_of(id))).collect();
        // Where a wire meets a faceplate: its port's row (or the
        // unassigned row); other things meet wires at their middle.
        let attach = |id: &str, port: Option<&str>| -> Option<(f64, i8)> {
            let (_, f) = faces.get(id)?;
            let b = match port {
                Some(p) => f.ports.iter().find(|b| b.id == p && b.segment == 0),
                None => f.unassigned.as_ref(),
            }?;
            let side = match b.side {
                Side::Left => -1,
                Side::Right => 1,
                Side::Either => 0,
            };
            Some((b.y + b.height / 2., side))
        };
        let placed = arrange(&projected, &missing, &positions, &sizes, &areas, &attach);
        positions.extend(placed);
    }
    flat_view.positions = positions;
    (projected, flat_view)
}

const COLUMN_GAP: f64 = 120.;
const ROW_GAP: f64 = 36.;

/// Place `ids` by flow: each connected group in columns (what feeds on the
/// left, what it feeds to the right), ordered to cut crossings; unconnected
/// things in a grid below, grouped by the area they're in.
fn arrange(
    doc: &Document,
    ids: &[String],
    placed: &BTreeMap<String, Point>,
    sizes: &BTreeMap<String, Size>,
    areas: &BTreeMap<String, String>,
    attach: &dyn Fn(&str, Option<&str>) -> Option<(f64, i8)>,
) -> BTreeMap<String, Point> {
    let index: BTreeMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let n = ids.len();
    // Edges by flow, each marked when it leaves its source by a faceplate's
    // left column: what it feeds then goes to the left.
    let mut out_edges: Vec<BTreeMap<usize, bool>> = vec![BTreeMap::new(); n];
    let mut neighbours: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    // Per thing: (neighbour, where the wire meets this thing, where it meets
    // the neighbour), as offsets from each top (a faceplate port's row).
    let mut adj: Vec<Vec<(usize, Option<f64>, Option<f64>)>> = vec![Vec::new(); n];
    for c in &doc.connections {
        let at: Vec<(usize, &Role)> = c
            .endpoints
            .iter()
            .filter_map(|ep| index.get(ep.entity.as_str()).map(|i| (*i, &ep.role)))
            .collect();
        let port_of = |i: usize| {
            c.endpoints
                .iter()
                .find(|ep| ep.entity == ids[i])
                .and_then(|ep| attach(&ids[i], ep.port.as_deref()))
        };
        let sources: Vec<usize> = at
            .iter()
            .filter(|(_, r)| **r == Role::Source)
            .map(|(i, _)| *i)
            .collect();
        let targets: Vec<usize> = at
            .iter()
            .filter(|(_, r)| **r == Role::Target)
            .map(|(i, _)| *i)
            .collect();
        let mut pairs = Vec::new();
        if !sources.is_empty() && !targets.is_empty() {
            for s in &sources {
                for t in &targets {
                    pairs.push((*s, *t));
                }
            }
        } else if let Some((first, _)) = at.first() {
            for (other, _) in &at[1..] {
                pairs.push((*first, *other));
            }
        }
        for (a, b) in pairs {
            if a != b {
                let left = port_of(a).is_some_and(|(_, side)| side < 0);
                let edge = out_edges[a].entry(b).or_insert(left);
                *edge = *edge && left;
                neighbours[a].insert(b);
                neighbours[b].insert(a);
                let y = |p: Option<(f64, i8)>| p.map(|(y, _)| y);
                adj[a].push((b, y(port_of(a)), y(port_of(b))));
                adj[b].push((a, y(port_of(b)), y(port_of(a))));
            }
        }
    }
    // Break cycles: drop edges that close a loop in a depth-first walk.
    let mut state = vec![0u8; n];
    let mut dag: Vec<BTreeMap<usize, bool>> = vec![BTreeMap::new(); n];
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, Vec<usize>)> =
            vec![(start, out_edges[start].keys().copied().collect())];
        state[start] = 1;
        while let Some((v, rest)) = stack.last_mut() {
            let v = *v;
            if let Some(w) = rest.pop() {
                match state[w] {
                    0 => {
                        dag[v].insert(w, out_edges[v][&w]);
                        state[w] = 1;
                        let next = out_edges[w].keys().copied().collect();
                        stack.push((w, next));
                    }
                    2 => {
                        dag[v].insert(w, out_edges[v][&w]);
                    }
                    _ => {} // back edge: a cycle; leave it out of the ranking
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    // Ranks (columns) by longest path. Flow runs rightwards, except that
    // a wire leaving a left-column breaker runs leftwards, and so does
    // everything downstream of it: a panel feeds out to both its sides.
    let mut indegree = vec![0usize; n];
    let mut preds: Vec<Vec<(usize, bool)>> = vec![Vec::new(); n];
    for v in 0..n {
        for (w, left) in &dag[v] {
            indegree[*w] += 1;
            preds[*w].push((v, *left));
        }
    }
    let mut rank = vec![0i64; n];
    let mut flow = vec![1i64; n];
    let step = |v: usize, left: bool, flow: &Vec<i64>| if left { -flow[v] } else { flow[v] };
    let mut queue: Vec<usize> = (0..n).filter(|v| indegree[*v] == 0).collect();
    let mut order = Vec::new();
    while let Some(v) = queue.pop() {
        order.push(v);
        if !preds[v].is_empty() {
            let rightwards: Vec<i64> = preds[v]
                .iter()
                .filter(|(s, left)| step(*s, *left, &flow) > 0)
                .map(|(s, _)| rank[*s] + 1)
                .collect();
            if let Some(r) = rightwards.iter().max() {
                rank[v] = *r;
                flow[v] = 1;
            } else {
                rank[v] = preds[v].iter().map(|(s, _)| rank[*s] - 1).min().unwrap();
                flow[v] = -1;
            }
        }
        for w in dag[v].keys().copied().collect::<Vec<_>>() {
            indegree[w] -= 1;
            if indegree[w] == 0 {
                queue.push(w);
            }
        }
    }
    // A pure source sits right next to what it feeds, on the side it feeds.
    for v in order.iter().rev() {
        if !preds[*v].is_empty() || dag[*v].is_empty() {
            continue;
        }
        let steps: Vec<i64> = dag[*v]
            .iter()
            .map(|(_, left)| step(*v, *left, &flow))
            .collect();
        if steps.iter().all(|d| *d > 0) {
            rank[*v] = dag[*v].keys().map(|w| rank[*w]).min().unwrap() - 1;
        } else if steps.iter().all(|d| *d < 0) {
            rank[*v] = dag[*v].keys().map(|w| rank[*w]).max().unwrap() + 1;
        }
    }
    // Weakly connected groups; things with no connection go in the grid.
    let mut group = vec![usize::MAX; n];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if group[start] != usize::MAX || neighbours[start].is_empty() {
            continue;
        }
        let g = groups.len();
        let mut members = vec![start];
        group[start] = g;
        let mut i = 0;
        while i < members.len() {
            for w in neighbours[members[i]].clone() {
                if group[w] == usize::MAX {
                    group[w] = g;
                    members.push(w);
                }
            }
            i += 1;
        }
        groups.push(members);
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let size = |i: usize| sizes[&ids[i]];
    let sort_name = |i: usize| {
        let e = doc.entities.iter().find(|e| e.id == ids[i]).unwrap();
        (
            areas.get(&ids[i]).cloned().unwrap_or_default(),
            e.name.clone(),
        )
    };
    let (origin_x, mut y) = if placed.is_empty() {
        (40., 40.)
    } else {
        let left = placed.values().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let bottom = placed
            .iter()
            .map(|(id, p)| p.y + sizes.get(id).map_or(CARD_HEIGHT, |s| s.height))
            .fold(f64::NEG_INFINITY, f64::max);
        (left, bottom + 120.)
    };
    let mut positions = BTreeMap::new();
    let mut widest = 0f64;
    for members in &groups {
        let base = members.iter().map(|v| rank[*v]).min().unwrap();
        let depth = members.iter().map(|v| rank[*v] - base).max().unwrap() as usize + 1;
        let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
        let mut sorted = members.clone();
        sorted.sort_by_key(|v| sort_name(*v));
        for v in sorted {
            layers[(rank[v] - base) as usize].push(v);
        }
        let offset = |v: usize, off: Option<f64>| off.unwrap_or(size(v).height / 2.);
        // Where along a neighbour a wire meets it, as a fraction of a slot:
        // a panel's breakers order the things they feed.
        let frac = |w: usize, off: Option<f64>| {
            (offset(w, off) / size(w).height).clamp(0., 1.) * 0.8 - 0.4
        };
        let in_layer =
            |w: usize, r: usize| group[w] == group[members[0]] && rank[w] - base == r as i64;
        // Barycentre sweeps: each column ordered by where its neighbours sit.
        let mut slot = vec![0f64; n];
        let reslot = |layers: &Vec<Vec<usize>>, slot: &mut Vec<f64>| {
            for layer in layers {
                for (k, v) in layer.iter().enumerate() {
                    slot[*v] = k as f64;
                }
            }
        };
        reslot(&layers, &mut slot);
        for sweep in 0..8 {
            let range: Vec<usize> = if sweep % 2 == 0 {
                (1..depth).collect()
            } else {
                (0..depth.saturating_sub(1)).rev().collect()
            };
            for r in range {
                let near = if sweep % 2 == 0 { r - 1 } else { r + 1 };
                let mut keyed: Vec<(f64, usize)> = layers[r]
                    .iter()
                    .map(|v| {
                        let ns: Vec<f64> = adj[*v]
                            .iter()
                            .filter(|(w, _, _)| in_layer(*w, near))
                            .map(|(w, _, wo)| slot[*w] + frac(*w, *wo))
                            .collect();
                        let key = if ns.is_empty() {
                            slot[*v]
                        } else {
                            ns.iter().sum::<f64>() / ns.len() as f64
                        };
                        (key, *v)
                    })
                    .collect();
                keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
                layers[r] = keyed.into_iter().map(|(_, v)| v).collect();
                reslot(&layers, &mut slot);
            }
        }
        // Then swap neighbours in a column while that removes crossings
        // with the columns either side.
        let crossings = |u: usize, v: usize, r: usize, slot: &Vec<f64>| -> usize {
            let mut count = 0;
            for near in [r.wrapping_sub(1), r + 1] {
                if near >= depth {
                    continue;
                }
                for (a, _, ao) in adj[u].iter().filter(|(w, _, _)| in_layer(*w, near)) {
                    for (b, _, bo) in adj[v].iter().filter(|(w, _, _)| in_layer(*w, near)) {
                        if slot[*a] + frac(*a, *ao) > slot[*b] + frac(*b, *bo) + 1e-9 {
                            count += 1;
                        }
                    }
                }
            }
            count
        };
        for _ in 0..20 {
            let mut improved = false;
            for r in 0..depth {
                for k in 0..layers[r].len().saturating_sub(1) {
                    let (u, v) = (layers[r][k], layers[r][k + 1]);
                    if crossings(v, u, r, &slot) < crossings(u, v, r, &slot) {
                        layers[r].swap(k, k + 1);
                        slot[u] = (k + 1) as f64;
                        slot[v] = k as f64;
                        improved = true;
                    }
                }
            }
            if !improved {
                break;
            }
        }
        // Heights: start stacked and centred, then line each thing up with
        // the median of where its wires meet its neighbours, keeping each
        // column's order and spacing (an exact least-squares fit).
        let heights: Vec<f64> = layers
            .iter()
            .map(|l| {
                l.iter().map(|v| size(*v).height).sum::<f64>()
                    + ROW_GAP * l.len().saturating_sub(1) as f64
            })
            .collect();
        let tallest = heights.iter().copied().fold(0., f64::max);
        let mut top = vec![0f64; n];
        for (r, layer) in layers.iter().enumerate() {
            let mut ly = (tallest - heights[r]) / 2.;
            for v in layer {
                top[*v] = ly;
                ly += size(*v).height + ROW_GAP;
            }
        }
        let passes: Vec<(usize, Vec<usize>)> = {
            let mut p = Vec::new();
            for it in 0..6 {
                for r in 1..depth {
                    p.push((r, vec![r - 1]));
                }
                for r in (0..depth.saturating_sub(1)).rev() {
                    p.push((r, vec![r + 1]));
                }
                if it >= 4 {
                    for r in 0..depth {
                        p.push((r, vec![r.wrapping_sub(1), r + 1]));
                    }
                }
            }
            p
        };
        for (r, nears) in passes {
            let layer = &layers[r];
            let desired: Vec<f64> = layer
                .iter()
                .map(|v| {
                    let mut at: Vec<f64> = adj[*v]
                        .iter()
                        .filter(|(w, _, _)| {
                            nears
                                .iter()
                                .any(|near| *near < depth && in_layer(*w, *near))
                        })
                        .map(|(w, own, wo)| top[*w] + offset(*w, *wo) - offset(*v, *own))
                        .collect();
                    if at.is_empty() {
                        return top[*v];
                    }
                    at.sort_by(f64::total_cmp);
                    let m = at.len() / 2;
                    if at.len() % 2 == 1 {
                        at[m]
                    } else {
                        (at[m - 1] + at[m]) / 2.
                    }
                })
                .collect();
            let hs: Vec<f64> = layer.iter().map(|v| size(*v).height).collect();
            for (v, y) in layer.iter().zip(fit_in_order(&desired, &hs, ROW_GAP)) {
                top[*v] = y;
            }
        }
        let lowest = members
            .iter()
            .map(|v| top[*v])
            .fold(f64::INFINITY, f64::min);
        let deepest = members
            .iter()
            .map(|v| top[*v] + size(*v).height)
            .fold(f64::NEG_INFINITY, f64::max);
        let mut x = origin_x;
        for layer in &layers {
            let width = layer.iter().map(|v| size(*v).width).fold(0., f64::max);
            for v in layer {
                positions.insert(
                    ids[*v].clone(),
                    Point {
                        x,
                        y: y + top[*v] - lowest,
                    },
                );
            }
            // Wider gaps where more wires cross between the columns.
            let crossing = layer
                .iter()
                .map(|v| {
                    neighbours[*v]
                        .iter()
                        .filter(|w| rank[**w] > rank[*v])
                        .count()
                })
                .sum::<usize>();
            x += width + COLUMN_GAP + (crossing as f64 * 6.).min(160.);
        }
        let tallest = deepest - lowest;
        widest = widest.max(x - origin_x);
        y += tallest + 110.;
    }
    // The unconnected, in reading order by area then name.
    let mut lone: Vec<usize> = (0..n).filter(|v| neighbours[*v].is_empty()).collect();
    lone.sort_by_key(|v| sort_name(*v));
    let row_width = widest.max(1240.);
    let mut x = origin_x;
    let mut row_height = 0f64;
    for v in lone {
        let s = size(v);
        if x > origin_x && x + s.width > origin_x + row_width {
            x = origin_x;
            y += row_height + ROW_GAP;
            row_height = 0.;
        }
        positions.insert(ids[v].clone(), Point { x, y });
        x += s.width + 40.;
        row_height = row_height.max(s.height);
    }
    positions
}

/// Tops for a column, in order, each at least its height plus `gap` below
/// the one above, as close as possible (least squares) to `desired`: pool
/// adjacent violators over the tops less their minimum offsets.
fn fit_in_order(desired: &[f64], heights: &[f64], gap: f64) -> Vec<f64> {
    let mut offset = vec![0.; desired.len()];
    for i in 1..desired.len() {
        offset[i] = offset[i - 1] + heights[i - 1] + gap;
    }
    let mut blocks: Vec<(f64, usize)> = Vec::new();
    for (d, o) in desired.iter().zip(&offset) {
        blocks.push((d - o, 1));
        while blocks.len() > 1 {
            let (s2, c2) = blocks[blocks.len() - 1];
            let (s1, c1) = blocks[blocks.len() - 2];
            if s1 / c1 as f64 > s2 / c2 as f64 {
                blocks.pop();
                *blocks.last_mut().unwrap() = (s1 + s2, c1 + c2);
            } else {
                break;
            }
        }
    }
    let mut out = Vec::with_capacity(desired.len());
    for (sum, count) in blocks {
        for _ in 0..count {
            out.push(sum / count as f64 + offset[out.len()]);
        }
    }
    out
}

// --- Routing ---------------------------------------------------------------

/// Clearance kept between a wire and any box it passes.
const MARGIN: f64 = 14.;
const BEND: f64 = 36.;
const CROSS: f64 = 28.;
/// Per unit of length run alongside another wire: cheap, since spreading
/// separates them afterwards, but enough to prefer an empty channel.
const SHARE: f64 = 0.35;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    Left,
    Right,
    Up,
    Down,
}
impl Dir {
    const ALL: [Dir; 4] = [Dir::Left, Dir::Right, Dir::Up, Dir::Down];
    fn index(self) -> usize {
        self as usize
    }
    fn reverse(self) -> Dir {
        match self {
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
        }
    }
    fn horizontal(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }
    fn step(self) -> (f64, f64) {
        match self {
            Dir::Left => (-1., 0.),
            Dir::Right => (1., 0.),
            Dir::Up => (0., -1.),
            Dir::Down => (0., 1.),
        }
    }
}

/// Where a wire may leave a box: a point on its edge and the way out.
#[derive(Clone, Copy, Debug)]
struct Pin {
    at: Point,
    dir: Dir,
    /// A faceplate port's row: it may not slide along the side.
    fixed: bool,
    node: usize,
}
impl Pin {
    fn stub(&self) -> Point {
        let (dx, dy) = self.dir.step();
        Point {
            x: self.at.x + dx * MARGIN,
            y: self.at.y + dy * MARGIN,
        }
    }
}

fn pins(nodes: &[Node], i: usize, port: Option<&str>) -> Vec<Pin> {
    let n = &nodes[i];
    let (x0, y0, x1, y1) = (
        n.position.x,
        n.position.y,
        n.position.x + n.width,
        n.position.y + n.height,
    );
    if let Some(rel) = faceplate_pin(n, port) {
        let y = y0 + rel.y;
        let pin = |at, dir| Pin {
            at,
            dir,
            fixed: true,
            node: i,
        };
        return if rel.x < 0. {
            vec![pin(Point { x: x0, y }, Dir::Left)]
        } else if rel.x > 0. {
            vec![pin(Point { x: x1, y }, Dir::Right)]
        } else {
            vec![
                pin(Point { x: x0, y }, Dir::Left),
                pin(Point { x: x1, y }, Dir::Right),
                pin(
                    Point {
                        x: (x0 + x1) / 2.,
                        y: y0,
                    },
                    Dir::Up,
                ),
            ]
        };
    }
    let (cx, cy) = ((x0 + x1) / 2., (y0 + y1) / 2.);
    let pin = |at, dir| Pin {
        at,
        dir,
        fixed: false,
        node: i,
    };
    vec![
        pin(Point { x: x0, y: cy }, Dir::Left),
        pin(Point { x: x1, y: cy }, Dir::Right),
        pin(Point { x: cx, y: y0 }, Dir::Up),
        pin(Point { x: cx, y: y1 }, Dir::Down),
    ]
}

/// The routing grid: lines along every box's clearance edge, every pin, and
/// the middle of every wide gap; nodes inside a box's clearance are blocked.
struct Grid {
    xs: Vec<f64>,
    ys: Vec<f64>,
    blocked: Vec<bool>,
    /// Edge (i, j) → (i + 1, j) runs through a box.
    block_h: Vec<bool>,
    /// Edge (i, j) → (i, j + 1) runs through a box.
    block_v: Vec<bool>,
}
fn coords(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    let mut out = Vec::with_capacity(v.len() * 2);
    for (k, c) in v.iter().enumerate() {
        out.push(*c);
        if let Some(next) = v.get(k + 1)
            && next - c > 48.
        {
            out.push((c + next) / 2.);
        }
    }
    out
}
impl Grid {
    fn new(rects: &[(f64, f64, f64, f64)], extra_x: &[f64], extra_y: &[f64]) -> Grid {
        let mut xs: Vec<f64> = extra_x.to_vec();
        let mut ys: Vec<f64> = extra_y.to_vec();
        let (mut lx, mut ly, mut hx, mut hy) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for &(x0, y0, x1, y1) in rects {
            xs.extend([x0, x1]);
            ys.extend([y0, y1]);
            lx = lx.min(x0);
            ly = ly.min(y0);
            hx = hx.max(x1);
            hy = hy.max(y1);
        }
        if lx.is_finite() {
            xs.extend([lx - 60., hx + 60.]);
            ys.extend([ly - 60., hy + 60.]);
        }
        let xs = coords(xs);
        let ys = coords(ys);
        let (nx, ny) = (xs.len(), ys.len());
        let mut grid = Grid {
            blocked: vec![false; nx * ny],
            block_h: vec![false; nx * ny],
            block_v: vec![false; nx * ny],
            xs,
            ys,
        };
        // Strictly inside, by index ranges: a box costs its own area only.
        let inside = |v: &[f64], lo: f64, hi: f64| {
            let a = v.partition_point(|c| *c <= lo + 1e-6);
            let b = v.partition_point(|c| *c < hi - 1e-6);
            a..b
        };
        // Edges whose open span meets the open box span.
        let spans = |v: &[f64], lo: f64, hi: f64| {
            let a = v.partition_point(|c| *c < lo + 1e-6).saturating_sub(1);
            let b = v.partition_point(|c| *c < hi - 1e-6);
            a..b.min(v.len().saturating_sub(1))
        };
        for &(x0, y0, x1, y1) in rects {
            for j in inside(&grid.ys, y0, y1) {
                for i in inside(&grid.xs, x0, x1) {
                    grid.blocked[j * nx + i] = true;
                }
                for i in spans(&grid.xs, x0, x1) {
                    if grid.xs[i] < x1 - 1e-6 && grid.xs[i + 1] > x0 + 1e-6 {
                        grid.block_h[j * nx + i] = true;
                    }
                }
            }
            for i in inside(&grid.xs, x0, x1) {
                for j in spans(&grid.ys, y0, y1) {
                    if grid.ys[j] < y1 - 1e-6 && grid.ys[j + 1] > y0 + 1e-6 {
                        grid.block_v[j * nx + i] = true;
                    }
                }
            }
        }
        grid
    }
    fn nx(&self) -> usize {
        self.xs.len()
    }
    fn find(&self, p: Point) -> Option<usize> {
        let i = self.xs.iter().position(|x| (x - p.x).abs() < 0.5)?;
        let j = self.ys.iter().position(|y| (y - p.y).abs() < 0.5)?;
        Some(j * self.nx() + i)
    }
    fn point(&self, k: usize) -> Point {
        Point {
            x: self.xs[k % self.nx()],
            y: self.ys[k / self.nx()],
        }
    }
    /// The next node from `k` in `dir`, if the way there is open.
    fn next(&self, k: usize, dir: Dir) -> Option<usize> {
        let nx = self.nx();
        let (i, j) = (k % nx, k / nx);
        let next = match dir {
            Dir::Left if i > 0 && !self.block_h[k - 1] => k - 1,
            Dir::Right if i + 1 < nx && !self.block_h[k] => k + 1,
            Dir::Up if j > 0 && !self.block_v[k - nx] => k - nx,
            Dir::Down if j + 1 < self.ys.len() && !self.block_v[k] => k + nx,
            _ => return None,
        };
        (!self.blocked[next]).then_some(next)
    }
}

/// How busy each grid edge is with other bundles' wires.
struct Usage {
    h: Vec<u16>,
    v: Vec<u16>,
}
impl Usage {
    fn mark(&mut self, grid: &Grid, path: &[usize], add: bool) {
        let nx = grid.nx();
        for w in path.windows(2) {
            let (a, b) = (w[0].min(w[1]), w[0].max(w[1]));
            let slot = if b == a + 1 {
                &mut self.h[a]
            } else if b == a + nx {
                &mut self.v[a]
            } else {
                continue;
            };
            *slot = if add {
                slot.saturating_add(1)
            } else {
                slot.saturating_sub(1)
            };
        }
    }
}

/// What a search may end on.
enum Goal<'a> {
    /// Pins: arrive at the stub, then turn into the box.
    Pins(&'a [(usize, Dir)]),
    /// Any node of a bundle's existing wires (a branch joining the tree).
    Tree(&'a BTreeSet<usize>, (f64, f64, f64, f64)),
}

struct Search {
    g: Vec<f32>,
    prev: Vec<u32>,
    touched: Vec<usize>,
}
impl Search {
    fn new(states: usize) -> Search {
        Search {
            g: vec![f32::INFINITY; states],
            prev: vec![u32::MAX; states],
            touched: Vec::new(),
        }
    }
    fn reset(&mut self) {
        for s in self.touched.drain(..) {
            self.g[s] = f32::INFINITY;
            self.prev[s] = u32::MAX;
        }
    }
    /// Cheapest path (grid nodes) from any start to the goal: length, plus
    /// bends, crossings of other wires and runs alongside them.
    fn run(
        &mut self,
        grid: &Grid,
        usage: &Usage,
        starts: &[(usize, Dir, f64)],
        goal: &Goal,
    ) -> Option<(Vec<usize>, f64)> {
        self.reset();
        let nx = grid.nx();
        let h = |k: usize| -> f64 {
            let p = grid.point(k);
            match goal {
                Goal::Pins(pins) => pins
                    .iter()
                    .map(|(g, _)| {
                        let q = grid.point(*g);
                        (p.x - q.x).abs() + (p.y - q.y).abs()
                    })
                    .fold(f64::INFINITY, f64::min),
                Goal::Tree(_, (x0, y0, x1, y1)) => {
                    (x0 - p.x).max(0.).max(p.x - x1) + (y0 - p.y).max(0.).max(p.y - y1)
                }
            }
        };
        #[derive(PartialEq)]
        struct Item(f64, usize);
        impl Eq for Item {}
        impl PartialOrd for Item {
            fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for Item {
            fn cmp(&self, o: &Self) -> Ordering {
                o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
            }
        }
        let mut heap = BinaryHeap::new();
        for &(k, d, cost) in starts {
            let s = k * 4 + d.index();
            if (cost as f32) < self.g[s] {
                self.g[s] = cost as f32;
                self.touched.push(s);
                heap.push(Item(cost + h(k), s));
            }
        }
        let mut best: Option<(f64, usize)> = None;
        let mut pops = 0usize;
        while let Some(Item(f, s)) = heap.pop() {
            if best.is_some_and(|(b, _)| f >= b) {
                break;
            }
            pops += 1;
            if pops > 400_000 {
                break;
            }
            let (k, d) = (s / 4, Dir::ALL[s % 4]);
            let g = self.g[s] as f64;
            if f - h(k) > g + 1e-3 {
                continue; // stale
            }
            match goal {
                Goal::Pins(pins) => {
                    for (gk, inward) in pins.iter() {
                        if *gk == k {
                            let total = g + MARGIN + if d == *inward { 0. } else { BEND };
                            if best.is_none_or(|(b, _)| total < b) {
                                best = Some((total, s));
                            }
                        }
                    }
                }
                Goal::Tree(nodes, _) => {
                    if nodes.contains(&k) && best.is_none_or(|(b, _)| g < b) {
                        best = Some((g, s));
                    }
                }
            }
            for nd in Dir::ALL {
                if nd == d.reverse() {
                    continue;
                }
                let Some(next) = grid.next(k, nd) else {
                    continue;
                };
                let a = grid.point(k);
                let b = grid.point(next);
                let len = (a.x - b.x).abs() + (a.y - b.y).abs();
                let mut cost = len;
                if nd != d {
                    cost += BEND;
                }
                let (lo, hi) = (k.min(next), k.max(next));
                let shared = if nd.horizontal() {
                    usage.h[lo]
                } else {
                    usage.v[lo]
                };
                let _ = hi;
                cost += SHARE * len * shared.min(3) as f64;
                // Going straight through a node another wire crosses.
                if nd == d {
                    let j = k / nx;
                    let i = k % nx;
                    let across = if nd.horizontal() {
                        j > 0 && j + 1 < grid.ys.len() && usage.v[k - nx] > 0 && usage.v[k] > 0
                    } else {
                        i > 0 && i + 1 < nx && usage.h[k - 1] > 0 && usage.h[k] > 0
                    };
                    if across {
                        cost += CROSS;
                    }
                }
                let ns = next * 4 + nd.index();
                let ng = g + cost;
                if (ng as f32) < self.g[ns] {
                    if self.g[ns].is_infinite() {
                        self.touched.push(ns);
                    }
                    self.g[ns] = ng as f32;
                    self.prev[ns] = s as u32;
                    heap.push(Item(ng + h(next), ns));
                }
            }
        }
        let (cost, end) = best?;
        let mut path = vec![end / 4];
        let mut s = end;
        while self.prev[s] != u32::MAX {
            s = self.prev[s] as usize;
            if path.last() != Some(&(s / 4)) {
                path.push(s / 4);
            }
        }
        path.reverse();
        Some((path, cost))
    }
}

/// One bundle's routed tree: per endpoint, its pin and the grid path from
/// its stub to the far end (the other endpoint, or where it joins the tree).
struct Routed {
    paths: Vec<(Pin, Vec<usize>, Option<Pin>)>,
    cost: f64,
}

struct Plan<'a> {
    nodes: &'a [Node],
    grid: Grid,
}
impl Plan<'_> {
    fn starts(&self, pins: &[Pin]) -> Vec<(usize, Dir, f64)> {
        pins.iter()
            .filter_map(|p| self.grid.find(p.stub()).map(|k| (k, p.dir, MARGIN)))
            .collect()
    }
    fn route_bundle(
        &self,
        search: &mut Search,
        usage: &Usage,
        ends: &[Vec<Pin>],
    ) -> Option<Routed> {
        let centre = |pins: &[Pin]| {
            let n = &self.nodes[pins[0].node];
            Point {
                x: n.position.x + n.width / 2.,
                y: n.position.y + n.height / 2.,
            }
        };
        // Start from the two ends farthest apart; the rest join nearest first.
        let mut pair = (0, 1);
        let mut far = -1.;
        for a in 0..ends.len() {
            for b in a + 1..ends.len() {
                let (p, q) = (centre(&ends[a]), centre(&ends[b]));
                let d = (p.x - q.x).abs() + (p.y - q.y).abs();
                if d > far {
                    far = d;
                    pair = (a, b);
                }
            }
        }
        let goal_pins: Vec<(usize, Dir)> = ends[pair.1]
            .iter()
            .filter_map(|p| self.grid.find(p.stub()).map(|k| (k, p.dir.reverse())))
            .collect();
        let (path, mut cost) = search.run(
            &self.grid,
            usage,
            &self.starts(&ends[pair.0]),
            &Goal::Pins(&goal_pins),
        )?;
        let start_pin = *ends[pair.0]
            .iter()
            .find(|p| self.grid.find(p.stub()) == Some(path[0]))?;
        let end_pin = *ends[pair.1]
            .iter()
            .find(|p| self.grid.find(p.stub()) == path.last().copied())?;
        let mut routed = Routed {
            paths: vec![(start_pin, path.clone(), Some(end_pin))],
            cost: 0.,
        };
        let mut tree: BTreeSet<usize> = BTreeSet::new();
        let add_tree = |tree: &mut BTreeSet<usize>, path: &[usize]| {
            for w in path.windows(2) {
                // Every node along the run, so a branch may join anywhere.
                let (a, b) = (w[0].min(w[1]), w[0].max(w[1]));
                let step = if b - a < self.grid.nx() {
                    1
                } else {
                    self.grid.nx()
                };
                let mut k = a;
                while k <= b {
                    tree.insert(k);
                    k += step;
                }
            }
            tree.extend(path.iter().copied());
        };
        add_tree(&mut tree, &path);
        let mut rest: Vec<usize> = (0..ends.len())
            .filter(|i| *i != pair.0 && *i != pair.1)
            .collect();
        while !rest.is_empty() {
            let bbox = tree.iter().fold(
                (
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ),
                |(a, b, c, d), k| {
                    let p = self.grid.point(*k);
                    (a.min(p.x), b.min(p.y), c.max(p.x), d.max(p.y))
                },
            );
            let dist = |i: usize| {
                let p = centre(&ends[i]);
                (bbox.0 - p.x).max(0.).max(p.x - bbox.2) + (bbox.1 - p.y).max(0.).max(p.y - bbox.3)
            };
            rest.sort_by(|a, b| dist(*a).total_cmp(&dist(*b)));
            let next = rest.remove(0);
            let (path, c) = search.run(
                &self.grid,
                usage,
                &self.starts(&ends[next]),
                &Goal::Tree(&tree, bbox),
            )?;
            let pin = *ends[next]
                .iter()
                .find(|p| self.grid.find(p.stub()) == Some(path[0]))?;
            cost += c;
            add_tree(&mut tree, &path);
            routed.paths.push((pin, path, None));
        }
        routed.cost = cost;
        Some(routed)
    }
}

/// A routed polyline for one endpoint, in world coordinates.
struct Line {
    bundle: usize,
    entity: String,
    role: Role,
    points: Vec<Point>,
    /// The first point is a pin: may it slide, and along what span?
    pin: Option<(bool, f64, f64)>,
    /// The last point is a pin too (the far end of a two-ended run).
    end_pin: Option<(bool, f64, f64)>,
}

/// Route every bundle on the flat plane, then spread parallel runs.
pub(crate) fn route(doc: &Document, view: &View, nodes: &[Node]) -> Vec<Bundle> {
    let index: BTreeMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let groups: Vec<(BundleKey, Vec<String>)> = bundle_groups(doc, view).into_iter().collect();
    // Pins for every endpoint of every bundle.
    let ends: Vec<Vec<(String, Role, Vec<Pin>)>> = groups
        .iter()
        .map(|((_, eps, ports), _)| {
            eps.iter()
                .filter_map(|(id, role)| {
                    let i = *index.get(id.as_str())?;
                    let port = ports.iter().find(|(e, _)| e == id).map(|(_, p)| p.as_str());
                    Some((id.clone(), role.clone(), pins(nodes, i, port)))
                })
                .collect()
        })
        .collect();
    let rects: Vec<(f64, f64, f64, f64)> = nodes
        .iter()
        .map(|n| {
            (
                n.position.x - MARGIN,
                n.position.y - MARGIN,
                n.position.x + n.width + MARGIN,
                n.position.y + n.height + MARGIN,
            )
        })
        .collect();
    let mut extra_x = Vec::new();
    let mut extra_y = Vec::new();
    for p in ends.iter().flatten().flat_map(|(_, _, pins)| pins) {
        let s = p.stub();
        extra_x.push(s.x);
        extra_y.push(s.y);
    }
    let plan = Plan {
        nodes,
        grid: Grid::new(&rects, &extra_x, &extra_y),
    };
    let cells = plan.grid.xs.len() * plan.grid.ys.len();
    let mut usage = Usage {
        h: vec![0; cells],
        v: vec![0; cells],
    };
    let mut search = Search::new(cells * 4);
    // Short runs first take the direct ways; then each bundle is ripped up
    // and rerouted once, knowing where everything else went.
    let span = |b: usize| {
        let pts: Vec<Point> = ends[b].iter().map(|(_, _, p)| p[0].at).collect();
        let xs = pts.iter().map(|p| p.x);
        let ys = pts.iter().map(|p| p.y);
        (xs.clone().fold(f64::NEG_INFINITY, f64::max) - xs.fold(f64::INFINITY, f64::min))
            + (ys.clone().fold(f64::NEG_INFINITY, f64::max) - ys.fold(f64::INFINITY, f64::min))
    };
    let mut order: Vec<usize> = (0..groups.len()).filter(|b| ends[*b].len() >= 2).collect();
    order.sort_by(|a, b| span(*a).total_cmp(&span(*b)).then(a.cmp(b)));
    let mut routed: Vec<Option<Routed>> = (0..groups.len()).map(|_| None).collect();
    for pass in 0..2 {
        for &b in &order {
            if let Some(r) = routed[b].take() {
                for (_, path, _) in &r.paths {
                    usage.mark(&plan.grid, path, false);
                }
                if pass > 0 && r.cost <= 0. {
                    routed[b] = Some(r);
                    continue;
                }
            }
            let pins: Vec<Vec<Pin>> = ends[b].iter().map(|(_, _, p)| p.clone()).collect();
            let r = plan.route_bundle(&mut search, &usage, &pins);
            if let Some(r) = &r {
                for (_, path, _) in &r.paths {
                    usage.mark(&plan.grid, path, true);
                }
            }
            routed[b] = r;
        }
    }
    // Polylines per endpoint: a two-ended run is split at its middle so each
    // end has its own branch (the viewer draws a target's arrow at its start).
    let mut lines: Vec<Line> = Vec::new();
    let span_of = |p: &Pin| -> (bool, f64, f64) {
        let n = &nodes[p.node];
        if p.dir.horizontal() {
            (!p.fixed, n.position.y + 10., n.position.y + n.height - 10.)
        } else {
            (!p.fixed, n.position.x + 12., n.position.x + n.width - 12.)
        }
    };
    let entity_of = |b: usize, pin: &Pin| -> (String, Role) {
        let id = &nodes[pin.node].id;
        let (_, role, _) = ends[b].iter().find(|(e, _, _)| e == id).unwrap();
        (id.clone(), role.clone())
    };
    for (b, r) in routed.iter().enumerate() {
        let Some(r) = r else { continue };
        for (k, (pin, path, end_pin)) in r.paths.iter().enumerate() {
            let mut pts = vec![pin.at];
            pts.extend(path.iter().map(|g| plan.grid.point(*g)));
            if let Some(end) = end_pin {
                pts.push(end.at);
            }
            let pts = simplify(pts);
            let (entity, role) = entity_of(b, pin);
            if k == 0
                && let Some(end) = end_pin
            {
                let (a, z) = split_middle(&pts);
                let (end_entity, end_role) = entity_of(b, end);
                lines.push(Line {
                    bundle: b,
                    entity,
                    role,
                    points: a,
                    pin: Some(span_of(pin)),
                    end_pin: None,
                });
                lines.push(Line {
                    bundle: b,
                    entity: end_entity,
                    role: end_role,
                    points: z,
                    pin: Some(span_of(end)),
                    end_pin: None,
                });
            } else {
                lines.push(Line {
                    bundle: b,
                    entity,
                    role,
                    points: pts,
                    pin: Some(span_of(pin)),
                    end_pin: None,
                });
            }
        }
    }
    spread(&mut lines, nodes);
    // Assemble bundles in the original group order.
    let mut bundles = Vec::new();
    for (b, ((layer, eps, _), connections)) in groups.into_iter().enumerate() {
        let mut branches: Vec<Branch> = lines
            .iter()
            .filter(|l| l.bundle == b)
            .map(|l| Branch {
                entity: l.entity.clone(),
                role: l.role.clone(),
                points: simplify(l.points.clone()),
            })
            .collect();
        if branches.is_empty() {
            branches = eps
                .iter()
                .map(|(id, role)| Branch {
                    entity: id.clone(),
                    role: role.clone(),
                    points: Vec::new(),
                })
                .collect();
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
    bundles
}

/// Split a polyline at half its length: the first half from the start, the
/// second reversed (from the far end), meeting at the same point.
fn split_middle(pts: &[Point]) -> (Vec<Point>, Vec<Point>) {
    let total: f64 = pts.windows(2).map(|w| distance(w[0], w[1])).sum();
    let mut left = total / 2.;
    let mut a = vec![pts[0]];
    for (k, w) in pts.windows(2).enumerate() {
        let d = distance(w[0], w[1]);
        if left <= d {
            let t = if d > 0. { left / d } else { 0. };
            let mid = Point {
                x: w[0].x + (w[1].x - w[0].x) * t,
                y: w[0].y + (w[1].y - w[0].y) * t,
            };
            a.push(mid);
            let mut z: Vec<Point> = pts[k + 1..].iter().rev().copied().collect();
            z.push(mid);
            return (simplify(a), simplify(z));
        }
        left -= d;
        a.push(w[1]);
    }
    (a, vec![*pts.last().unwrap(), *pts.last().unwrap()])
}

/// A straight run of a line, for spreading.
#[derive(Clone, Copy)]
struct Run {
    line: usize,
    /// Index of its first point in the line.
    k: usize,
    at: f64,
    lo: f64,
    hi: f64,
}

/// Spread parallel runs: wires of different bundles sharing a grid line are
/// ordered so they don't cross where they part, and set `TRACK_SPACING`
/// apart, within the free space between boxes. A run that starts at a card's
/// pin moves the pin along the side with it, spreading wires on a side.
fn spread(lines: &mut [Line], nodes: &[Node]) {
    for horizontal in [true, false] {
        let mut runs: Vec<Run> = Vec::new();
        for (li, l) in lines.iter().enumerate() {
            for (k, w) in l.points.windows(2).enumerate() {
                let h = (w[0].y - w[1].y).abs() < 1e-6 && (w[0].x - w[1].x).abs() > 1e-6;
                let v = (w[0].x - w[1].x).abs() < 1e-6 && (w[0].y - w[1].y).abs() > 1e-6;
                if (horizontal && h) || (!horizontal && v) {
                    let (at, a, b) = if horizontal {
                        (w[0].y, w[0].x, w[1].x)
                    } else {
                        (w[0].x, w[0].y, w[1].y)
                    };
                    runs.push(Run {
                        line: li,
                        k,
                        at,
                        lo: a.min(b),
                        hi: a.max(b),
                    });
                }
            }
        }
        // Channels: runs on the same line whose spans overlap.
        runs.sort_by(|a, b| a.at.total_cmp(&b.at).then(a.lo.total_cmp(&b.lo)));
        let mut start = 0;
        while start < runs.len() {
            let mut end = start + 1;
            let mut reach = runs[start].hi;
            while end < runs.len()
                && (runs[end].at - runs[start].at).abs() < 0.5
                && runs[end].lo < reach - 1e-6
            {
                reach = reach.max(runs[end].hi);
                end += 1;
            }
            let channel: Vec<Run> = runs[start..end].to_vec();
            start = end;
            // Tracks: one per bundle (a bundle's own runs share a trunk).
            let mut tracks: BTreeMap<usize, Vec<Run>> = BTreeMap::new();
            for r in &channel {
                tracks.entry(lines[r.line].bundle).or_default().push(*r);
            }
            if tracks.len() < 2 {
                continue;
            }
            let at = channel[0].at;
            let lo = channel.iter().map(|r| r.lo).fold(f64::INFINITY, f64::min);
            let hi = channel
                .iter()
                .map(|r| r.hi)
                .fold(f64::NEG_INFINITY, f64::max);
            // Free space either side, between the boxes the channel passes.
            let (mut room_lo, mut room_hi) = (f64::NEG_INFINITY, f64::INFINITY);
            for n in nodes {
                let (a0, a1, c0, c1) = if horizontal {
                    (
                        n.position.x,
                        n.position.x + n.width,
                        n.position.y,
                        n.position.y + n.height,
                    )
                } else {
                    (
                        n.position.y,
                        n.position.y + n.height,
                        n.position.x,
                        n.position.x + n.width,
                    )
                };
                if a1 <= lo + 1e-6 || a0 >= hi - 1e-6 {
                    continue;
                }
                if c1 <= at + 1e-6 {
                    room_lo = room_lo.max(c1 + 5.);
                } else if c0 >= at - 1e-6 {
                    room_hi = room_hi.min(c0 - 5.);
                }
            }
            // Order: how each track turns where it leaves the channel.
            let width = (hi - lo).max(1.);
            let mut keyed: Vec<(f64, usize, Vec<Run>)> = tracks
                .into_iter()
                .map(|(b, rs)| {
                    let mut key = 0.;
                    for r in &rs {
                        let pts = &lines[r.line].points;
                        for (end, neighbour) in
                            [(r.k, r.k.checked_sub(1)), (r.k + 1, Some(r.k + 2))]
                        {
                            let Some(nb) = neighbour.filter(|nb| *nb < pts.len()) else {
                                continue;
                            };
                            let p = pts[end];
                            let q = pts[nb];
                            let (along, off) = if horizontal {
                                (p.x, q.y - p.y)
                            } else {
                                (p.y, q.x - p.x)
                            };
                            if off.abs() < 1e-6 {
                                continue;
                            }
                            let side = off.signum();
                            let pos = (along - lo) / width;
                            // At the high end a wire turning early should be on
                            // the side it turns to; at the low end, a late one.
                            let is_high = (along - r.hi).abs() < 1e-6;
                            let weight = if is_high { 1. + (1. - pos) } else { 1. + pos };
                            key += side * weight;
                        }
                    }
                    (key, b, rs)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let count = keyed.len();
            let mut spacing = TRACK_SPACING;
            if room_lo.is_finite() && room_hi.is_finite() {
                let room = room_hi - room_lo;
                spacing = spacing.min(room / count as f64).max(2.);
            }
            let total = spacing * (count - 1) as f64;
            let mut first = at - total / 2.;
            if room_lo.is_finite() {
                first = first.max(room_lo);
            }
            if room_hi.is_finite() {
                first = first.min(room_hi - total);
            }
            // A fixed pin (a faceplate port) holds its track where it is.
            for (rank, (_, _, rs)) in keyed.iter().enumerate() {
                if rs.iter().any(|r| touches_fixed_pin(&lines[r.line], r.k)) {
                    first = at - spacing * rank as f64;
                    break;
                }
            }
            for (rank, (_, b, rs)) in keyed.iter().enumerate() {
                let mut to = first + spacing * rank as f64;
                // A sliding pin stays on its side.
                for r in rs {
                    if let Some((_, a, z)) = pin_span(&lines[r.line], r.k) {
                        to = to.clamp(a, z);
                    }
                    if touches_fixed_pin(&lines[r.line], r.k) {
                        to = at;
                    }
                }
                if (to - at).abs() < 1e-6 {
                    continue;
                }
                // The whole straight stretch of this bundle on the line: its
                // runs here plus any of its runs they touch end to end.
                let mut spans: Vec<(f64, f64)> = rs.iter().map(|r| (r.lo, r.hi)).collect();
                let same: Vec<(f64, f64)> = runs
                    .iter()
                    .filter(|r| lines[r.line].bundle == *b && (r.at - at).abs() < 1e-6)
                    .map(|r| (r.lo, r.hi))
                    .collect();
                loop {
                    let before = spans.len();
                    for s in &same {
                        if !spans.contains(s)
                            && spans.iter().any(|t| s.0 <= t.1 + 1e-6 && t.0 <= s.1 + 1e-6)
                        {
                            spans.push(*s);
                        }
                    }
                    if spans.len() == before {
                        break;
                    }
                }
                for l in lines.iter_mut().filter(|l| l.bundle == *b) {
                    for p in l.points.iter_mut() {
                        let (c, along) = if horizontal {
                            (&mut p.y, p.x)
                        } else {
                            (&mut p.x, p.y)
                        };
                        if (*c - at).abs() < 1e-6
                            && spans
                                .iter()
                                .any(|(lo, hi)| along >= lo - 1e-6 && along <= hi + 1e-6)
                        {
                            *c = to;
                        }
                    }
                }
            }
        }
    }
}
fn pin_span(l: &Line, k: usize) -> Option<(bool, f64, f64)> {
    if k == 0 {
        l.pin.filter(|p| p.0)
    } else if k + 2 == l.points.len() {
        l.end_pin.filter(|p| p.0)
    } else {
        None
    }
}
fn touches_fixed_pin(l: &Line, k: usize) -> bool {
    (k == 0 && l.pin.is_some_and(|p| !p.0))
        || (k + 2 == l.points.len() && l.end_pin.is_some_and(|p| !p.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_view() -> View {
        View {
            areas: false,
            ..View::default()
        }
    }
    /// Every routed wire: right angles only, starts on its own box's edge,
    /// never through a box, and no two bundles share a stretch of line.
    pub(crate) fn check(l: &Layout) -> (usize, usize, f64) {
        let rect = |n: &Node| {
            (
                n.position.x,
                n.position.y,
                n.position.x + n.width,
                n.position.y + n.height,
            )
        };
        let mut bends = 0;
        let mut length = 0.;
        let mut runs: Vec<(usize, bool, f64, f64, f64)> = Vec::new();
        for (bi, b) in l.bundles.iter().enumerate() {
            for br in &b.branches {
                assert!(br.points.len() >= 2, "{} unrouted", br.entity);
                let n = l.nodes.iter().find(|n| n.id == br.entity).unwrap();
                let (x0, y0, x1, y1) = rect(n);
                let p = br.points[0];
                let on_edge = ((p.x - x0).abs() < 1e-6 || (p.x - x1).abs() < 1e-6)
                    && p.y >= y0 - 1e-6
                    && p.y <= y1 + 1e-6
                    || ((p.y - y0).abs() < 1e-6 || (p.y - y1).abs() < 1e-6)
                        && p.x >= x0 - 1e-6
                        && p.x <= x1 + 1e-6;
                assert!(on_edge, "{} starts off its edge at {p:?}", br.entity);
                bends += br.points.len().saturating_sub(2);
                for w in br.points.windows(2) {
                    let h = (w[0].y - w[1].y).abs() < 1e-6;
                    let v = (w[0].x - w[1].x).abs() < 1e-6;
                    assert!(h || v, "diagonal in {}", br.entity);
                    length += distance(w[0], w[1]);
                    let (at, a, z) = if h {
                        (w[0].y, w[0].x.min(w[1].x), w[0].x.max(w[1].x))
                    } else {
                        (w[0].x, w[0].y.min(w[1].y), w[0].y.max(w[1].y))
                    };
                    runs.push((bi, h, at, a, z));
                    for m in &l.nodes {
                        let (mx0, my0, mx1, my1) = rect(m);
                        let through = if h {
                            at > my0 + 1e-6 && at < my1 - 1e-6 && a < mx1 - 1e-6 && z > mx0 + 1e-6
                        } else {
                            at > mx0 + 1e-6 && at < mx1 - 1e-6 && a < my1 - 1e-6 && z > my0 + 1e-6
                        };
                        assert!(!through, "{} runs through {}", br.entity, m.id);
                    }
                }
            }
        }
        let mut shared = 0;
        for (i, r) in runs.iter().enumerate() {
            for s in &runs[i + 1..] {
                if r.0 != s.0
                    && r.1 == s.1
                    && (r.2 - s.2).abs() < 1e-6
                    && r.3 < s.4 - 1e-6
                    && s.3 < r.4 - 1e-6
                {
                    shared += 1;
                }
            }
        }
        (bends, shared, length)
    }

    #[test]
    fn flat_layout_routes_every_wire_cleanly() {
        let doc = sample();
        let l = layout(&LayoutRequest {
            document: doc.clone(),
            view: flat_view(),
        })
        .unwrap();
        // No containers: every node is a card.
        assert!(l.nodes.iter().all(|n| !n.container));
        assert!(
            l.unrouted_connections.is_empty(),
            "{:?}",
            l.unrouted_connections
        );
        let (_, shared, _) = check(&l);
        assert_eq!(shared, 0, "parallel runs are spread apart");
        // The arrangement is reported for the flat view, not the nested one.
        let flat = l.flat_positions.clone().unwrap();
        assert!(!flat.is_empty());
        assert!(l.positions.is_empty());
        // Unconnected spaces are not drawn; connected things are.
        let shown: BTreeSet<_> = l.nodes.iter().map(|n| n.id.as_str()).collect();
        for c in &doc.connections {
            for ep in &c.endpoints {
                assert!(shown.contains(ep.entity.as_str()));
            }
        }
        // Laid out again from its own arrangement, it holds still.
        let again = layout(&LayoutRequest {
            document: doc.clone(),
            view: View {
                flat_positions: flat.clone(),
                ..flat_view()
            },
        })
        .unwrap();
        assert_eq!(again.flat_positions.unwrap(), flat);
    }

    #[test]
    fn wires_leave_by_the_side_facing_where_they_go() {
        // Two cards, one straight above the other: the wire runs top to bottom.
        let mut doc = sample();
        doc.connections.retain(|c| c.endpoints.len() == 2);
        let c = doc.connections[0].clone();
        let (a, b) = (c.endpoints[0].entity.clone(), c.endpoints[1].entity.clone());
        doc.connections = vec![c];
        let mut flat_positions = BTreeMap::new();
        flat_positions.insert(a.clone(), Point { x: 100., y: 100. });
        flat_positions.insert(b.clone(), Point { x: 100., y: 400. });
        for (k, e) in doc
            .entities
            .iter()
            .filter(|e| e.id != a && e.id != b)
            .enumerate()
        {
            flat_positions.insert(
                e.id.clone(),
                Point {
                    x: 1000. + k as f64 * 300.,
                    y: 1000.,
                },
            );
        }
        let l = layout(&LayoutRequest {
            document: doc,
            view: View {
                flat_positions,
                ..flat_view()
            },
        })
        .unwrap();
        let br = &l.bundles[0].branches;
        let start = |id: &str| br.iter().find(|b| b.entity == id).unwrap().points[0];
        assert_eq!(
            start(&a).y,
            100. + CARD_HEIGHT,
            "leaves the bottom of the upper card"
        );
        assert_eq!(start(&b).y, 400., "enters the top of the lower card");
        let (bends, _, _) = check(&l);
        assert_eq!(bends, 0, "a straight drop");
    }

    #[test]
    fn moving_in_flat_mode_keeps_the_nested_arrangement() {
        let doc = sample();
        let first = layout(&LayoutRequest {
            document: doc.clone(),
            view: flat_view(),
        })
        .unwrap();
        let view = View {
            flat_positions: first.flat_positions.clone().unwrap(),
            ..flat_view()
        };
        let id = first.nodes[0].id.clone();
        let moved = move_entity(&MoveRequest {
            document: doc,
            view: view.clone(),
            entity: id.clone(),
            position: Point { x: 2000., y: 2000. },
            dragging: false,
        })
        .unwrap();
        assert_eq!(moved.flat_positions[&id], Point { x: 2000., y: 2000. });
        assert_eq!(moved.positions, view.positions);
    }

    #[test]
    fn columns_fit_their_tops_in_order() {
        // Free to sit where wanted.
        assert_eq!(fit_in_order(&[0., 200.], &[50., 50.], 10.), vec![0., 200.]);
        // Wanting the same spot: split the difference, keep the spacing.
        assert_eq!(
            fit_in_order(&[100., 100.], &[50., 50.], 10.),
            vec![70., 130.]
        );
    }

    fn chain(n: usize) -> Document {
        let mut doc = sample();
        let ids: Vec<String> = doc
            .entities
            .iter()
            .filter(|e| e.kind != "space")
            .take(n)
            .map(|e| e.id.clone())
            .collect();
        let layer = doc.layers[0].id.clone();
        doc.connections = ids
            .windows(2)
            .enumerate()
            .map(|(k, w)| Connection {
                id: format!("c{k}"),
                name: format!("c{k}"),
                layer: layer.clone(),
                endpoints: vec![
                    Endpoint {
                        entity: w[0].clone(),
                        role: Role::Source,
                        port: None,
                    },
                    Endpoint {
                        entity: w[1].clone(),
                        role: Role::Target,
                        port: None,
                    },
                ],
                properties: Vec::new(),
            })
            .collect();
        doc
    }

    #[test]
    fn a_chain_lines_up_straight() {
        let doc = chain(4);
        let l = layout(&LayoutRequest {
            document: doc.clone(),
            view: flat_view(),
        })
        .unwrap();
        let y = |id: &str| l.nodes.iter().find(|n| n.id == id).unwrap().position.y;
        let chained: Vec<&str> = doc
            .connections
            .iter()
            .flat_map(|c| c.endpoints.iter().map(|e| e.entity.as_str()))
            .collect();
        for id in &chained {
            assert_eq!(y(id), y(chained[0]), "{id} is level with the chain");
        }
        let (bends, _, _) = check(&l);
        assert_eq!(bends, 0, "straight wires");
    }

    #[test]
    fn a_fan_out_does_not_cross_itself() {
        // One source feeding four, which each feed one more: the second
        // column's order follows the first, so no wire crosses another.
        let mut doc = sample();
        let ids: Vec<String> = doc
            .entities
            .iter()
            .filter(|e| e.kind != "space")
            .take(9)
            .map(|e| e.id.clone())
            .collect();
        let layer = doc.layers[0].id.clone();
        let edge = |k: usize, a: &str, b: &str| Connection {
            id: format!("e{k}"),
            name: format!("e{k}"),
            layer: layer.clone(),
            endpoints: vec![
                Endpoint {
                    entity: a.into(),
                    role: Role::Source,
                    port: None,
                },
                Endpoint {
                    entity: b.into(),
                    role: Role::Target,
                    port: None,
                },
            ],
            properties: Vec::new(),
        };
        let mut cs = Vec::new();
        for k in 0..4 {
            cs.push(edge(k, &ids[0], &ids[1 + k]));
            // Feed the next column in reverse name order, to give it work.
            cs.push(edge(10 + k, &ids[1 + k], &ids[8 - k]));
        }
        doc.connections = cs;
        let l = layout(&LayoutRequest {
            document: doc,
            view: flat_view(),
        })
        .unwrap();
        let y = |id: &str| l.nodes.iter().find(|n| n.id == id).unwrap().position.y;
        for k in 0..3 {
            let (a, b) = (y(&ids[1 + k]), y(&ids[2 + k]));
            let (c, d) = (y(&ids[8 - k]), y(&ids[7 - k]));
            assert_eq!(a < b, c < d, "column orders agree");
        }
    }

    fn port(id: &str, at: u32) -> Port {
        serde_json::from_value(serde_json::json!({ "id": id, "position": at })).unwrap()
    }

    #[test]
    fn empty_panel_rows_fold_into_a_strip() {
        let mut doc = sample();
        let panel = doc.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        panel.faceplate = Some(Faceplate::Panel);
        panel.ports = vec![port("1", 1), port("30", 30)];
        for c in &mut doc.connections {
            for ep in &mut c.endpoints {
                ep.port = None;
            }
        }
        let panel = doc.entities.iter().find(|e| e.id == "panel").unwrap();
        let (size, face) = faceplate_layout(panel, false);
        // Rows 2..14 (spaces 3–28) hold nothing: one strip instead.
        assert_eq!(face.gaps.len(), 1);
        assert_eq!(face.gaps[0].position, "3–28");
        let gap = &face.gaps[0];
        let thirty = face.ports.iter().find(|b| b.id == "30").unwrap();
        assert!(
            thirty.y > gap.y && thirty.y < gap.y + 40.,
            "30 sits just below the strip"
        );
        assert!(
            size.height < FACE_HEADER + 6. * FACE_ROW,
            "compact: {}",
            size.height
        );
        // Space 2 (beside 1) and 29 (beside 30) remain as empty slots.
        let empties: Vec<&str> = face.empty.iter().map(|b| b.position.as_str()).collect();
        assert_eq!(empties, ["2", "29"]);
    }

    #[test]
    fn a_left_breaker_feeds_to_the_left() {
        let mut doc = sample();
        let others: Vec<String> = doc
            .entities
            .iter()
            .filter(|e| e.kind != "space" && e.id != "panel")
            .take(3)
            .map(|e| e.id.clone())
            .collect();
        let panel = doc.entities.iter_mut().find(|e| e.id == "panel").unwrap();
        panel.faceplate = Some(Faceplate::Panel);
        panel.ports = vec![port("1", 1), port("2", 2)];
        let layer = doc.layers[0].id.clone();
        let edge = |id: &str, a: &str, port: Option<&str>, b: &str| Connection {
            id: id.into(),
            name: id.into(),
            layer: layer.clone(),
            endpoints: vec![
                Endpoint {
                    entity: a.into(),
                    role: Role::Source,
                    port: port.map(str::to_string),
                },
                Endpoint {
                    entity: b.into(),
                    role: Role::Target,
                    port: None,
                },
            ],
            properties: Vec::new(),
        };
        doc.connections = vec![
            edge("left", "panel", Some("1"), &others[0]),
            edge("further", &others[0], None, &others[1]),
            edge("right", "panel", Some("2"), &others[2]),
        ];
        let mut view = flat_view();
        view.faceplates.insert("panel".into());
        let l = layout(&LayoutRequest {
            document: doc,
            view,
        })
        .unwrap();
        let x = |id: &str| l.nodes.iter().find(|n| n.id == id).unwrap().position.x;
        assert!(
            x(&others[0]) < x("panel"),
            "space 1's load is on the panel's left"
        );
        assert!(
            x(&others[1]) < x(&others[0]),
            "and what it feeds, further left"
        );
        assert!(x(&others[2]) > x("panel"), "space 2's load is on the right");
        let (bends, _, _) = check(&l);
        assert!(bends <= 2, "no wraparounds: {bends} bends");
    }

    /// `REAL_DOC=path cargo test -p stufftracker-core real_document -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_document() {
        let path = std::env::var("REAL_DOC").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let doc: Document = serde_json::from_value(v["document"].clone()).unwrap();
        let t = std::time::Instant::now();
        let l = layout(&LayoutRequest {
            document: doc.clone(),
            view: flat_view(),
        })
        .unwrap();
        let first = t.elapsed();
        let t = std::time::Instant::now();
        let again = layout(&LayoutRequest {
            document: doc.clone(),
            view: View {
                flat_positions: l.flat_positions.clone().unwrap(),
                ..flat_view()
            },
        })
        .unwrap();
        let second = t.elapsed();
        let (bends, shared, length) = check(&again);
        println!(
            "flat: {} nodes, {} bundles, {} unrouted, {bends} bends, {shared} shared runs, length {length:.0}; layout {first:?}, relayout {second:?}",
            again.nodes.len(),
            again.bundles.len(),
            again.unrouted_connections.len(),
        );
        let t = std::time::Instant::now();
        let nested = layout(&LayoutRequest {
            document: doc,
            view: View::default(),
        })
        .unwrap();
        println!(
            "nested: {} unrouted, layout {:?}",
            nested.unrouted_connections.len(),
            t.elapsed()
        );
    }
}
