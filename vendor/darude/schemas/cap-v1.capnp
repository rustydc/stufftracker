@0xe7a9c4d21b8f3a15;
# cap.v1 — the membrane-implemented surface EVERY platform-tracked cap
# answers (docs/grain-abi-v2.md, "Cap"). No grain implements this and
# no interface declares it: the caretaker intercepts the interface id
# on every wrapped cap, so the surface is true by construction —
# synthesized interfaces included, third parties can't get it wrong.
# The edge-in-hand dual of Self's root-down view: describe what you
# hold, narrow it, persist it, renounce it.

using P = import "persistent.capnp";

interface Cap {
  # Describe the edge you hold: the grant row's projection plus the
  # claimed schema bytes — answered by the membrane, no grain wakes.
  #
  # A bare cap on any wire is enough; carrying (cap, interfaceId,
  # schema) triples next to caps is a receiver-side cache, not
  # protocol.
  describe @0 () -> (desc :Desc, schema :Data);

  # Mint a narrowed child of the edge you hold: `rights` a subset of
  # THIS grant's rights (empty = keep the parent's), `tag` branding
  # calls through the child for attribution ("alice").
  #
  # Attenuation never crosses an interface's rights vocabulary. The
  # child parents under this grant, so severing you severs it.
  # Subsumes Taggable.withTag (attenuate with rights=[] is withTag).
  attenuate @1 (rights :List(Text), tag :Text) -> (cap :Capability);

  # Durability — THE holder-facing save/forget (stage 4: the old
  # Persistent mixin is graveyard). save() always returns a real
  # platform token.
  #
  # Sturdy refs and wakeup policies per docs/durable-held-caps.md,
  # behind one universal surface; the provider-facing recreate probe
  # is Recreatable.key (restorable.capnp).
  save   @2 () -> (token :Data, policy :P.WakeupPolicy, name :Text);
  forget @3 (token :Data) -> ();

  # Renounce the edge YOU hold — revoked_at on this grant, taking
  # everything minted under it along. Always safe (authority can only
  # shrink), so it needs no right.
  #
  # The dual of Self.sever, which prunes edges you GRANTED; two verbs
  # so audit never conflates them.
  drop @4 () -> ();

  # One held edge, projected — whoever's asking, whichever door
  # (describe here; Self.held is the bulk read of the same shape).
  struct Desc {
    label       @0 :Text;          # holder-relative (offer label / logical)
    interface   @1 :Text;
    interfaceId @2 :UInt64;
    rights      @3 :List(Text);
    source      @4 :Source;
    offerer     @5 :Text;
    tag         @6 :Text;          # attribution brand, if attenuated
    slot        @7 :Text;          # bound slot (requires logical); "" = none
  }
  enum Source {
    grant @0;   # a live broker grant
    saved @1;   # a durable saved-cap wrapper
  }
}
