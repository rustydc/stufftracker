@0x90a6d8048746a3ac;
# observer.v1 — the "observation is a capability" primitive (docs/audit.md).
#
# A grant may carry an Observer. The caretaker minted for that grant then
# reports each exercise of the grant to the Observer: post-call `fire` for
# recording/notifying, and (later) pre-call `gate` for enforcement. Because
# the Observer is passed in at grant time it is chained under the grant —
# severable with it, attributed to it, attenuable by its delegates — so
# audit/metering/notification/enforcement all become ordinary, revocable
# delegations rather than platform subsystems.
#
# An Observer can be save()d for a durable token — via the membrane's
# `cap.v1` save, not a mixin here — and delivered to across the
# observer grain's hibernation (see docs/audit.md "Hibernation" —
# durability is a property of *saving* the Observer, not of this
# interface).

# An inert, self-describing record of ONE capability exercise.
#
# Inert BY CONSTRUCTION: an Exercise never carries a live capability. Caps
# that appeared in the observed call's params/results are rendered as
# `CapRef` placeholders (interface name only), so observing an exercise can
# never hand the observer the authority it was only meant to watch. "Record
# params/responses" (docs/audit.md content capture) means a richer
# descriptor here — never live references.
struct Exercise {
  # The exercised caretaker's own capability id — the grant being used.
  # Kept distinct from `actor` (the acting persona) so the record
  # separates *who acted* from *whose authority they used*.
  #
  # The acts-as split the embodiment model makes and the old flat audit
  # row collapsed. `subject` is the holder the caretaker is bound to.
  authorityCapId @0 :Text;
  actor          @1 :Text;
  subject        @2 :Text;

  interface  @3 :Text;
  method     @4 :UInt16;
  methodName @5 :Text;   # resolved name if known, else empty.

  phase @6 :Phase;
  error @7 :Text;        # empty unless phase = failed.

  # Two-phase timing (docs/audit.md): `invoked` is stamped before the
  # forward; a completion (`ok`/`failed`) carries `endedAtMillis`. An
  # `invoked` never followed by a completion is a call in flight or lost.
  startedAtMillis @8 :Int64;
  endedAtMillis   @9 :Int64;   # 0 while in flight (phase = invoked).

  # Caps that crossed in the observed message, as inert placeholders only.
  caps @10 :List(CapRef);

  # Redacted JSON of the observed call's params — opt-in content
  # capture (docs/audit.md), empty unless the grant's observer was
  # attached with `capture`.
  #
  # `resultsJson` likewise carries the results on a successful forward.
  # Any capability is rendered as a "«cap»" placeholder string, never a
  # live reference — so capture enriches the descriptor without ever
  # making it non-inert.
  paramsJson  @11 :Text;
  resultsJson @12 :Text;

  enum Phase {
    invoked @0;   # write-ahead, before the forward.
    ok      @1;
    failed  @2;
    revoked @3;   # the cap was severed; the call was refused, not forwarded.
  }

  struct CapRef {
    # The placeholder's declared interface, if the schema names it; empty
    # otherwise. Deliberately carries no handle — a CapRef is not vendable.
    interface @0 :Text;
  }
}

interface Observer {
  # Post-call, async, fire-and-forget. `events` is a batch — coalesced
  # delivery, mirroring trigger.v1 — so a busy grant costs one re-entry per
  # flush, not one per call. Returns nothing: observation, not a request.
  fire @0 (events :List(Exercise)) -> ();

  # Pre-call veto, awaited: return `allow = false` to refuse the
  # observed call before it is forwarded (`reason` surfaces to the
  # caller as the failure text). An Observer with no gate policy
  # returns `allow = true`.
  #
  # This is the enforcement half (the llm-enforcer's pre-call gate,
  # generalized); it puts the observer grain in the synchronous call
  # path, so it is opt-in and lands after `fire` (docs/audit.md
  # migration).
  gate @1 (event :Exercise) -> (allow :Bool, reason :Text);
}
