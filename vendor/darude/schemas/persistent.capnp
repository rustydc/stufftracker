@0xd87fe9bbd64c0e02;
# WakeupPolicy — the one surviving shape of the old persistent.capnp.
#
# GRAVEYARD (docs/grain-abi-v2.md, stage 4): `interface Persistent` is
# dead. Its dual-role token — "a holder's save() returns a platform
# SturdyRef, but the same method on a provider returns recreate-key
# bytes" — split into two explicit surfaces:
#
#   - holder-facing durability is `cap.v1`'s save/forget (cap-v1.capnp),
#     answered by the membrane on every platform-tracked cap — no
#     interface opts in, nothing extends anything;
#   - the provider-facing recreate contract is `Recreatable.key()`
#     (restorable.capnp): the membrane's eager crossing probe calls it,
#     and `Restorable.restore(objectId)` rebuilds from what it returned.
#
# "Token sometimes means objectId" ends. What remains here is the
# wakeup-policy vocabulary both of those surfaces share.

# The Sink's tolerance for being woken to receive a notification on a
# token it owns (docs/durable-held-caps.md). The Sink bears the cold-boot
# cost, so it declares the policy; the platform enforces it. Returned from
# save() (the provider chooses it for the cap being saved) and recorded on
# the holder's saved_caps row.
struct WakeupPolicy {
  kind @0 :Kind;
  # Period for debounce/digest, in milliseconds; ignored otherwise.
  intervalMs @1 :UInt32;
  enum Kind {
    # Wake + deliver on every (coalesced) fire. The default.
    immediate @0;
    # Wake at most once per intervalMs.
    debounce @1;
    # Wake once per intervalMs, only if something is pending.
    digest @2;
    # Never wake proactively; deliver pending on the holder's next
    # natural wake (handshake). Zero extra wakes.
    piggyback @3;
  }
}
