@0xc22545fc1d7dd781;
# web.fetch.v1 — a single-method "fetch a URL" capability the user
# can offer to an LLM agent as a tool.
#
# The agent (llm-openai-provider) lowers any offered cap's methods
# to model tool definitions via its runtime SchemaLoader — text
# params become string args, the result struct becomes the tool
# result. So this interface needs no special agent support: offer
# a web-fetch grain's cap to a chat, and the model gains a
# `webFetchV1_fetch` tool that returns page contents.
#
# Egress is the grain's, not the agent's: the web-fetch grain holds
# `requires: net: egress: true`. The agent reaches the open web
# only through this mediated cap — every fetch is a broker-audited
# call, and revoking the cap cuts the agent's web access without
# touching the chat or the model.

using Blob = import "blob.capnp".Blob;

interface WebFetchV1 {
  # HTTP(S) GET of a URL. Returns the response status + a text body,
  # truncated to keep large pages inside the model's context.
  #
  # A transport/decode failure comes back as status=0 + a non-empty
  # `error` (NOT a capnp error) so the agent gets a usable tool
  # result it can reason about rather than an exception.
  fetch @0 (req :FetchRequest) -> (resp :FetchResponse);

  struct FetchRequest {
    url @0 :Text;
    # Max body bytes to return. 0 = provider default (64 KiB).
    # Caps the tool result so one big page can't blow the context.
    maxBytes @1 :UInt32;
  }

  struct FetchResponse {
    status      @0 :UInt32;   # HTTP status; 0 if the request failed
    contentType @1 :Text;
    body        @2 :Text;     # decoded text body, possibly truncated
    error       @3 :Text;     # non-empty on transport/decode failure
    truncated   @4 :Bool;     # true if body was cut at maxBytes
    # Binary responses (image/*, application/pdf): `body` stays empty
    # and the payload rides here as the shared Blob, which the agent
    # loop shows to the model as a vision/document block.
    #
    # (See blob.capnp.) Binary responses aren't truncated — one that
    # exceeds the size cap errors in-band instead (a cut image is
    # garbage). That cap is the agent loop's own blob ceiling (4 MiB),
    # not the text `maxBytes` one: below it the loop can show the
    # payload to the model, above it the loop refuses regardless, so
    # the grain declines exactly what was never viewable anyway.
    # `maxBytes` doesn't apply here — it bounds context, and a blob
    # costs context only if the model views it.
    blob        @5 :Blob;
  }
}
