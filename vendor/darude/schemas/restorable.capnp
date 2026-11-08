@0xc4f1a08b6e3a2b97;
# Restorable — grain-side callback for sub-object SturdyRef restore.
#
# A grain that wants to persist references to its OWN sub-objects
# (rows in its database, specific documents, scoped subcaps it
# issues via accept_cap) advertises `provides: restorable.v1` in its
# manifest and registers an impl at handshake time.
#
# Save side: the grain calls
#   BootstrapApi.saveSubObject(interface, objectId)
# where `objectId` is grain-defined opaque bytes — typically a row
# key, scope id, or whatever the grain needs to reconstruct the
# subcap later. The platform mints a SturdyRef token recording
# (grain_id, interface, objectId) and returns it.
#
# Restore side: when any grain calls BootstrapApi.restore(token)
# with a token whose object_id is set, the platform routes the
# request to the OWNING grain's restorable.v1 cap (waking it if
# hibernated) and asks it to reconstitute. The returned cap is
# wrapped in a caretaker before being handed to the requester —
# audit + sever still apply.

using P = import "persistent.capnp";

interface Restorable {
  # Re-vend a previously-persisted sub-object cap. `objectId` is
  # whatever bytes the grain handed the platform via saveSubObject.
  # If the underlying object no longer exists, return a failed
  # promise (the platform translates that into a restore failure).
  restore @0 (objectId :Data) -> (cap :Capability);
}

# Recreatable — the provider-facing recreate contract, the other half
# of the stage-4 token split (docs/grain-abi-v2.md graveyard; the old
# `Persistent.save` overloaded its token as "recreate key when a
# provider answers").
#
# A provider serves this on a sub-object cap it vends when that object
# is rebuildable: `key()` returns the opaque bytes its
# `Restorable.restore(objectId)` will rebuild from, the wakeup policy
# it wants for deliveries through the eventual token, and an advisory
# display name. The MEMBRANE is the caller — its eager crossing probe
# asks every cap crossing out of a provider; a cap that answers
# unimplemented is honestly session-bound (a sink, a cursor) and stays
# ephemeral. Holders never see this interface: their durability verb
# is `cap.v1`'s save(), which always returns a real platform token.
interface Recreatable {
  key @0 () -> (objectId :Data, policy :P.WakeupPolicy, name :Text);
}
