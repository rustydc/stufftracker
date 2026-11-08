// Re-exports of the generated capnp modules. Downstream crates use
// `darude_wire::bootstrap_capnp::bootstrap_api::Client` etc.
//
// Each schema file additionally exposes its serialized
// `CodeGeneratorRequest` bytes as a `pub const` (e.g.
// WAVE_V1_SCHEMA). Rust apps populating an OpaqueCap grab the matching
// constant for the schema that defines their cap's interface.
// See platformd-rs/schemas/introspectable.capnp for the protocol.

// capnpc-generated code triggers various style lints. We don't own it,
// so silence the lints rather than patch generated output.
#![allow(clippy::all, clippy::pedantic, missing_docs, unused_parens)]

pub mod accept_cap_capnp {
    include!(concat!(env!("OUT_DIR"), "/accept_cap_capnp.rs"));
}
pub mod agent_notes_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/agent_notes_v1_capnp.rs"));
}
pub mod cap_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/cap_v1_capnp.rs"));
}
pub mod grain_v2_capnp {
    include!(concat!(env!("OUT_DIR"), "/grain_v2_capnp.rs"));
}
pub mod blob_capnp {
    include!(concat!(env!("OUT_DIR"), "/blob_capnp.rs"));
}
pub mod cap_echo_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/cap_echo_v1_capnp.rs"));
}
pub mod change_listener_capnp {
    include!(concat!(env!("OUT_DIR"), "/change_listener_capnp.rs"));
}
pub mod code_exec_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/code_exec_v1_capnp.rs"));
}
pub mod delegate_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/delegate_v1_capnp.rs"));
}
pub mod llm_chat_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/llm_chat_v1_capnp.rs"));
}
pub mod introspectable_capnp {
    include!(concat!(env!("OUT_DIR"), "/introspectable_capnp.rs"));
}
pub mod saver_capnp {
    include!(concat!(env!("OUT_DIR"), "/saver_capnp.rs"));
}
pub mod schedule_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/schedule_v1_capnp.rs"));
}
pub mod shelf_capnp {
    include!(concat!(env!("OUT_DIR"), "/shelf_capnp.rs"));
}
pub mod powerbox_request_capnp {
    include!(concat!(env!("OUT_DIR"), "/powerbox_request_capnp.rs"));
}
pub mod user_notify_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/user_notify_v1_capnp.rs"));
}
pub mod observer_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/observer_v1_capnp.rs"));
}
pub mod persistent_capnp {
    include!(concat!(env!("OUT_DIR"), "/persistent_capnp.rs"));
}
pub mod principal_capnp {
    include!(concat!(env!("OUT_DIR"), "/principal_capnp.rs"));
}
pub mod restorable_capnp {
    include!(concat!(env!("OUT_DIR"), "/restorable_capnp.rs"));
}
pub mod self_notify_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/self_notify_v1_capnp.rs"));
}
pub mod self_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/self_v1_capnp.rs"));
}
pub mod session_capnp {
    include!(concat!(env!("OUT_DIR"), "/session_capnp.rs"));
}
pub mod spawn_capnp {
    include!(concat!(env!("OUT_DIR"), "/spawn_capnp.rs"));
}
pub mod subagent_capnp {
    include!(concat!(env!("OUT_DIR"), "/subagent_capnp.rs"));
}
pub mod text_writer_capnp {
    include!(concat!(env!("OUT_DIR"), "/text_writer_capnp.rs"));
}
pub mod byte_writer_capnp {
    include!(concat!(env!("OUT_DIR"), "/byte_writer_capnp.rs"));
}
pub mod trigger_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/trigger_v1_capnp.rs"));
}
pub mod web_fetch_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/web_fetch_v1_capnp.rs"));
}
pub mod web_search_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/web_search_v1_capnp.rs"));
}
pub mod workspace_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/workspace_v1_capnp.rs"));
}
pub mod fs_v1_capnp {
    include!(concat!(env!("OUT_DIR"), "/fs_v1_capnp.rs"));
}

/// Raw `CodeGeneratorRequest` bytes for each platform .capnp file, one
/// constant per file. To send a cap as an OpaqueCap, the sender drops the
/// matching constant into `OpaqueCap.schema` (e.g. CODE_EXEC_V1_SCHEMA for
/// any cap implementing `code.exec.v1`). The receiver's SchemaLoader parses
/// it once and dispatches calls dynamically — see darude-runtime's
/// `introspect` module and its use in the llm-openai-provider agent loop.
///
/// Each blob is the file's transitive closure: e.g. cap-echo-v1 imports
/// llm-chat, so CAP_ECHO_V1_SCHEMA's CodeGeneratorRequest contains both
/// interfaces' nodes — one parse gives the loader everything the receiver
/// needs to dispatch. App-defined interfaces (wave, miniflux, …) live in
/// their own bridge crates, not here.
pub const ACCEPT_CAP_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/accept-cap.schema.bin"));
pub const AGENT_NOTES_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/agent-notes-v1.schema.bin"));
pub const CAP_V1_SCHEMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cap-v1.schema.bin"));
pub const CAP_ECHO_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/cap-echo-v1.schema.bin"));
pub const CHANGE_LISTENER_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/change-listener.schema.bin"));
pub const CODE_EXEC_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/code-exec-v1.schema.bin"));
pub const DELEGATE_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/delegate-v1.schema.bin"));
pub const LLM_CHAT_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/llm-chat-v1.schema.bin"));
pub const INTROSPECTABLE_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/introspectable.schema.bin"));
pub const SAVER_SCHEMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/saver.schema.bin"));
pub const SCHEDULE_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/schedule-v1.schema.bin"));
pub const SHELF_SCHEMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/shelf.schema.bin"));
pub const POWERBOX_REQUEST_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/powerbox-request.schema.bin"));
pub const USER_NOTIFY_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/user-notify-v1.schema.bin"));
pub const OBSERVER_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/observer-v1.schema.bin"));
pub const PERSISTENT_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/persistent.schema.bin"));
pub const RESTORABLE_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/restorable.schema.bin"));
pub const SPAWN_SCHEMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/spawn.schema.bin"));
pub const SUBAGENT_SCHEMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/subagent.schema.bin"));
pub const TEXT_WRITER_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/text-writer.schema.bin"));
pub const BYTE_WRITER_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/byte-writer.schema.bin"));
pub const TRIGGER_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/trigger-v1.schema.bin"));
pub const SELF_NOTIFY_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/self-notify-v1.schema.bin"));
pub const SELF_V1_SCHEMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/self-v1.schema.bin"));
pub const WEB_FETCH_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/web-fetch-v1.schema.bin"));
pub const WEB_SEARCH_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/web-search-v1.schema.bin"));
pub const WORKSPACE_V1_SCHEMA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/workspace-v1.schema.bin"));
pub const FS_V1_SCHEMA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fs-v1.schema.bin"));

// -- Schema registry --------------------------------------------------
//
// Interface name/id -> CGR schema bytes, backed by a process-global
// registry instead of a compile-time `match`. It's seeded once with the
// schemas compiled into this crate (built-ins below), and `register_schema`
// lets the platform add more at runtime — e.g. schemas a grain ships in
// its SessionBootstrap handshake (phase 2), so a third-party interface
// becomes offerable/introspectable without recompiling platformd.
//
// `schema_for_interface{,_id}` keep returning `&'static [u8]`: built-in
// bytes are already `'static`, and runtime-registered bytes are leaked
// once (a bounded, intentional leak — each interface is registered at
// most once and its schema lives for the whole process). This keeps
// every call site byte-for-byte identical to the old free functions.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Copy)]
struct SchemaEntry {
    id: u64,
    bytes: &'static [u8],
}

/// Every interface schema compiled into THIS crate, by name -> (id,
/// bytes). This is the static build artifact: a grain in this workspace
/// links `darude_wire`, so it can self-describe its provided interfaces
/// from here at SessionBootstrap time (see runtime's SessionProvides).
/// Unlike the registry below, this never changes and includes the
/// app-defined interfaces too — a grain providing one needs its bytes.
fn compiled() -> &'static HashMap<&'static str, SchemaEntry> {
    static MAP: OnceLock<HashMap<&'static str, SchemaEntry>> = OnceLock::new();
    MAP.get_or_init(|| {
        use capnp::traits::HasTypeId;
        let mut m = HashMap::new();
        macro_rules! compiled {
            ($name:literal, $client:ty, $bytes:expr) => {
                m.insert(
                    $name,
                    SchemaEntry {
                        id: <$client as HasTypeId>::TYPE_ID,
                        bytes: $bytes,
                    },
                );
            };
        }
        compiled!(
            "accept_cap.v1",
            accept_cap_capnp::accept_cap::Client,
            ACCEPT_CAP_SCHEMA
        );
        compiled!(
            "agent_notes.v1",
            agent_notes_v1_capnp::agent_notes::Client,
            AGENT_NOTES_V1_SCHEMA
        );
        compiled!("cap.v1", cap_v1_capnp::cap::Client, CAP_V1_SCHEMA);
        compiled!(
            "cap_echo.v1",
            cap_echo_v1_capnp::cap_echo::Client,
            CAP_ECHO_V1_SCHEMA
        );
        compiled!(
            "change_listener.v1",
            change_listener_capnp::change_listener::Client,
            CHANGE_LISTENER_SCHEMA
        );
        compiled!(
            "delegate.v1",
            delegate_v1_capnp::delegate::Client,
            DELEGATE_V1_SCHEMA
        );
        compiled!(
            "llm.chat.v1",
            llm_chat_v1_capnp::llm_chat_v1::Client,
            LLM_CHAT_V1_SCHEMA
        );
        compiled!("saver.v1", saver_capnp::saver::Client, SAVER_SCHEMA);
        compiled!(
            "deliver.v1",
            schedule_v1_capnp::deliver::Client,
            SCHEDULE_V1_SCHEMA
        );
        compiled!(
            "schedule.v1",
            schedule_v1_capnp::schedule::Client,
            SCHEDULE_V1_SCHEMA
        );
        compiled!(
            "text_writer.v1",
            text_writer_capnp::text_writer::Client,
            TEXT_WRITER_SCHEMA
        );
        compiled!(
            "byte_writer.v1",
            byte_writer_capnp::byte_writer::Client,
            BYTE_WRITER_SCHEMA
        );
        compiled!(
            "trigger.v1",
            trigger_v1_capnp::trigger::Client,
            TRIGGER_V1_SCHEMA
        );
        compiled!(
            "observer.v1",
            observer_v1_capnp::observer::Client,
            OBSERVER_V1_SCHEMA
        );
        compiled!(
            "self_notify.v1",
            self_notify_v1_capnp::self_notify::Client,
            SELF_NOTIFY_V1_SCHEMA
        );
        compiled!("self.v1", self_v1_capnp::self_::Client, SELF_V1_SCHEMA);
        compiled!(
            "web.fetch.v1",
            web_fetch_v1_capnp::web_fetch_v1::Client,
            WEB_FETCH_V1_SCHEMA
        );
        compiled!(
            "web.search.v1",
            web_search_v1_capnp::web_search_v1::Client,
            WEB_SEARCH_V1_SCHEMA
        );
        compiled!(
            "code.exec.v1",
            code_exec_v1_capnp::code_exec_v1::Client,
            CODE_EXEC_V1_SCHEMA
        );
        compiled!(
            "workspace.v1",
            workspace_v1_capnp::workspace_v1::Client,
            WORKSPACE_V1_SCHEMA
        );
        compiled!(
            "subagent.v1",
            subagent_capnp::subagent_v1::Client,
            SUBAGENT_SCHEMA
        );
        compiled!(
            "subagent_mgr.v1",
            subagent_capnp::subagent_mgr::Client,
            SUBAGENT_SCHEMA
        );
        compiled!("shelf.v1", shelf_capnp::shelf::Client, SHELF_SCHEMA);
        // A NAMED shelf shared with a grain (docs/shelves.md, agent leg):
        // same wire type as the home shelf, distinct dotted name so the
        // grant is ordinary shareable authority — NOT platform-synth, so
        // it flows into Self.held()'s stash and lowers as an agent tool.
        compiled!("shelves.v1", shelf_capnp::shelf::Client, SHELF_SCHEMA);
        compiled!("factory.v1", shelf_capnp::factory::Client, SHELF_SCHEMA);
        // A volume (or any file-tree provider) shared with a user or a
        // grain (docs/fs-caps.md). Like shelves.v1: ordinary shareable
        // authority, NOT platform-synth, so a grain-share flows into
        // Self.held()'s stash and lowers as an agent tool.
        compiled!("fs.v1", fs_v1_capnp::fs_v1::Client, FS_V1_SCHEMA);
        compiled!(
            "powerbox_request.v1",
            powerbox_request_capnp::powerbox_request::Client,
            POWERBOX_REQUEST_SCHEMA
        );
        compiled!(
            "user-notify.v1",
            user_notify_v1_capnp::user_notify_v1::Client,
            USER_NOTIFY_V1_SCHEMA
        );
        m
    })
}

/// Look up a compiled-in schema by interface name. Used by grains to
/// self-describe their provided interfaces at handshake — independent of
/// the runtime registry's state (which is platform knowledge, not the
/// grain's). Returns the full compiled set, app interfaces included.
pub fn compiled_schema_for_interface(name: &str) -> Option<(u64, &'static [u8])> {
    compiled().get(name).map(|e| (e.id, e.bytes))
}

/// Interface names the PLATFORM seeds its registry with at startup: the
/// platform's own infrastructure + the services platformd itself speaks
/// or introspects. Interfaces defined by a single app (cap_echo here; the
/// real apps — wave, miniflux, … — live in their own bridge crates) are
/// deliberately omitted: the platform learns them only when a grain ships
/// them in its handshake (register_schema), which proves platformd doesn't
/// depend on app schemas being compiled into it.
/// (change_listener stays: it's referenced by the cap-param annotation's
/// id->name mapping, not provided by a single app.)
const PLATFORM_SEEDED: &[&str] = &[
    "accept_cap.v1",
    // The membrane-implemented per-cap surface (docs/grain-abi-v2.md):
    // seeded so describe/attenuate lower and introspect platform-wide.
    "cap.v1",
    "change_listener.v1",
    "llm.chat.v1",
    "saver.v1",
    "deliver.v1",
    "schedule.v1",
    // Held-cap delegation: a platform-served cap a grain uses to share a
    // saved sub-cap it holds with another grain (docs/durable-held-caps.md).
    "delegate.v1",
    "text_writer.v1",
    "trigger.v1",
    // The observation-is-a-capability primitive (docs/audit.md): an
    // Observer passed into a grant is delivered to (fire) / consulted
    // (gate) by the caretaker, so the platform must name + introspect it.
    "observer.v1",
    "web.fetch.v1",
    "web.search.v1",
    "code.exec.v1",
    // The subagent surface is platform infrastructure: spawn() types
    // returned caps against subagent.v1, and the parent-side manager
    // tool ships as an OpaqueCap the platform must be able to name.
    "subagent.v1",
    "subagent_mgr.v1",
    // The agent's pinned-notes tool (docs/grain-abi-v2.md "Agents
    // under v2"): an in-process shim the chat lowers, whose CAP
    // crosses the membrane into the llm like any tool — the platform
    // must introspect it to mediate the call, same as subagent_mgr.
    "agent_notes.v1",
    // Shelf + Factory are platform-served (the user's home + the
    // catalog's app-classes); their caps flow to grains as OpaqueCaps
    // (shelf entries), so the platform must name and describe them.
    "shelf.v1",
    "shelves.v1",
    "factory.v1",
    // Platform volumes served as fs.v1 (docs/fs-caps.md): the grants
    // flow to grains as OpaqueCaps, so the platform must name and
    // describe the interface.
    "fs.v1",
    // The consent flow's grain-side door; platform-served.
    "powerbox_request.v1",
    // The attention channel (ping the grain owner's devices);
    // platform-served, lowered into agent tool lists.
    "user-notify.v1",
    // The reflective self-discovery handle (docs/self-discovery.md): a
    // grain enumerates its own held authority and vends a live cap per
    // entry. Platform-served, so the platform must name + introspect it.
    "self.v1",
    // The mint-a-durable-callback tool an agent grain vends to its own
    // model. It rides every agent turn as an OpaqueCap, so the dispatch
    // comes back through the membrane typed against this interface —
    // the platform must introspect it to mediate the call (and the
    // Trigger it returns is what detached runs take as `done`).
    "self_notify.v1",
];

struct Registry {
    by_name: HashMap<String, SchemaEntry>,
    /// Reverse index: capnp type id -> interface name. The type id is the
    /// canonical identity (it's what the wire + dynamic_call key on), so
    /// id lookups are O(1), not a scan, and dedup happens on the id.
    id_to_name: HashMap<u64, String>,
}

fn registry() -> &'static RwLock<Registry> {
    static REG: OnceLock<RwLock<Registry>> = OnceLock::new();
    REG.get_or_init(|| {
        let mut by_name = HashMap::new();
        let mut id_to_name = HashMap::new();
        for name in PLATFORM_SEEDED {
            if let Some(e) = compiled().get(name) {
                by_name.insert(
                    name.to_string(),
                    SchemaEntry {
                        id: e.id,
                        bytes: e.bytes,
                    },
                );
                id_to_name.insert(e.id, name.to_string());
            }
        }
        RwLock::new(Registry {
            by_name,
            id_to_name,
        })
    })
}

/// Is `name` platform-seeded — resolvable from the registry at boot
/// with no grain's help? The announce bytes-elision keys on THIS, not
/// on "compiled": an app-defined interface (cap_echo.v1) is compiled
/// into every workspace binary yet the platform deliberately learns
/// app schemas only from grains, so its announce must ship the bytes.
pub fn platform_seeded(name: &str) -> bool {
    PLATFORM_SEEDED.contains(&name)
}

/// Register an interface's CGR schema bytes at runtime, keyed by its
/// capnp type id (with the name as a descriptor). First-wins on EITHER
/// axis: a grain can't shadow a built-in's name, nor reuse a type id
/// already mapped — one type id is one schema. The bytes are leaked to
/// `'static` so lookups can hand them out as `&'static`; callers pass
/// owned `Vec<u8>` (e.g. from a grain's handshake) and transfer ownership.
pub fn register_schema(name: &str, id: u64, bytes: Vec<u8>) {
    let mut reg = registry().write().unwrap();
    // A NAME maps to exactly one id, first registration wins — a
    // built-in can never be shadowed.
    if reg.by_name.contains_key(name) {
        return;
    }
    // The capnp type id is the interface's IDENTITY; names are aliases.
    // A new name for an already-known id is an ALIAS: it resolves by
    // name (same id, the CANONICAL registration's bytes — one id is one
    // schema, an alias never swaps the bytes), while the reverse
    // id→name lookup keeps answering the canonical (first) name.
    if let Some(canonical) = reg.id_to_name.get(&id).cloned() {
        if let Some(entry) = reg.by_name.get(&canonical).copied() {
            reg.by_name.insert(name.to_string(), entry);
        }
        return;
    }
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    reg.by_name
        .insert(name.to_string(), SchemaEntry { id, bytes: leaked });
    reg.id_to_name.insert(id, name.to_string());
}

/// Re-register an interface whose SCHEMA EVOLVED under the same name
/// and id — the `Self.announce` path for a redeployed capscript
/// project (methods appended, id stable). Unlike `register_schema`
/// (strictly first-wins, so a built-in can't be shadowed), this
/// REPLACES the stored bytes — but only when the name is new or
/// already maps to the SAME id, so it can never repoint a name at a
/// different interface. Returns false (and changes nothing) on an id
/// mismatch. The replaced bytes leak like all registry bytes: bounded
/// by deploys, intentional.
pub fn reregister_schema(name: &str, id: u64, bytes: Vec<u8>) -> bool {
    let mut reg = registry().write().unwrap();
    if let Some(existing) = reg.by_name.get(name) {
        if existing.id != id {
            return false;
        }
    }
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    reg.by_name
        .insert(name.to_string(), SchemaEntry { id, bytes: leaked });
    reg.id_to_name.entry(id).or_insert_with(|| name.to_string());
    true
}

/// Map an interface name (the same string used in manifest `requires:`
/// and `provides:`) to its capnp type id and CGR schema bytes. Powerbox
/// uses this to fill OpaqueCap.interfaceId + OpaqueCap.schema when vending
/// a cap. None for interfaces the platform doesn't know about.
pub fn schema_for_interface(name: &str) -> Option<(u64, &'static [u8])> {
    let reg = registry().read().unwrap();
    reg.by_name.get(name).map(|e| (e.id, e.bytes))
}

/// Every interface name currently in the registry. Order is unspecified
/// (it's a hash map); callers that need determinism should sort.
pub fn known_interfaces() -> Vec<String> {
    let reg = registry().read().unwrap();
    reg.by_name.keys().cloned().collect()
}

/// Reverse lookup: map a capnp interface type id back to its platform
/// name, from the runtime registry only. The registry is seeded with the
/// platform's own interfaces and grown from grain-declared schemas (live
/// handshake + durable rehydrate at boot), so this resolves any interface
/// a connected/known grain provides — without falling back to a
/// compile-time table. None if no grain has declared this id.
pub fn interface_name_for_id(id: u64) -> Option<String> {
    let reg = registry().read().unwrap();
    reg.id_to_name.get(&id).cloned()
}

/// Reverse of `schema_for_interface`: look up CGR bytes by type id. Used
/// by callers that get a cap back from a method (typed `:Capability`
/// field) and need its schema bytes to package it as an OpaqueCap. None
/// if the platform doesn't know this interface.
pub fn schema_for_interface_id(id: u64) -> Option<&'static [u8]> {
    let reg = registry().read().unwrap();
    reg.id_to_name
        .get(&id)
        .and_then(|n| reg.by_name.get(n))
        .map(|e| e.bytes)
}

// Doc comments are no longer a side table here: build.rs injects the
// schema-source-extracted docs into each emitted CGR's sourceInfo
// (darude-schema-docs), so the `*_SCHEMA` bytes themselves carry them.
// Consumers read docs off the loaded schema via
// `darude_runtime::introspect::SchemaCache`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_fetch_interface_is_offerable() {
        // The powerbox offer path calls schema_for_interface to
        // package a cap as an OpaqueCap. web.fetch.v1 must resolve
        // to its type id + non-empty CGR so the agent can lower it.
        let (type_id, schema) = schema_for_interface("web.fetch.v1").expect("registered");
        use capnp::traits::HasTypeId;
        assert_eq!(
            type_id,
            <web_fetch_v1_capnp::web_fetch_v1::Client as HasTypeId>::TYPE_ID
        );
        assert!(!schema.is_empty(), "CGR bytes must ship in the OpaqueCap");
        // Unknown interfaces stay None.
        assert!(schema_for_interface("nope.v1").is_none());
    }

    #[test]
    fn app_interfaces_not_compiled_into_platform() {
        // After the wire breakup, app interfaces (wave.v1) aren't compiled into
        // this crate AT ALL — they live in their bridge crates, and the
        // platform learns them only from a grain's handshake. That's what
        // proves the platform doesn't depend on app schemas.
        assert!(compiled_schema_for_interface("wave.v1").is_none());
        // A platform-internal interface that simply isn't seeded (cap_echo.v1)
        // is still compiled-but-unseeded — registered only on demand.
        assert!(compiled_schema_for_interface("cap_echo.v1").is_some());
        assert!(schema_for_interface("cap_echo.v1").is_none());
        // Platform-own interfaces ARE seeded.
        assert!(schema_for_interface("trigger.v1").is_some());
        assert!(schema_for_interface("accept_cap.v1").is_some());
    }

    #[test]
    fn register_schema_adds_runtime_interface_and_is_idempotent() {
        // A schema not compiled into this crate (e.g. one a grain ships
        // in its handshake) becomes resolvable by name + id after
        // registration — the phase-2 path.
        let id = 0x1234_5678_9abc_def0;
        assert!(schema_for_interface("thirdparty.v1").is_none());
        register_schema("thirdparty.v1", id, vec![1, 2, 3]);
        let (got_id, bytes) = schema_for_interface("thirdparty.v1").expect("registered");
        assert_eq!(got_id, id);
        assert_eq!(bytes, &[1, 2, 3]);
        assert_eq!(schema_for_interface_id(id), Some(&[1u8, 2, 3][..]));
        assert_eq!(interface_name_for_id(id).as_deref(), Some("thirdparty.v1"));
        // Re-registering the same name is a no-op (first registration
        // wins; a built-in can't be shadowed).
        register_schema("thirdparty.v1", 0xdead, vec![9, 9]);
        assert_eq!(schema_for_interface("thirdparty.v1").unwrap().0, id);
    }

    #[test]
    fn register_schema_aliases_on_type_id() {
        // The type id is the interface's IDENTITY; names are aliases. A
        // second name for a known id resolves to the CANONICAL schema
        // (an alias never swaps the bytes — one id is one schema), and
        // the reverse lookup keeps answering the canonical name.
        let id = 0x0bad_f00d_0bad_f00d;
        register_schema("alpha.v1", id, vec![7, 7]);
        register_schema("alpha-alias.v1", id, vec![8]);
        let (alias_id, alias_bytes) =
            schema_for_interface("alpha-alias.v1").expect("alias resolves");
        assert_eq!(alias_id, id);
        assert_eq!(alias_bytes, &[7, 7], "alias serves the canonical bytes");
        assert_eq!(interface_name_for_id(id).as_deref(), Some("alpha.v1"));
        assert_eq!(schema_for_interface_id(id), Some(&[7u8, 7][..]));
    }

    #[test]
    fn builtins_cannot_be_shadowed() {
        // Registering over a compiled-in name leaves the built-in intact.
        let (orig_id, _) = schema_for_interface("web.fetch.v1").unwrap();
        register_schema("web.fetch.v1", 0xbad, vec![0]);
        assert_eq!(schema_for_interface("web.fetch.v1").unwrap().0, orig_id);
    }
}
