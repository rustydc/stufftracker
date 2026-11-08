@0xb4e2a7c9d1f80356;
# trigger.v1 — a generic, cap-bearing notification / action-request
# primitive. One method, fire-and-forget, carrying an open-ended
# JSON-like payload.
#
# Motivation:
#
#   A lot of cross-app plumbing reduces to "tell that thing something
#   happened, here's some metadata, and maybe here's a cap it can use".
#   Launching a script, asking an agent to do something, a plain
#   notification between grains — they all want a structured-but-open
#   payload rather than a bespoke interface per case.
#
#   capnp ships `Json.Value` (capnp/compat/json.capnp) which is the
#   moral equivalent of protobuf's `Struct`, but it has two problems
#   here: it can't carry a capability, and it collapses every number
#   to Float64. `TriggerValue` is the same idea with neither flaw —
#   it mirrors the neutral `Value` tree in darude-runtime's introspect
#   module (Null/Bool/Int/UInt/Float/Text/Data/List/Struct/Cap), so it
#   round-trips through the existing marshalling for free.
#
# Lowering:
#
#   The whole point of the `cap` arm being an OpaqueCap (not a bare
#   `Capability`) is that the receiver can lower a TriggerValue to JSON
#   for an agent or a Monty script with the machinery that already
#   exists for OpaqueCap tool arguments: introspect the interface from
#   the inline (cached) schema bytes, register the cap as a `cap://N`
#   handle, and hand the model/script a plain JSON value with cap-refs
#   where the caps were. No new lowering path, no central registry.
#
# Callbacks:
#
#   trigger() returns nothing. A caller who wants a reply passes a cap
#   in the payload (e.g. a one-shot callback, or a ChangeListener) and
#   lets the receiver call back through it. Keeping the method void
#   means notify/launch/ask all share one signature and one schema.

using Intro = import "introspectable.capnp";

# A neutral, self-describing value: JSON-like, but able to carry live
# capabilities and to preserve integer width. Mirrors the runtime
# `Value` enum so conversion is mechanical.
struct TriggerValue {
  union {
    # Null, or an explicitly-empty value.
    null    @0 :Void;
    bool    @1 :Bool;
    # Signed and unsigned kept distinct (unlike Json.Value's single
    # Float64 `number`) so Int64 ids / epoch-millis survive the round
    # trip without precision loss.
    int     @2 :Int64;
    uint    @3 :UInt64;
    float   @4 :Float64;
    text    @5 :Text;
    data    @6 :Data;
    list    @7 :List(TriggerValue);
    # A JSON-object-shaped value: ordered (name, value) pairs. Ordered
    # (not a map) to match capnp's own Json.Value convention and the
    # `Value::Struct(Vec<(String, Value)>)` it lowers to.
    object  @8 :List(Field);
    # A live capability, packaged self-describingly (as an OpaqueCap)
    # so the receiver can introspect and lower it without knowing the
    # interface at compile time.
    #
    # The arm Json.Value / protobuf Struct can't express. See the
    # OpaqueCap protocol in introspectable.capnp (schema bytes are
    # required on first send per connection/direction, cacheable after).
    cap     @9 :Intro.OpaqueCap;
  }

  struct Field {
    name    @0 :Text;
    value   @1 :TriggerValue;
  }
}

# A Trigger can be saved across hibernation through the membrane: a
# holder that receives one calls `cap.v1` save() for a durable token,
# and the crossing probe asks the Trigger's provider for its recreate
# key via `Recreatable.key` (docs/durable-held-caps.md). A provider
# that doesn't serve Recreatable is honestly session-bound and its
# triggers are only ever fired live.
interface Trigger {
  # Fire-and-forget notification: deliver a batch of pokes (`payloads`)
  # to this trigger's receiver in one call. A live single fire is just a
  # 1-element list. Returns nothing.
  #
  # BATCH-capable: the platform coalesces a Sink's pending pokes
  # (across all the watches sharing an inbox) and hands them over
  # together, so the receiver does one re-entry for the whole batch
  # rather than one per poke (docs/durable-held-caps.md — "batch the
  # poke summaries"). Each element is the open-ended metadata for one
  # poke (a cursor, a change summary); the receiver drains specifics
  # via the source's own `changesSince`.
  #
  # A notification, not a request/response: a caller wanting a result
  # puts a callback cap in a payload and waits for the receiver to call
  # back. capnp-rpc serialises concurrent calls FIFO per cap, so a
  # receiver sees batches in send order.
  trigger @0 (payloads :List(TriggerValue)) -> ();
}
