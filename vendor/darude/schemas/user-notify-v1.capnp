@0xd8b3f61c29a47e05;
# UserNotify — the "attention" platform capability: let a grain ping
# its OWNER's devices (Web Push today) with a short, human-readable
# message. The counterpart to powerbox-request.capnp on the altitude
# ladder: powerbox asks for authority over the owner's RESOURCES
# (consent-mediated); notify asks only for the owner's ATTENTION, so it
# is manifest-declared and launch-granted like the other platform caps
# (spawn, schedule, shelf, powerbox_request) — see
# rpc::is_platform_synth and install::grant_user_notify_caps.
#
# Consent layers, in order of authority:
#   1. Device level: nothing reaches a phone unless the owner enabled
#      notifications there (the shell PWA's subscribe flow). No
#      subscriptions ⇒ notify() succeeds with delivered = 0.
#   2. Per-grain sever: the grant is its own parentless row — cutting
#      it in the capabilities tab mutes exactly this grain.
#   3. A per-row rate limit platform-side, so a looping agent can't
#      buzz a phone unbounded. The limit keys on the AUTHORITY ROW, so
#      a script the grain forwards the cap into (code-exec tools=[...])
#      draws from the same budget.
#
# For an LLM agent the cap lowers into an ordinary `notify` tool via
# the standard schema introspection — "tell me when the build is done"
# needs no new agent machinery.

interface UserNotifyV1 {
  # Ping the grain owner's subscribed devices with a notification.
  # `title`/`body` show verbatim; same-`tag` notifications REPLACE each
  # other on the device. Returns how many devices accepted — 0 is
  # success (no subscribed device), so do NOT retry on 0.
  #
  # Keep `title`/`body` short (the platform clips defensively). `tag`
  # is the coalescing key — same-tag notifications replace rather than
  # stack, so a progress-updating agent should reuse one tag. Empty tag
  # = a per-grain default (still coalesced per grain, not unbounded).
  # Errors: severed cap, rate limit exceeded, or a grain with no
  # resolvable owner (platform-owned grains).
  notify @0 (title :Text, body :Text, tag :Text) -> (delivered :UInt32);

  # Retract: close the notification carrying `tag` on ALL of the
  # owner's devices — the cross-device acknowledgement. Call on real
  # acknowledgement (the owner viewed the thing), not speculatively.
  # Empty tag = the per-grain default tag notify() uses.
  #
  # "Read it on the phone, the tablet goes quiet." Delivered as a
  # silent push the service worker turns into close-by-tag; browsers
  # budget silent pushes — hence not speculative. Draws from the same
  # rate budget and honors the same sever/owner gates as notify().
  dismiss @1 (tag :Text) -> ();
}
