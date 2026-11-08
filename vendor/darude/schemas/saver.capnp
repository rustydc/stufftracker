@0xf2db253e9eb1b7f9;
# Saver — mint a SturdyRef token for one of the grain's OWN restorable
# sub-objects. The save-side counterpart to restorable.capnp's
# `restore()`.
#
# A grain that wants to hand a peer a durable reference to one of its
# sub-objects (a specific document, a callback target, a Trigger that
# delivers into one of its objects) advertises `provides: restorable.v1`
# (to reconstruct on the way back) and receives a `Saver` in its
# `requires` (to mint on the way out).
#
#   token = saver.save("trigger.v1", blob)
#
# `objectId` is the same opaque blob `restorable.restore()` will get
# back — grain-defined bytes that name/encode whatever it needs to
# rebuild the cap (a row key, a self-describing delivery instruction,
# ...). The returned `token` is plain bytes: the grain can embed it in a
# payload or hand it to a peer, and because no live capability crosses
# the wire the non-transitive caretaker membrane never applies.
#
# Authorization: the platform binds the new token's parent_cap_id from
# THIS `Saver` cap's own provenance (the grant that delivered it to the
# grain), so revoking the grain's access severs every token it minted —
# no parent argument is needed or accepted.

interface Saver {
  save @0 (interface :Text, objectId :Data) -> (token :Data);
}
