@0xc4f1a3d5e7b9c2f4;
# Shared change-subscription primitives. Both wave.v1 and rustpad.v1
# expose a `listen()` method that takes a ChangeListener cap and
# returns a Subscription handle; the LLM agent (or any consumer)
# can hold the same ChangeListener and pass it to either backend.
#
# Generic text descriptions only — implementations decide what to
# say ("blip appended by alice", "12 chars inserted at offset 42").
# Structured events live on per-backend interfaces (e.g.
# wave.v1.subscribe() still vends typed ChangeEvent payloads).

interface ChangeListener {
  # Called once per logical change at the source, with a human-readable
  # `description` of the change.
  #
  # Concurrent calls are serialised by capnp-rpc (FIFO per cap), so the
  # listener sees changes in the order the source emitted them. The
  # bridge does NOT retry on call failure — if the listener's cap is
  # severed or unreachable, the next event causes the source to drop
  # the subscription.
  changed @0 (description :Text) -> ();
}

# Opaque handle returned by listen(). Holding it keeps the
# subscription alive; dropping it (or severing via broker) tears
# the subscription down. No methods — its lifetime IS the
# subscription's lifetime.
interface Subscription {}
