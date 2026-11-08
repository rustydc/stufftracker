@0xd1a9b3e7f4c821a5;
# AcceptCap — receiver-side of the powerbox-offer flow.
#
# A grain that wants to receive caps from the user advertises
# `provides: accept_cap.v1` in its manifest, alongside its own
# main interface. The platform's "powerbox" UI flow (driven by
# the receiver's frontend — e.g. the chat app's "+ add grain"
# button) is just a thin HTTP endpoint:
#
#   1. The user, while in receiver R's UI, picks a grain G from
#      their grain list to introduce.
#   2. R's frontend POSTs `/api/powerbox/offer { target: R, source: G }`
#      to platformd.
#   3. platformd looks up G's primary provided interface from G's
#      manifest, pulls G's cap from the providers registry,
#      packages it as an OpaqueCap (with inline CGR bytes from
#      darude_wire::schema_for_interface), records a broker grant
#      for audit, and calls `R.acceptCap.accept(opaqueCap)`.
#
# The receiver never names an interface upfront. It just accepts
# whatever the user offers. The OpaqueCap's inline schema lets
# the receiver introspect what kind of cap arrived at runtime
# (e.g. lower it to LLM tools as in the agent loop).

using Intro = import "introspectable.capnp";

interface AcceptCap {
  # Receive a cap the user is introducing to this grain, delivered as
  # a self-describing OpaqueCap. The implementor typically stashes it
  # somewhere that subsequent operations can consult.
  #
  # For chat-style apps holding an LLM tool surface, that's a
  # per-conversation `Vec<OpaqueCap>` consulted on the next agent()
  # call.
  accept @0 (cap :Intro.OpaqueCap) -> ();
}
