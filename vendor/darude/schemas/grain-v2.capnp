@0xb3d7f8a2c95e4d11;
# Grain ABI v2 — the dial-in registration protocol
# (docs/grain-abi-v2.md, "Lifecycle"). platformd binds one
# platform.sock per grain into the run dir; the grain boots holding
# exactly that socket, dials it when — and only when — its
# initialization is done, and registers. THE REGISTER CALL ARRIVING IS
# READINESS: no socket polling, no handshake retry loops, no
# ready_secs heuristics. Connection drop is grain-gone;
# re-registration after a wake carries a fresh slot snapshot, which is
# the entire hibernation-resync protocol.
#
# Sessions are OBJECTS on this one connection, not connections of
# their own: the platform calls initSession per (principal, rights)
# and holds the returned Session; eviction is dropping the reference.
# Session.get is the pull that replaced the v1 provides list — nothing
# is snapshotted, so nothing goes stale, and the rebind nudge has no
# reason to exist for a v2 grain.
#
# The v1 session ABI is retired (graveyard stage 3), and there is no
# `abi:` manifest knob: a capnp surface in the manifest (an api, an
# export, hooks, or an interface-typed require) is what makes platformd
# bind platform.sock and set DARUDE_PLATFORM_SOCKET — the runtime's
# serve_grain refuses to run without the env var.

using P = import "principal.capnp";
using C = import "cap-v1.capnp";

interface Bootstrap {
  # Register the grain: readiness, by definition. `slots` is the
  # current wiring snapshot; `ctl` is the machine's platform handle
  # for claims; `principal` is null unless the manifest declares
  # `embodied: true` — identity is provisioned at birth, not wired.
  #
  # `slotDecls` is the grain's needs-claims, carried IN the register
  # call (the manifest exit: slots are code-declared, not manifest
  # entries) and recorded BEFORE readiness fires — so anything that
  # awaited this grain's registration can route offers against its
  # slots without a second sync point. Upserted per registration;
  # GrainCtl.declareSlot remains for slots grown later at runtime.
  #
  # Response `slots` are the wiring bindings, each pre-resolved —
  # rows projected once at the sync point. A re-registration (wake,
  # restart) replaces the previous one and carries fresh slots.
  registerGrain @0 (grain :Grain, slotDecls :List(SlotDecl))
      -> (ctl :GrainCtl, slots :List(SlotBinding), principal :Principal);
}

# Grain-implemented: the machine the platform calls into.
interface Grain {
  # Open a serving context. principal/rights are DATA about who is
  # calling — render their view, enforce app logic — never an
  # authority channel.
  #
  # Attribution of the grain's own outgoing calls is explicit
  # (Cap.attenuate(tag), docs/grain-abi-v2.md "Attribution").
  # Session death = the platform dropping the cap.
  initSession @0 (principal :P.Principal, rights :List(Text), isPlatform :Bool)
      -> (session :Session);

  # Wiring diffs, fired while the registration lives. Best-effort
  # NOTIFICATIONS: rows are the truth, a hibernated grain misses them
  # and reconciles from the snapshot at its next registerGrain.
  slotBound   @1 (binding :SlotBinding) -> ();
  slotRevoked @2 (slot :Text, label :Text) -> ();
}

# Grain-implemented: one serving context.
interface Session {
  # Pull a served interface. `found` false (cap null) when this
  # session doesn't serve `interface` — the platform's caretakers
  # pull on first use and cache per session, pipelined so the first
  # method call and the pull share a round trip.
  get @0 (interface :Text) -> (cap :Capability, found :Bool);
}

# Platform-implemented: the machine's half of the claims registry
# (docs/grain-abi-v2.md, "The registry"). The v2 face of
# Self.announce/retract — same table, same rules: provenance is this
# grain, re-announcing upserts, wire-known names claim the fact
# without the bytes.
interface GrainCtl {
  announce @0 (name :Text, interfaceId :UInt64, schema :Data) -> ();
  retract  @1 (name :Text) -> ();

  # Needs-claims (docs/grain-abi-v2.md, "Slots"): declare / retract
  # the slots others may wire — the mirror of announce/retract.
  #
  # Same single-writer discipline, persisted as rows so the powerbox
  # renders and validates offers with the grain ASLEEP. Slots are NOT
  # manifest entries: the code is the truth about its own needs, and
  # a plugin host may grow a slot at runtime. Bindings are never
  # gated on declarations (rows tolerate skew: pre-boot spawn wiring,
  # orphans on retraction).
  declareSlot @2 (slot :SlotDecl) -> ();
  retractSlot @3 (name :Text) -> ();
}

# Platform-implemented: explore a principal's tree (docs/
# grain-abi-v2.md, "Principal"). One interface serves an embodied
# grain's judgment loop and the shell's capabilities page — a user is
# a principal whose client happens to be the shell.
interface Principal {
  whoami @0 () -> (info :PrincipalInfo);

  # My in-edges, atomically: every held cap as (desc, schema, cap) —
  # desc.slot marks slot bindings, "" = unslotted. Lazy caretakers;
  # enumerating wakes nothing.
  held @1 () -> (entries :List(Entry));

  # My rooted subtree — the edges granted OUT of my authority, each
  # with who holds it. v1's Self could sever these but never see them.
  granted @2 () -> (edges :List(GrantedEdge));

  # Re-derive a live cap from a saved-cap token (Cap.save's output).
  restore @3 (token :Data) -> (cap :Capability, found :Bool);

  # Prune an edge I granted / attach an observer to one. Declared for
  # the full doc shape; unimplemented until the membrane grows a
  # cap→row reverse map (callers get Unimplemented, not surprises).
  sever   @4 (target :Capability) -> ();
  observe @5 (target :Capability, capture :Bool, gate :Bool) -> ();

  # The durable persona, on the principal row — the agent survives
  # its bodies.
  #
  # `persona` is OWNER-controlled (the shell edits it; grains inject
  # it as a system prompt); `notes` is the agent's own brief pinned
  # text ("Humor setting: 75%"), writable here. Small fields by
  # design: the full memory apparatus is grain territory.
  profile     @6 () -> (persona :Text, notes :Text);
  updateNotes @7 (notes :Text) -> ();

  struct Entry {
    desc   @0 :C.Cap.Desc;
    schema @1 :Data;
    cap    @2 :Capability;
  }
  struct GrantedEdge {
    desc    @0 :C.Cap.Desc;   # the edge as its holder sees it
    subject @1 :Text;     # who holds it (instance or user id)
    capId   @2 :Text;     # the row — what sever-by-id APIs take today
  }
}

# Who a principal IS — richer than the per-session P.Principal
# (username/displayName): the whoami answer for an embodied grain.
struct PrincipalInfo {
  id      @0 :Text;   # instance id, or user id
  name    @1 :Text;   # display name
  kind    @2 :Kind;
  owner   @3 :Text;   # owning user id, for grain principals ("" = none)
  package @4 :Text;   # "" for users
  enum Kind { user @0; grain @1; anonymous @2; }
}

# A declared slot: a typed PURPOSE others may wire into. `interface`
# empty = untyped (accepts anything, explicit-pick only — never
# auto-routed); `many` slots accumulate labeled bindings (the
# reified stash — "tools" by convention, never by platform magic).
struct SlotDecl {
  name      @0 :Text;
  interface @1 :Text;
  rights    @2 :List(Text);   # minimum rights a binding needs
  many      @3 :Bool;
  doc       @4 :Text;         # powerbox hint ("formula tools")
}

# One wiring binding, as delivered at registration. `slot` is the
# declared slot name; `label` distinguishes bindings of a `many:`
# slot (== slot for singular ones).
struct SlotBinding {
  slot  @0 :Text;
  label @1 :Text;
  cap   @2 :Capability;
}
