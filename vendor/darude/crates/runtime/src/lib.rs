// Grain-side runtime helpers.
//
// ABI v2 (docs/grain-abi-v2.md): a grain with a capnp surface DIALS
// the platform-bound UDS at `DARUDE_PLATFORM_SOCKET` when its init is
// done and calls `Bootstrap.registerGrain` — the register call
// arriving IS readiness. Slot wiring comes back in the registration
// response (plus slotBound/slotRevoked diffs); sessions are objects
// platformd creates over that one registered connection (initSession
// per principal/rights, Session.get pulls the provides). See the
// `sessions` module for the helpers that wrap that flow and the
// `introspect` module for the OpaqueCap receive-side dispatcher.

/// The calling convention all caller-facing surfaces share (agent tools,
/// the console, script engines): a single-struct param/result is flattened.
pub mod call_shape;
/// Introspectable-cap dispatch: load an interface's schema at runtime
/// and call its methods dynamically. Built on the vendored capnp's
/// `schema_loader`; this is the receiver side of the OpaqueCap protocol.
pub mod held;
pub mod introspect;

/// trigger.v1 payload conversion: `TriggerValue` <-> the neutral
/// `introspect::Value` tree, so a fire-and-forget `trigger()` payload
/// lowers to JSON (with cap-refs for any embedded caps) through the
/// same machinery as OpaqueCap tool arguments.
pub mod trigger;

/// Lower a capnp param/result struct to JSON Schema. Shared by the LLM
/// provider (OpenAI tool defs) and the platform console (HTML forms).
pub mod tool_schema;

/// Shared tool-naming: turns a cap's (label, interface) into a stable,
/// identifier-safe handle, so the agent, Monty scripts, and Grist
/// formulas all name the same cap the same way. See `tool_names`.
pub mod tool_names;

/// JSON <-> neutral `Value` conversion. `json_to_value` is schema-
/// directed (JSON flattens int/uint/float); `value_to_json` renders a
/// result tree (caps become a placeholder). Used by the platform console
/// to marshal form input and render results.
pub mod json_value;
/// Links a capability hands to a person: absolute, on the grain's own
/// public URL (`DARUDE_GRAIN_URL`), so they work outside the grain.
pub mod links;
pub mod multiface;

/// Grain-side ABI v2 machinery: dial `DARUDE_PLATFORM_SOCKET`,
/// register, then serve `initSession` over the registered
/// connection — calling a grain-supplied factory to build the
/// per-session provides from a `SessionContext`.
pub mod sessions;

/// Grant-rights checks: the shared "does this cap unlock mutation?"
/// predicate and the standard read-only refusal, replacing per-bridge
/// copies of both. Rights match exactly; `owner` implies every
/// app-level right.
pub mod rights;

/// Source-side durable-watch bookkeeping (docs/durable-held-caps.md):
/// save the caller's persistable Trigger, persist watch rows across
/// hibernation, re-arm from the handshake re-vend, poll-and-fire with a
/// cursor payload. A bridge adds `watch`/`changesSince` by supplying a
/// scope type and a change probe.
pub mod watch;

// Wall-clock millis since the epoch — THE shared clock helper (defined
// in `watch`, where bridges already import it; re-exported here so
// everything else stops hand-rolling copies).
pub use watch::now_millis;

pub use darude_wire::accept_cap_capnp::accept_cap;
pub use darude_wire::blob_capnp::blob;
pub use darude_wire::byte_writer_capnp::byte_writer;
pub use darude_wire::cap_v1_capnp::cap as cap_v1;
pub use darude_wire::change_listener_capnp::{change_listener, subscription};
pub use darude_wire::code_exec_v1_capnp::code_exec_v1;
pub use darude_wire::fs_v1_capnp::fs_v1;
pub use darude_wire::introspectable_capnp::opaque_cap;
pub use darude_wire::llm_chat_v1_capnp::llm_chat_v1;
pub use darude_wire::observer_v1_capnp::{exercise, observer};
/// The reflective self-discovery handle (`requires: self`), re-exported
/// so a grain using only darude_runtime can take the binding — e.g. the
/// pull-first watch re-arm (`WatchSet::rearm_from_self`).
pub use darude_wire::self_v1_capnp::self_;
pub use darude_wire::subagent_capnp::{subagent_mgr, subagent_v1};
pub use darude_wire::text_writer_capnp::text_writer;
pub use darude_wire::trigger_v1_capnp::trigger as trigger_iface;
pub use darude_wire::web_fetch_v1_capnp::web_fetch_v1;
pub use darude_wire::web_search_v1_capnp::web_search_v1;
pub use darude_wire::workspace_v1_capnp::workspace_v1;
