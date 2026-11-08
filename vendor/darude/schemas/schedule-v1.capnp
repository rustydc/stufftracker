@0x90eef5e9b76d65fb;
# schedule.v1 — time-based and on-demand trigger firing.
#
# Built on the saved-Trigger token: a `token` here is a SturdyRef
# (saver.capnp) for a `trigger.v1.Trigger` sub-object on some target
# grain. Firing restores it (waking the target) and calls
# `trigger(payload)`. See trigger-v1.capnp for the payload type.

using Trigger = import "trigger-v1.capnp";

# Deliver — the single trigger-firing primitive. Restore `token` to a
# Trigger, wake the target grain, call `trigger(payload)`. Used by
# source grains for event delivery (the grain holds its subscribers'
# tokens and calls deliver() when its object changes) AND internally by
# the scheduler. The platform never learns what the token targets or why
# it fired — it only sees opaque bytes and a payload.
interface Deliver {
  # Fire a saved Trigger: restore `token` (waking the target grain if
  # needed) and call `trigger(payload)` through it. `delivered = false`
  # means the token is dead — the caller should drop it from its list.
  #
  # delivered = false  => token unknown, its parent grant was severed,
  # or the target couldn't reconstruct the sub-object. Fire-and-forget
  # otherwise (the bool is an ack that the call was dispatched, not
  # that the work completed).
  deliver @0 (token :Data, payload :Trigger.TriggerValue) -> (delivered :Bool);
}

# Schedule — the clock feature, and the only platform-stored trigger
# state (one `scheduled_triggers` row per call). The supervisor's
# schedule sweep fires due rows through the same path as `Deliver`.
#
# `payload` must be data-only: a live cap can't be durably stored, so
# the `cap` arm of TriggerValue is rejected at schedule time.
interface Schedule {
  # One-shot at an absolute wall-clock time (epoch millis).
  at     @0 (epochMillis  :Int64,  token :Data, payload :Trigger.TriggerValue) -> (id :Data);
  # One-shot after a relative delay.
  after  @1 (delayMillis  :UInt64, token :Data, payload :Trigger.TriggerValue) -> (id :Data);
  # Repeating every `periodMillis`; the row re-arms after each fire.
  every  @2 (periodMillis :UInt64, token :Data, payload :Trigger.TriggerValue) -> (id :Data);
  # Cancel a previously-scheduled entry. `id` is the opaque handle the
  # scheduling call returned; cancel is scoped to the owner_subject that
  # created the row, so a holder can only cancel its own schedules.
  cancel @3 (id :Data) -> ();
}
