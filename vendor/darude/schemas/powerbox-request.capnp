@0xc4f2a8d95e7b3610;
# PowerboxRequest — the grain-side door of the consent flow
# (docs/powerbox.md). The dual of accept-cap.capnp: AcceptCap is how a
# cap the USER initiates reaches a grain; PowerboxRequest is how a
# grain ASKS for one it was never wired with.
#
# The flow is deliberately asynchronous — a human decides:
#
#   1. The grain calls request("llm.chat.v1", reason, label, slot).
#      The platform records a pending request and returns its id.
#      Nothing is granted; the call never blocks on consent.
#   2. The grain's OWNER sees the request in the shell (SSE-pushed),
#      picks a source grain they hold that provides the interface,
#      optionally attenuates rights, and approves — or denies.
#   3. Approval rides the SAME machinery as a user-driven powerbox
#      offer: a broker grant chained under the approver's own access
#      to the source (revoking their access severs the introduction,
#      transitively), caretaker-wrapped, delivered to the grain's
#      accept_cap.v1 channel — or bound into the named `many:` requires
#      slot when the request carries one. Delivery replays on every
#      grain restart like any other offer.
#
# The requesting grain is untrusted: it names an interface, it never
# names a source. What it receives is always a broker-minted caretaker
# — audited, severable, attenuated to whatever the approver picked.

interface PowerboxRequest {
  # File a consent request for a capability: name an `interfaceName`
  # (e.g. "llm.chat.v1") and a `reason`; the grain's owner later
  # approves or denies in the shell. Returns a pending `requestId` —
  # nothing is granted now, the call never blocks on consent.
  #
  # `interfaceName` is the platform interface string (manifest
  # vocabulary). `reason` is shown to the owner verbatim — the grain
  # making its case. `label` names the delivery (the accept channel
  # dedups by label; agents lower it to a tool-name prefix). `slot`
  # optionally names one of the grain's own `many:` requires slots to
  # bind into instead of the accept channel; empty = accept channel.
  # Re-requesting while an identical request is pending returns the
  # existing id (idempotent).
  request @0 (interfaceName :Text, reason :Text, label :Text, slot :Text)
          -> (requestId :Text);
}
