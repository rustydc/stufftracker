@0xc4f8a2d17e9b6031;
# subagent.v1 — drive a chat grain as a subagent over capnp.
#
# A chat grain (basic-chat) provides this alongside its HTTP UI so a
# PARENT agent can spawn a copy of it and assign it work. The parent
# holds the returned spawn caretaker; run() executes one agentic task
# against the child's own llm require and tool stash — both of which
# the parent delegated at spawn time (spawn.capnp's SpawnGrant), so
# the child's whole authority is a severable, audited subtree of the
# parent's.
#
# The task and its transcript are persisted as an ordinary
# conversation in the child grain, so a human can open the child's UI
# and watch (or audit) the subagent's work after the fact.

using Blob = import "blob.capnp".Blob;
using Trigger = import "trigger-v1.capnp".Trigger;

interface SubagentV1 {
  # Run one task to completion through the child's agent loop and
  # return the final text. A loop failure comes back in `error` with
  # empty `result`.
  #
  # `model` is passed through to the llm require (empty = upstream
  # default). `steps` counts the loop's recorded steps (model turns
  # + tool dispatches) for the parent's trace.
  #
  # `conversationId` selects the thread the task runs in. Empty =
  # a fresh conversation (one run() = one thread in the child's UI).
  # Passing a prior run's returned `conversationId` appends the task
  # to that thread instead, so a re-driven child keeps its context —
  # the follow-up run sees everything the earlier runs saw. An
  # unknown id fails the call rather than silently starting over.
  #
  # `blobs` are binary payloads (images, PDFs, files) attached to
  # the task turn, exactly like attachments on a user message: the
  # child's model sees them as parts and can pass them onward to its
  # own Blob-taking tools.
  run @0 (task :Text, model :Text, conversationId :Text, blobs :List(Blob))
      -> (result :Text, error :Text, steps :UInt32, conversationId :Text);

  # The DETACHED sibling of run(): start the task and return
  # immediately with a `runId` + the conversation it runs in; on
  # completion the child fires `done` with a
  # `{runId, conversationId, ok, result|error}` payload.
  #
  # This is the "detachable operation" shape
  # (docs/tool-calling-conventions.md).
  #
  # `done` must be a durable (persistable) Trigger — the child save()s
  # it before accepting the task, so the completion survives both
  # grains' hibernation: the fire routes through the platform's saved-
  # cap delivery, waking the parent if it's asleep. A parent agent
  # mints one with its self_notify tool and passes it here as a
  # cap-ref argument. A trigger whose save() fails fails the call —
  # a detached run with a completion ping that can't outlive a nap
  # is a silent-loss bug, not a degraded mode.
  #
  # task/model/conversationId/blobs mean exactly what they do on
  # run(). The task turn (and its conversation, when fresh) is
  # committed before this returns, so the returned conversationId is
  # immediately valid for watching the child's UI or a follow-up run.
  start @1 (task :Text, model :Text, conversationId :Text, blobs :List(Blob),
            done :Trigger)
        -> (runId :Text, conversationId :Text);
}

# subagent_mgr.v1 — the PARENT-side tool surface.
#
# The chat grain vends this per-turn as an in-process tool cap (the
# same pattern as self_notify), so its model can spawn attenuated
# subagents. spawn() is one blocking round trip: spawn the sibling
# chat grain, delegate the selected tools + the llm backend to it,
# run the task, return the child's final text. The grain implementing
# this holds the spawn.v1 cap; the model never touches spawn.v1
# directly.

interface SubagentMgr {
  # Spawn an attenuated subagent: create a sibling chat grain, hand
  # it the selected `tools` plus the parent's llm backend, run
  # `task` to completion, and return the child's final text.
  #
  # `tools` selects, by handle/label, which of the parent's stashed
  # caps the child receives (an unknown handle fails the call —
  # never silently under-provision). `name` labels the child
  # instance in the shell. The child does NOT receive the parent's
  # conversation, and it can only recurse if the platform granted
  # it a spawner of its own.
  #
  # `budgetTokens`, when non-zero, gives the child its OWN token
  # budget: the parent spawns a fresh llm-enforcer for the child
  # (chained under the parent's own llm cap) and points the child's
  # llm at it, so the child runs `child chat → child enforcer(budget)
  # → parent enforcer → provider`. The child cannot exceed its budget
  # nor, through it, blow the parent's. Requires the parent's package
  # to contain an `enforcer` grain (the agent package does); zero =
  # share the parent's enforcer, as before (docs/subagents.md).
  #
  # `keep`, when true, makes the child LONG-LIVED: the platform
  # offers the child's subagent.v1 back to the parent as a durable
  # stash cap labeled `name`, so after this call returns the parent's
  # model holds a `<name>_run` tool it can call to assign follow-up
  # tasks. The stash row survives both grains' hibernation (it rides
  # the same replay-on-ready as any powerbox cap, and calls through
  # it wake the child), and severing it — or anything above it —
  # cuts the child loose. keep=false is the one-shot behavior: the
  # child instance persists for audit but the parent keeps no handle.
  #
  # `blobs` are attachments handed to the child's first task turn
  # (see SubagentV1.run).
  spawn @0 (task :Text, tools :List(Text), name :Text, budgetTokens :UInt64,
            keep :Bool, blobs :List(Blob))
        -> (result :Text, instanceId :Text, error :Text, steps :UInt32);

  # Durably delegate one of the parent's stashed tools to another
  # grain by handle — canonically a kept subagent the parent
  # spawned. The recipient gets its OWN durable hold on the tool,
  # kept across its hibernation.
  #
  # Unlike spawn()'s `tools` (a LIVE cap handed to the child at
  # birth, alive only while the parent's session is), the parent
  # save()s the named tool to a SturdyRef and calls delegate.v1, so
  # the child re-vends the tool at its own handshake and keeps it
  # across its own hibernation. The recipient's hold chains under
  # the parent's, so severing the parent (or the tool) cascades, and
  # the child's calls audit under its own row.
  #
  # `tool` is a stash handle (exact or sanitized, like spawn's `tools`);
  # `recipient` is a grain instance id (e.g. an instanceId a keep-spawn
  # returned). `rights` attenuates (empty = inherit the tool's rights).
  # The parent must hold a delegate.v1 cap (the `delegate` require);
  # errors (in `error`, ok=false) if the handle is unknown, the tool
  # can't be saved, or delegation is refused.
  delegate @1 (tool :Text, recipient :Text, rights :List(Text))
           -> (ok :Bool, error :Text);
}
