@0x9a7c3e5d1b8f6042;
# self_notify.v1 — a generic "mint a durable self-callback" tool an agent
# grain vends to its OWN LLM.
#
# An LLM can't author a capnp capability, so to subscribe to anything
# (outline.watch, schedule, a build hook) it needs a tool that mints the
# callback for it. `notifyMe(memo, wake)` returns a Trigger the model then
# hands — as a cap-ref tool argument — to any sink that takes a Trigger. When
# that sink fires, the agent grain is re-entered with `memo` (the agent's own
# instruction, baked in at mint time) plus the sink's payload.
#
# This is deliberately app-agnostic: it names no sink. The cap-handoff to a
# concrete sink (`outline.watch(notifyMe = cap://N, …)`) rides the agent's
# existing cap-ref tool-argument resolution. See
# platformd-rs/docs/durable-held-caps.md (the agent is the Sink; the memo is
# the "irreducible residue" the platform can't invent).

using Trigger = import "trigger-v1.capnp".Trigger;

interface SelfNotify {
  # Mint a durable callback into THIS agent: when a sink fires the
  # returned Trigger, the agent is re-entered with `memo` (your
  # instruction to replay) plus the sink's payload. Pass the Trigger to
  # any sink that takes one.
  #
  # `memo` example: "re-summarize the roadmap and post to #eng".
  # `wake` is the agent's disruptability, lowered to a WakeupPolicy
  # by the grain — one of:
  #   "immediate" | "debounce:<ms>" | "digest:<ms>" | "piggyback"
  # The agent implements restorable.v1 so the platform can rebuild the
  # callback on wake.
  notifyMe @0 (memo :Text, wake :Text) -> (callback :Trigger);
}
