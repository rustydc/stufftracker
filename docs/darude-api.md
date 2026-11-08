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

Connections contain `id`, `name`, `layer`, and `endpoints`, each with an entity ID
and a role (`source`, `target`, or `peer`). Containment cycles, dangling parents,
unknown layers/endpoints, duplicate endpoints, and invalid layer colors are
rejected by the same validation as the HTTP API. A layer color must be six-digit
hex, such as `#ff8800`.

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
