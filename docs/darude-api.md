# StuffTracker capability API

The grain provides `stufftracker.v1`. Its wire contract is
[`stufftracker-v1.capnp`](../crates/server/stufftracker-v1.capnp). The schema and
caller-facing documentation are compiled into the binary and announced to Darude
at ABI v2 registration. Darude can discover the interface without a platform
source change, and expose the same methods through agents, scripts, and its
capability console.

Install or upgrade the package using `darude.app.yaml` and `img.tar`, then grant
its `stufftracker.v1` capability to a consumer through Darude's powerbox. The
consumer's handle is chosen when wiring the capability; `tracker` below is an
example handle. A read grant can query inventory and views. A write grant can
also mutate them. `owner` implies both. Missing or unrecognized rights grant
neither. Every method checks its session's rights, including calls made after
another session with broader access was opened.

| Methods                                                  | Behavior                                                            |
| -------------------------------------------------------- | ------------------------------------------------------------------- |
| `getDocument`, `putDocument`                             | Read or atomically replace the complete inventory                   |
| `listEntities`, `getEntity`, `putEntity`, `deleteEntity` | Query and edit the containment tree and external links              |
| `listConnections`, `putConnection`, `deleteConnection`   | Query and edit hyperedges, layers, and source/target/peer endpoints |
| `listLayers`, `putLayer`, `deleteLayer`                  | Manage connection layers                                            |
| `listViews`, `putView`, `deleteView`                     | Manage saved perspectives                                           |
| `putPorts`                                               | Set a thing's physical layout: faceplate kind and ports             |

Entity and connection listings accept `offset` and `limit` (default 100, maximum
200), and return `nextOffset`, `hasMore`, and the inventory `revision`. Entity
filters are `kind` and immediate `parent`; connection filters are `layer` and
participating `entity`. Empty filters match all. If the revision changes between
pages, restart the listing for a consistent inventory, or use `getDocument`.

## Editing safely

Inventory mutations require `expectedRevision` from a previous read and return
the new revision. The comparison and write run in a SQLite transaction, shared
with HTTP edits. A stale revision fails with a `409` error; refetch before
retrying. There is no unconditional overwrite mode.

`putEntity`, `putConnection`, and `putLayer` create an ID if absent, or **fully
replace** that object if present. They are not patches: fetch the object and
preserve its other fields when updating only a name, parent, or link.

For example, read `getDocument`, then call `putEntity` with arguments shaped as:

```json
{
  "expectedRevision": 1,
  "entity": {
    "id": "kitchen-pendants",
    "name": "Kitchen pendant lights",
    "kind": "device",
    "parent": "kitchen",
    "links": [
      {
        "provider": "home_assistant",
        "kind": "entity",
        "externalId": "light.kitchen_pendants"
      }
    ]
  }
}
```

Use the actual revision from your read. Root entities use an empty `parent` on
the wire; internally and in the HTTP JSON model this is `null`. External links
are references only; they do not fetch live state or grant Home Assistant access.
`putEntity` replaces the entire `links` list.

Connections contain `id`, `name`, `layer`, and `endpoints`, each with an entity ID,
a role (`source`, `target`, or `peer`), and optionally the entity's `port`. Containment cycles, dangling parents,
unknown layers/endpoints, duplicate endpoints, and invalid layer colors are
rejected by the same validation as the HTTP API. A layer color must be six-digit
hex, such as `#ff8800`.

## Ports and physical layouts

An entity may carry `ports` (switch ports, panel breakers, controller terminals)
and a `faceplate` saying how they are drawn: `panel`, `switch`, `terminals`, or
`none`. Both are returned by `getEntity`, `listEntities` and `getDocument`, and
written by `putDocument`; `putEntity` **keeps** an entity's stored ports and
faceplate, so renaming a panel never drops its breakers. Change them with
`putPorts`, which replaces the entity's whole port list:

```json
{
  "expectedRevision": 7,
  "entity": "panel-c",
  "faceplate": "panel",
  "ports": [
    {
      "id": "1",
      "label": "20 A outlet below C",
      "position": 1,
      "poles": 1,
      "half": ""
    },
    {
      "id": "3/5",
      "label": "30 A well pump",
      "position": 3,
      "poles": 2,
      "half": ""
    },
    {
      "id": "6A",
      "label": "15 A shed lights",
      "position": 6,
      "poles": 1,
      "half": "A"
    },
    {
      "id": "6B",
      "label": "15 A shed outlets",
      "position": 6,
      "poles": 1,
      "half": "B"
    },
    {
      "id": "7B/9A",
      "label": "20 A baseboard",
      "position": 7,
      "poles": 2,
      "halves": ["B", "A"]
    },
    { "id": "MAIN", "label": "100 A main lugs", "main": true }
  ]
}
```

- `id` is unique within the entity and is what an endpoint names; use what is
  printed on the device.
- `position` is the starting space or port number (`0` if unknown). Panels
  number spaces odd down the left column and even down the right. A breaker of
  `poles` N covers position, position+2, … on its side: a 240 V double-pole
  breaker at 3 covers 3 and 5.
- `half` is `A` or `B` for one half of a tandem (twin) breaker; two halves may
  share a space, nothing else may.
- `halves` gives the half of each space instead, one entry per pole (`A`, `B`,
  or `""` for the whole space), for linked breakers that change half between
  spaces. A quad breaker at 1 is two ports: the outer pair `["A", "B"]` (1A/3B)
  and the inner pair `["B", "A"]` (1B/3A); a 7B-to-9A linked pair is
  `["B", "A"]` at 7. Leave `half` empty when `halves` is set. A breaker whose
  spaces are not adjacent (the outer pair) is drawn as tied segments.
- `main` marks the supply: a panel's main breaker or main lugs, a switch's
  uplink, a controller's line input. It takes no `position` and is drawn in a
  row above the numbered positions. Point an incoming feed's target endpoint
  at it (a subpanel's main lugs, a transfer switch's utility and generator
  inputs as two mains). A back-fed main breaker in branch spaces is an
  ordinary breaker.
- `label` is free text: amps, speed or PoE, what the terminal drives.

Overlapping spaces, duplicate IDs, more than three poles, or a half other than
A/B are rejected. A connection endpoint names a port with `port` (empty when
unknown); `putPorts` refuses to remove a port an endpoint still names, so
reassign or clear those endpoints first (or use `putDocument`).

Deleting an entity refuses if it has children or participates in connections.
Deleting a layer refuses while connections use it. Delete or reparent references
explicitly first, or use a validated `putDocument` to make coordinated changes
atomically. Missing inventory objects return `404`; deletes do not silently
increment the revision for an absent object.

## Saved views

A saved view contains `id`, `name`, and `viewJson`. `viewJson` is a JSON-encoded
HTTP `View` object, so layout fields can evolve without changing the capability
wire schema. For example:

```json
{
  "view": {
    "id": "overview",
    "name": "House overview",
    "viewJson": "{\"layout_version\":2,\"positions\":{},\"collapsed\":[\"kitchen\"],\"hidden_layers\":[\"water\"]}"
  }
}
```

A view may also list `faceplates` (entity IDs drawn as their physical layout)
and a `focus` that shows only part of the graph: `entities` to start from (a
space brings its contents), `hops` (connections to follow; omit for all), and
`direction`: `both`, `upstream` (what feeds them) or `downstream` (what they
feed). For example, everything fed by the barn panel, drawn as a faceplate:

```json
{
  "layout_version": 2,
  "positions": {},
  "collapsed": [],
  "hidden_layers": [],
  "faceplates": ["barn-panel"],
  "focus": { "entities": ["barn-panel"], "direction": "downstream" }
}
```

A focused view is laid out compactly by itself; its `positions` are those of
the full view and are left untouched.

Saved views do not change inventory revisions and use last-writer-wins by ID,
matching the existing HTTP behavior. View JSON and coordinates are validated;
`putView` accepts at most 2 MiB of view JSON. `deleteView` is idempotent.

Errors are Cap'n Proto failures prefixed with HTTP-style codes: `400` for invalid
input or referenced-object deletion, `403` for insufficient rights, `404` for a
missing object, `409` for stale inventory revisions, and `500` for storage errors.

## Runtime and verification

SQLite initialization and HTTP socket binding finish before the runtime registers
with `DARUDE_PLATFORM_SOCKET`. Both surfaces share the same database and storage
operations. Disconnecting from the platform ends the process; Darude restarts or
wakes it and the interface is registered and announced again. No external network
access is required.

Standalone mode continues serving its existing HTTP API without a platform
socket. The Cap'n Proto API is available through a Darude session; it does not
open an unauthenticated RPC TCP port.

Run `cargo test --workspace`. Tests cover every method's rights, typed inventory
and link round trips, pagination, validation, cross-surface revision conflicts,
and saved views. A real child-process test uses a mock Darude bootstrap over a
Unix socket to verify registration, schema/documentation announcement, session
rights, HTTP/RPC shared state, and persistence across restart. This is protocol
integration coverage; it does not run Darude's full sandbox or hibernation stack.

The grain SDK is vendored from a pinned Darude commit under `vendor/darude` so
builds do not depend on a neighboring checkout. See its provenance file before
upgrading the SDK or patched Cap'n Proto crates.
