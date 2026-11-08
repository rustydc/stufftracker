@0xe1c2a3b4d5e6f709;
# self.v1 — the reflective handle a principal has on its own authority
# (docs/self-discovery.md, "The `Self` metacap"). This is the READ half:
# enumerate what you hold, and vend a live cap for one entry on demand.
# (The operations half — observe/sever/spawn/attenuate — and `watch` are
# staged later; see the migration in the doc.)
#
# Platform-served (like spawn/schedule/…): the cap on the wire is rooted
# in platformd, not another grain. A grain receives it via the normal
# `requires:` mechanism (`requires: { self: { interface: self.v1 } }`),
# which the compatibility shim (migration step 4) eventually replaces
# with a single bootstrap-issued Self.
#
# HELD, not rooted. `held()` enumerates the caps you *hold* (your shelf),
# NOT the delegation subtree — the two are deliberately distinct
# (docs/self-discovery.md, "Hold vs. rooted"). Sever/observe act on the
# rooted subtree; discovery is what you can use.

using Intro = import "introspectable.capnp";
using Trigger = import "trigger-v1.capnp";
using Observer = import "observer-v1.capnp";

interface Self {
  # Enumerate this principal's authority surface in ONE response — the
  # discovery walk (Broker::held_caps + the platformd interface/rights
  # resolution), partitioned into the two kinds of holding:
  #
  #   `slots`   — the DECLARED requires slots (from the manifest), each
  #               with its current bindings. A slot exists whether or not
  #               anything is bound: an empty `bindings` list is
  #               "declared and waiting" — what an agent needs to ask for
  #               a fill, and what a powerbox UI needs for "what can I
  #               wire here". Singular slots carry 0..1 bindings; `many:`
  #               slots 0..N. A principal with no manifest (a user) has
  #               no slots.
  #   `entries` — everything held OUTSIDE a slot: offered tools,
  #               delegated caps, displaced (unbound) former bindings,
  #               and durable saved caps. The stash. Grants whose slot
  #               was dropped from the manifest land here too (orphans).
  #
  # One call returns both so a watch()-driven re-enumerate can never see
  # a torn state between the two views. LAZY throughout: inline caps are
  # caretakers that dial nothing until called, so enumerating wakes no
  # provider ("restored on use, not on list").
  held @0 () -> (slots :List(Slot), entries :List(Entry));

  # Vend a live cap for the held entry labelled `label` — use after
  # held() to actually obtain and call something you hold. `found` is
  # false (and cap null) when no held entry carries that label.
  #
  # This is where a saved cap is restored (and its provider potentially
  # woken) and a held grant is wrapped in a fresh caretaker — the cap
  # comes back membraned, dialed under the grant's own rights, exactly
  # as a handshake binding would. `interfaceId` + `schema` package it so
  # the caller can lower it into an agent tool (an OpaqueCap) without
  # compile-time knowledge of the type.
  get @1 (label :Text) -> (cap :Capability, interfaceId :UInt64, schema :Data, found :Bool);


  # GRAVEYARD (docs/grain-abi-v2.md, stage 2): the authority-change
  # poke is gone.
  #
  # Replaced by Grain.slotBound/slotRevoked diffs (every delivery is
  # a slot binding now that the offer fork closed) plus the per-turn
  # pulls embodiments do anyway. Ordinal kept as a tombstone; the
  # platform answers Unimplemented.
  watch @2 (cb :Trigger.Trigger) -> ();

  # -- The operations half (docs/self-discovery.md, "The Self metacap") --
  #
  # Reflective ops over the caller's ROOTED subtree — the authority it
  # granted. Each names its `target` by passing the cap the caller holds;
  # the platform recovers the broker id (the same move spawn uses for
  # spawn-passed caps) and checks it is in the caller's rooted subtree
  # before acting — you administer only authority you rooted, never
  # outside your own subtree. Attenuable: each op needs the matching right
  # on the Self grant (`sever` / `observe`), so a Self minted without a
  # right can't perform that op.

  # Revoke `target` (and its whole subtree, transitively). Needs `sever`.
  sever @3 (target :Capability) -> ();

  # Re-derive a live cap from a durable saved-cap TOKEN this principal
  # holds. Only the holder's OWN tokens restore — `found` is false for
  # anyone else's, or for a severed/forgotten token. Lazy: nothing is
  # dialed until the returned cap is called.
  #
  # (docs/durable-held-caps.md, docs/self-discovery.md step 5 — the
  # pull that retires the handshake's `restored` re-vend.) A token
  # whose SturdyRef was severed/forgotten is also GC'd, exactly like
  # the push path did. A trigger token comes back as the wakeup-policy-
  # aware durable trigger (fire() applies the Sink's policy — queue vs
  # wake); any other saved cap comes back as a restore-backed caretaker
  # at its real interface. Lazy either way.
  restore @5 (token :Text) -> (cap :Capability, found :Bool);

  # Attach `observer` to `target`: every call through the target grant
  # fires the observer, with optional content `capture` and a pre-call
  # `gate` veto. Needs `observe`.
  #
  # (docs/audit.md.) The observation is chained under the target grant,
  # so severing the grant tears it down too.
  observe @4 (target :Capability, observer :Observer.Observer,
              capture :Bool, gate :Bool) -> ();

  # Register an interface this grain NOW serves — the runtime sibling
  # of the handshake's provides list, for a grain whose served surface
  # changed after boot (capscript deploying a project's synthesized
  # interface, docs/capscript.md).
  #
  # Same registration path and rules as the handshake batch: provenance
  # is the CALLER (the Self cap is per-grain, so the claim can't be
  # forged), a name registered by a different grain is not overwritable
  # (first-registrant-wins — no shadowing llm.chat.v1), and
  # re-announcing under the same name and id REPLACES the stored schema
  # bytes (schema evolution on redeploy). Carries no cap: live impls
  # still reach consumers only through session handshakes; announce
  # moves the DURABLE fact ("this grain serves X") the moment it
  # becomes true. The platform also nudges this grain's cached sessions
  # to re-open, so a session opened before the announce can't keep
  # serving a stale provides map.
  announce @6 (name :Text, interfaceId :UInt64, schema :Data) -> ();

  # Retract a claim announce (or a handshake) made — the explicit end
  # of "this grain serves `name`". Only the caller's OWN claim;
  # idempotent.
  #
  # The registry keys claims per (grain, name), so retracting cannot
  # touch another grain's claim on the same name. Live sessions keep
  # serving; the claim gates future pins, not existing grants.
  retract @7 (name :Text) -> ();

  # One declared requires slot, with its live fill state. `name` is the
  # manifest logical; `interface`/`rights`/`many` are the DECLARATION
  # (what may bind here, the minimum rights a binding needs, and the
  # cardinality); `bindings` is what's bound right now. A binding entry
  # whose `cap` is null is wired but not vendable through Self (a heavy
  # platform-synthesized manager — still delivered via requires).
  struct Slot {
    name        @0 :Text;
    interface   @1 :Text;
    interfaceId @2 :UInt64;
    rights      @3 :List(Text);
    many        @4 :Bool;
    bindings    @5 :List(Entry);
  }

  # One held cap, projected. `label` names it (get() takes this back);
  # `interface`/`interfaceId` say what it is (for dispatch + lowering);
  # `rights` is what the grant carries; `source` distinguishes a live
  # grant from a durable saved cap; `offerer`/`tag` are provenance.
  struct Entry {
    label       @0 :Text;
    interface   @1 :Text;
    interfaceId @2 :UInt64;
    rights      @3 :List(Text);
    source      @4 :Source;
    offerer     @5 :Text;
    tag         @6 :Text;
    # The LIVE cap for this entry — a lazy caretaker that dials nothing
    # until called. Null for a cap the platform can't vend through Self
    # (a platform-synthesized manager — acquired via requires).
    #
    # (A grant forwards to its provider; a SAVED cap wakes the provider
    # + restore()s the sub-object on first call, at its real interface.)
    # So enumerating still wakes nothing — the "wake" only ever happens
    # on use, for either kind.
    cap         @7 :Capability;
  }

  enum Source {
    grant @0;   # a live broker grant (a bound/offered cap)
    saved @1;   # a durable SturdyRef the principal holds
  }
}
