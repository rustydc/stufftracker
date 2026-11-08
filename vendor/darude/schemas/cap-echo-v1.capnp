@0xe3a7c1d92f6b8045;
# cap-echo.v1 — a tiny test grain whose only method takes a CAPABILITY
# parameter. It exists to exercise the console's cap-param delegation
# end to end without any external/network dependency: the console form
# renders `echo`'s `llm` param as a grain picker, the user picks an
# llm.chat.v1 grain, and the platform resolves + binds that cap into the
# call. cap-echo then calls chat() through the passed cap and returns the
# text — so a successful round trip proves the whole delegation chain.

using Llm = import "llm-chat-v1.capnp";

interface CapEcho {
  # Call `llm.chat({user: prompt})` through the passed-in cap and return
  # the response text. `llm` is a typed-cap param — the thing under test.
  echo @0 (llm :Llm.LlmChatV1, prompt :Text) -> (text :Text);

  # Hand the passed-in cap straight back. Exercises the console's
  # RETURNED-cap path: the result carries a live llm.chat.v1 cap, which
  # the console stashes as a re-usable ref and can feed into echo's `llm`.
  passthru @1 (llm :Llm.LlmChatV1) -> (llm :Llm.LlmChatV1);

  # No-cap smoke method: echo the message back.
  #
  # Exists so a test can drive ONE tool dispatch through a delegated
  # cap-echo caretaker (audit row, sever check) without needing a
  # second cap to pass in — the subagent e2e's "child calls its
  # delegated tool" probe.
  ping @3 (msg :Text) -> (msg :Text);

  # Hand a whole LIST of caps straight back.
  #
  # The only schema that carries a bare `List(Capability)` across a
  # method boundary — it exists to exercise the membrane's per-element
  # re-wrapping of cap lists (every returned element must itself be a
  # caretaker: audited and severable).
  passthruMany @2 (llms :List(Llm.LlmChatV1)) -> (llms :List(Llm.LlmChatV1));

  # List the labels on the shelf cap this grain received at handshake
  # (its owner's home, `requires: home: shelf.v1`). Errors if the
  # grain was launched without the slot granted.
  #
  # Exercises the grain-side shelf.v1 delivery end to end: platformd
  # resolves the shelf grant into a platform-served ShelfImpl, the
  # grain calls list() through it, and the labels ride back. (Fail
  # loudly, not empty.)
  shelfLabels @4 () -> (labels :List(Text));

  # Walk the shelf for the factory entry whose appId matches, then call
  # factory.spawn(name) through the entry's cap. Returns the spawned
  # instance's id.
  #
  # Exercises the whole capnp factory path from inside a grain: the
  # platform-served Factory rides back inside a shelf Entry, and
  # spawning through it lands an instance owned by this grain's OWNER
  # (the shelf's user) — the agent-instantiates-an-app flow.
  shelfSpawn @5 (appId :Text, name :Text) -> (instanceId :Text);

  # File a powerbox consent request for `interface` through the shelf's
  # owner (via the powerbox_request.v1 cap delivered at handshake).
  # Returns the request id.
  #
  # Exercises the grain-side ASK path — the owner then approves in the
  # shell (or the e2e does via HTTP), and the granted cap lands on this
  # grain's accept_cap.v1 channel below.
  requestCap @6 (interface :Text, reason :Text, label :Text) -> (requestId :Text);

  # How many caps this grain has received on its accept_cap.v1 channel
  # (via approved powerbox requests or user-driven offers). Lets the
  # e2e assert delivery landed after approval, and that a denied
  # request delivers nothing.
  receivedCount @7 () -> (count :UInt32);
}
