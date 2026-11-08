@0xc4f1a8e93d27b650;
# agent_notes.v1 — the agent's own pinned-notes tool (docs/
# grain-abi-v2.md, "Agents under v2": the tool-driven memory write,
# landed on the platform's most durable object). An embodied grain
# lowers this in-process, backed by its Principal's profile — the
# model never sees the Principal API, only this small tool.

interface AgentNotes {
  # Read the current pinned notes (and the owner-set persona, for
  # context — the persona is read-only from here).
  read @0 () -> (persona :Text, notes :Text);

  # Replace the pinned notes wholesale. Keep them BRIEF — a few lines
  # of durable self-configuration ("Humor setting: 75%"), not a
  # journal; the platform caps the field at a few KB.
  write @1 (notes :Text) -> ();
}
