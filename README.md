# StuffTracker

An independent inventory tree and an interactive map of the systems connecting it.
Rust owns the graph model, collapse projection, layout and orthogonal routing. The
same core runs in the browser through WASM or natively on the Rust server. SQLite
stores the inventory and saved perspectives. The TypeScript shell renders SVG and
handles browser input.

## Run standalone

Prerequisites: Rust 1.95+, Node 22+, a C compiler (bundled SQLite), the
`wasm32-unknown-unknown` Rust target, and `wasm-bindgen-cli` **0.2.114**.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.114 --locked
npm ci
npm run build
cargo run -p stufftracker
# Open http://127.0.0.1:8090
```

The server embeds the entire viewer, including WASM, into its binary. Build the
viewer before compiling the server. `STUFFTRACKER_STATE_DIR` defaults to `data/`;
the database is `stufftracker.sqlite3`. `STUFFTRACKER_BIND` defaults to
`127.0.0.1:8090`. Standalone mode is a trusted local app without user accounts.

A sample Juniper House inventory is seeded transactionally on first boot only.
It includes spaces, devices, power circuits, Ethernet/Wi-Fi, water, and an
automation. Subsequent boots preserve your data.

## Explore and edit

- Spaces are nested boxes: a house contains floors, floors contain rooms, and rooms
  contain devices. Drag a space header to move its entire subtree, or drag a device
  within its parent. Drag empty canvas/space interiors to pan; scroll to zoom;
  **Fit all** reframes.
- Click a card or a tree row to inspect it. Use a space's +/− button or double-click
  its card to collapse/expand. Collapse and layer toggles preserve all positions.
- Toggle power, network, water, and automation layers independently.
- Use **+** in the inventory sidebar to add a child of the selected thing. Apply
  name/type/parent edits in the inspector, or add a directed or undirected
  connection. Multiple targets make a hyperedge. Delete removes a subtree and
  prunes affected endpoints. **Save changes** commits inventory edits to SQLite.
- **Save view** stores positions, folded branches, and layer visibility separately
  from inventory data. Selecting a saved view and saving updates it.
- **Export viewer** downloads one self-contained HTML file with the current
  inventory and perspective, JS, CSS, and embedded Rust/WASM. Open it directly
  from disk, offline. Viewers can drag, expand/collapse, pan/zoom, toggle layers,
  and export another snapshot. They cannot edit the inventory or save to a server.
  Interaction is stateless unless exported again.

On narrow screens the inspector is hidden: use a desktop viewport to edit.

## Darude grain

This is a separate repository, not a Darude workspace crate. It follows Darude's
[grain-authoring contract](https://github.com/rustydc/darude/blob/main/docs/grain-authoring.md).

On Linux, with the standalone build prerequisites plus Python 3:

```sh
scripts/pack-darude.sh
```

That produces `img.tar` beside `darude.app.yaml`. The packer includes the Rust
binary and its shared-library closure in a standard OCI archive, without Docker
or a dependency on the Darude checkout. Install the directory through
Darude's `POST /api/apps` with `{"appDir":"/absolute/path/to/stufftracker"}`.
The platform's HTTP-only readiness gate is the Unix socket binding, which happens
**after** SQLite setup. State uses `DARUDE_STATE_DIR`; HTTP uses
`DARUDE_HTTP_SOCKET`. No capnp bootstrap is necessary for this HTTP-only version.
A grain never binds a TCP listener. Missing Darude permissions fail closed for
writes: only `write` or `owner` in the proxy-injected `X-Darude-Permissions` allows
inventory or saved-view mutation. Read shares can rearrange their local view and
export snapshots. The app requires no egress.

## Model and API

Each entity has one parent and arbitrary external references
`{provider, kind, external_id}`. Home Assistant references will point to its areas,
devices or entities without making its hierarchy authoritative here. A sample
link is attached to the kitchen pendants; no live HA connection exists yet.

Connections contain two or more unique entity endpoints. Endpoints have `source`,
`target`, or `peer` roles. One source and several targets represent fanout;
all peers represent an undirected connection; several source/target endpoints
can represent richer relationships. Each connection belongs to a layer.

The projection replaces hidden endpoints with visible collapsed ancestors and
bundles connections with identical visible endpoint/role sets and layer. Each
bundle routes branches to a shared junction inside the endpoints’ common ancestor. Orthogonal paths use a rectilinear
visibility grid and Dijkstra search with a bend penalty around card rectangles.
Positions are parent-relative offsets, allocated once and retained for hidden
entities. Rust computes nested container sizes and absolute frames. Dragging beyond a
parent grows its bounds, recursively expanding ancestors; bounds shrink again
to the current contents when that space is no longer needed. Dragging left or up
moves container origins while preserving sibling world positions. Moving a space translates all descendants
without rewriting their offsets. Collapse retains expanded sibling slots so
neighbors stay fixed. Routing follows the hierarchy one level at a time: each space treats its immediate
child boxes as solid obstacles. A connection crosses each ancestor through a shared
boundary gateway, then continues through the containing space. Unrelated rooms
cannot become shortcuts, and branches cannot leave and re-enter an endpoint room.
Junctions are selected in nearby common-space corridors rather than at the far
edge of the floor. Room-local branches share the same gateway. Independent connections reserve separate tracks with a 10-unit
centerline gap (8 units of padding around 2-unit strokes). Device ports and room
gateways use separate attachment positions. All ports and space boundary crossings
use the left or right edge, with horizontal approaches; routing and subsequent
geometry optimization cannot introduce top/bottom entries.

Connections crossing the same region of the hierarchy prefer a shared corridor.
Lane pools are centered in gaps between expanded sibling footprints, with the
existing 10-unit separation. A source space linking to neighboring spaces uses
the same preferred gap, and lower hierarchy levels inherit the approach side.
Collapsed spaces retain their expanded footprint for corridor planning, keeping
these assignments stable. Affinity is a soft cost on vertical runs: long runs
prefer the shared lanes, while an obvious direct horizontal link stays straight.
The router tries distinct free-space regions and may relax affinity when needed
to fit a complete connection; side-only crossings and padding remain hard rules.

Parallel runs in either flow direction
respect this spacing; right-angle crossings remain allowed. Branches of the same
bundled connection may intentionally share trunks. Junction dots are derived from the union
of the final routed line segments: only points with three or four distinct
directions receive markers. Overlapping tails are deduplicated, so a solver anchor,
ordinary bend, or straight run does not automatically get a dot. Each connection
is analyzed independently; crossings of unrelated connections are not joins. Reservations affect routes,
not entity positions. A final geometry pass removes avoidable elbows across nested
space boundaries. It keeps endpoint attachment directions, required track
clearance, shared room entries, and existing branch joins, and rejects shortcuts
through unrelated rooms or room headers. Unshared boundary crossings may align
into a single straight run instead of adding a step for every nesting level. Shared tracks can also align as a unit: all incident branches and their
room crossings move together, and unused common tails to a solver anchor are
trimmed. Entity positions and endpoint ports remain fixed. If a corridor cannot fit a complete connection with the
required spacing, the layout reports it in `unrouted_connections` and the viewer
shows a “need more routing space” count instead of drawing overlapping or partial
connections. Moving a card
recomputes routes without relaying out neighbors. This is a starting engine,
not a full hierarchical channel router: partial-route trunk bundling across different connections, incremental route
caching and manual waypoints are future work.
Overlapping cards or congested corridors can make a connection unroutable; it is
reported rather than drawn through obstacles or without the required padding. Prototype limits are 500 entities/2,000 connections;
performance at those limits is not yet established.

| Endpoint               | Behavior                                                                     |
| ---------------------- | ---------------------------------------------------------------------------- |
| `GET api/document`     | `{revision, document}`                                                       |
| `PUT api/document`     | Validated atomic replacement with matching revision; stale revisions get 409 |
| `GET api/views`        | Saved perspectives                                                           |
| `POST api/views`       | Upsert `{id, name, view}`                                                    |
| `DELETE api/views/:id` | Delete perspective                                                           |
| `POST api/layout`      | Native layout of `{document, view}` using the same Rust crate                |
| `GET api/session`      | Whether this request can persist changes                                     |

SQLite uses WAL and a transaction for inventory writes. Schema v1 separates
entities, connections, layers, views, and the document revision. Entity/connection
payloads are JSON within those tables so model changes can evolve independently.
Concurrent inventory editors use revision checks; saved views currently use
last-write-wins. View layout version 2 stores parent-relative positions. Earlier
flat views rebuild coordinates once while retaining their collapse and layer
choices; saving the view persists the migrated geometry.

## Validation

```sh
npm run build
npm run check
npm run export:sample
cargo test --workspace
cargo fmt --all -- --check
# With a disposable sample server running (browser tests edit its inventory):
CHROMIUM_PATH=/usr/bin/chromium npm run test:browser
# Or omit CHROMIUM_PATH if using Playwright-managed Chromium.
cargo build --release -p stufftracker
npm run test:grain
python3 scripts/verify-oci.py img.tar
```

Browser coverage exercises real WASM rendering, dragging, stable collapse/filter
positions, saved views, directed connections, conflict handling, and a downloaded
HTML loaded offline with no HTTP requests. Managed Chrome blocks `file:` navigation
in this environment, so the test feeds the downloaded HTML directly to an offline
browser context. The Darude package should additionally
be verified against a running platform across cold start, restart and hibernation
before treating it as released. A capnp tool surface and capability-mediated HA
integration are intentionally deferred.

`test:grain` runs an isolated process with a disposable SQLite database and Unix
socket, checking readiness, share permissions, revision conflicts, saved views,
and state persistence across a stop/start. Full Darude sandbox/hibernation tests
require a running platform and are not covered by this process-level check.

`npm run export:sample` generates `work/sample-viewer.html` from the current
build and the sample inventory. It is a self-contained stateless viewer with the
nested-space layout. `POST api/move` accepts `{document, view, entity, position}`
and returns a view with a parent-constrained move, using the same Rust operation
as browser dragging; `position` is in world coordinates.

Routing retains per-connection attachment hints in saved views and offline exports.
Previous device ports and space gateways receive a hysteresis preference during
dragging, so nearly equal paths do not repeatedly swap attachment sides or slots.
Obstacle clearance and wire spacing remain hard constraints.

Mobile canvas gestures support two-finger pinch zoom and pan around the moving
finger midpoint. Lifting one finger continues panning without resuming a node drag.
Run `npm run test:mobile` for touch-input checks against the server and offline export.
