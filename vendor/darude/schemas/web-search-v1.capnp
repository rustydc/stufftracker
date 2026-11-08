@0xf24e90ef5ba16d0a;
# web.search.v1 — a "search the web" capability the user offers to
# an LLM agent. Provider-agnostic by design: the interface is just
# query -> results; the backing grain decides the engine (Brave
# today; a SearXNG compose-grain or Tavily could drop in behind the
# same interface, like llm.chat.v1's swappable providers).
#
# Composes with web.fetch.v1: search returns titles/urls/snippets,
# the agent picks interesting urls and calls fetch() to read them.
# Two small tools instead of one fat one.
#
# Egress + any API key live in the search grain, not the agent. The
# model reaches a search engine only through this mediated, broker-
# audited cap; revoke it and the agent loses search without
# touching the chat.

interface WebSearchV1 {
  # Run a web search. Failures come back in-band (empty results +
  # non-empty `error`), not as capnp errors, so the agent gets a
  # tool result it can reason about rather than an exception.
  search @0 (req :SearchRequest) -> (resp :SearchResponse);

  struct SearchRequest {
    query @0 :Text;
    # Max results to return. 0 = provider default (10). Capped by
    # the provider to keep tool results inside the model context.
    count @1 :UInt32;
  }

  struct SearchResponse {
    results @0 :List(Result);
    error   @1 :Text;   # non-empty on transport/auth/decode failure
  }

  struct Result {
    title   @0 :Text;
    url     @1 :Text;
    snippet @2 :Text;
  }
}
