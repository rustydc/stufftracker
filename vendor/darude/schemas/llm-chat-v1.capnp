@0xc8a7f43e1b2d4a02;
# llm.chat.v1 — direct port of packages/runtime/src/interfaces/llm-chat-v1.ts.
#
# Wire signature only. Constraint schema (model allowlist, budget,
# rate limits) is private to whichever component enforces them.

using Intro = import "introspectable.capnp";
using TextWriter = import "text-writer.capnp".TextWriter;
using Blob = import "blob.capnp".Blob;

# Durability and delegation-tagging are membrane surfaces, not part of
# this interface: any caretaker-wrapped cap answers `cap.v1`'s
# save/forget (a SturdyRef token) and attenuate (rights subset + tag)
# — docs/grain-abi-v2.md "Cap". The old extends(Persistent) mixin and
# Taggable.withTag are graveyard (stage 4).
interface LlmChatV1 {
  chat @0 (req :Request) -> (resp :Response);

  # Agentic call: a prompt plus a bag of capabilities the model may
  # use as tools; the provider loops, dispatching each tool_call
  # through the corresponding cap, until the model returns a final
  # text response (or hits `maxSteps`).
  #
  # Mechanics: the provider resolves each OpaqueCap's schema (either
  # inline or from the per-connection SchemaCache), lowers each
  # interface to a tool-use definition (capnp interface methods →
  # OpenAI/Anthropic tool list), passes them to the model, and
  # dispatches every tool_call dynamically through the corresponding
  # cap.
  #
  # Identity for cap calls: the agent runs UNDER the caller's cap
  # — i.e. the LLM never sees the user, but every dispatched
  # tool_call reaches the underlying caretaker tagged with whatever
  # display_tag the agent cap was minted under. So an agent cap
  # minted as `withTag("alice [agent]")` and given a wave cap
  # bound to participant `alice@local.net` produces audit rows
  # attributing the writes to alice [agent].
  agent @1 (req :AgentRequest) -> (resp :AgentResponse);

  # Streaming variant of agent(): same tool loop, but final-text
  # chunks are emitted to `sink` as they're generated; the full
  # AgentResponse is still returned at the end.
  #
  # Each generated chunk appears as one TextWriter.append. The
  # returned AgentResponse carries the full text, step trace, and
  # token totals so callers can audit + display the final state.
  # Tool dispatch happens internally; tool calls and their results
  # are NOT streamed to the sink (they show up in resp.steps).
  #
  # Why share TextWriter with wave.v1 + rustpad.v1: the chat app
  # passes whatever sink it wants — typically its own HTTP-SSE
  # sink, but in principle could be a wave's blip writer (LLM
  # output streams straight into a wave blip) or a pad writer
  # (output streams into a pad). No new schema needed for those.
  #
  # `events` is optional. When the caller passes it, the provider
  # emits one `started` ToolEvent as each tool call is dispatched
  # and one `finished` ToolEvent when that call completes, so a UI
  # can show live per-call status while the loop runs. Tool calls
  # within one model turn may be dispatched concurrently, so events
  # from different calls can interleave — correlate on callId.
  agentStream @2 (req :AgentRequest, sink :TextWriter, events :AgentEventSink) -> (resp :AgentResponse);

  # List the models this endpoint can serve. The returned `id` is
  # what a caller puts in Request.model / AgentRequest.model to
  # select that model.
  #
  # A leaf provider returns its own catalog; an enforcer forwards its
  # upstream's; a router aggregates the union across its configured
  # providers (stamping each with the originating provider, so a UI
  # can group by it). Optional — implementers that don't override it
  # return the default "unimplemented", which callers treat as "no
  # catalog" (empty list).
  listModels @3 () -> (models :List(ModelInfo));

  struct ModelInfo {
    id          @0 :Text;  # value for Request.model / AgentRequest.model
    displayName @1 :Text;  # human-facing label
    provider    @2 :Text;  # originating provider, e.g. "OpenAI" — group key
  }

  # One piece of a message's content. A message is an ordered list of
  # parts: text interleaved with binary attachments (the shared Blob —
  # see blob.capnp). Providers render each part into their vendor's
  # content-block shape: image/* becomes a vision block, application/pdf
  # a document block (OpenAI input_image/input_file), anything else is
  # degraded to an in-band note.
  #
  # Tool history rides as its own parts, so a caller replaying earlier
  # turns hands the model the calls it really made rather than a text
  # transcript of them: `toolCall` parts on an assistant message (the
  # calls that turn emitted, alongside any text it said first), and
  # `toolResult` parts on a `tool` message (one per call, answering it
  # by id). agent/agentStream render them as the vendor's native
  # tool-call blocks; chat(), which carries no tools, degrades them to
  # labeled text.
  struct Part {
    union {
      text       @0 :Text;
      blob       @1 :Blob;
      toolCall   @2 :ToolCall;
      toolResult @3 :ToolResult;
    }
  }

  struct ToolCall {
    id       @0 :Text;  # unique within the request; results answer it
    name     @1 :Text;  # the tool name the model called
    argsJson @2 :Text;  # the JSON arguments, verbatim
  }

  struct ToolResult {
    callId  @0 :Text;   # the ToolCall.id this answers
    content @1 :Text;   # what the tool returned (or its error), as text
  }

  struct Message {
    role  @0 :Role;
    parts @1 :List(Part);
  }

  enum Role {
    system    @0;
    user      @1;
    assistant @2;
    # Tool results (toolResult parts) answering the preceding
    # assistant message's toolCall parts.
    tool      @3;
  }

  struct Request {
    messages        @0 :List(Message);
    model           @1 :Text;     # empty = caller has no preference
    temperature     @2 :Float32;  # NaN = unset
    maxOutputTokens @3 :UInt32;   # 0 = unset
  }

  struct Response {
    text               @0 :Text;
    model              @1 :Text;
    inputTokens        @2 :UInt32;  # 0 if provider didn't report
    outputTokens       @3 :UInt32;
    providerResponseId @4 :Text;
  }

  struct AgentRequest {
    messages @0 :List(Message);
    # Capabilities the model may call as tools — each an OpaqueCap
    # the provider lowers to a tool definition. Caps come and go
    # per-request; the agent accumulates no state across calls.
    #
    # The provider walks each cap's schema, lowers it to a tool
    # definition, calls model.tools=[...]. To share state (e.g. an
    # open BlipWriter the agent should keep appending to), pass that
    # cap on every turn.
    tools    @1 :List(Intro.OpaqueCap);
    # Maximum tool-call iterations before the loop must terminate
    # with text or an error. 0 = provider default (probably 8 —
    # enough for "search → getWave → append" but bounded).
    maxSteps @2 :UInt32;
    # Optional model + temperature, same shape as Request.
    model           @3 :Text;
    temperature     @4 :Float32;
    # Per-model-turn output token cap. 0 = provider default.
    maxOutputTokens @5 :UInt32;
    # Pre-registered blobs: conversation attachments the caller
    # replays BY REFERENCE instead of re-inlining their bytes into
    # `messages` every turn.
    #
    # Each is registered under its `id` in the loop's blob registry,
    # so the model can `blob_view` it or pass the id into a
    # Blob-typed tool param. The caller states each id inline in the
    # message text where the attachment originally sat.
    blobs @6 :List(SeedBlob);

    struct SeedBlob {
      id   @0 :Text;   # e.g. "blob://h2-0" — any unique ref string
      blob @1 :Blob;
    }

    # Handles the model expanded on EARLIER turns of this
    # conversation (via the loop's `expand` tool); the provider
    # pre-expands these at loop start. Unknown handles are ignored.
    #
    # Pre-expanding keeps tool sets expanded for the conversation's
    # lifetime instead of re-collapsing behind `expand` every turn —
    # which both saves a round trip and keeps the rendered tool
    # context stable for prompt caching. The caller learns which
    # handles were expanded from the step trace (a toolResult step
    # with toolName "expand").
    expandedHandles @7 :List(Text);

    # Reasoning depth: "" (the default — reasoning models reason at
    # the vendor's own depth), "none" (explicitly off, best-effort),
    # or a tier "low" / "medium" / "high".
    #
    # The default means reasoning-capable models reason: providers
    # enable adaptive thinking where asking is free (Anthropic 4.6+,
    # unless the caller set a temperature — the knobs conflict there)
    # and defer to server defaults elsewhere (OpenAI reasoning models
    # are on by default). Models that would need a fixed thinking
    # budget (older Claude) stay off unless a tier is set. "none" maps
    # to the vendor's off switch where one exists and is ignored on
    # models that always reason (Fable, o-series). A tier maps to the
    # vendor's knob (Anthropic thinking + effort or budget_tokens;
    # OpenAI/OpenRouter reasoning.effort) and disables `temperature`,
    # which vendors reject alongside explicit reasoning. Relays
    # (enforcer, router) pass the field through untouched.
    reasoningEffort @8 :Text;
  }

  struct AgentResponse {
    # Final text returned by the model after all tool calls. Empty
    # if the model hit maxSteps or errored.
    text   @0 :Text;
    # Per-step accounting. Useful for cost analysis / debugging.
    steps  @1 :List(Step);
    # Set on early termination (model error, exceeded budget,
    # cap-dispatch error). Empty on clean text response.
    error  @2 :Text;
    # The model id the provider actually used (from its last model
    # turn) — provenance for the caller, who may have sent an empty
    # `model` and let a router pick. Empty when the provider doesn't
    # report one. Relays (enforcer, router) pass it through untouched.
    model  @3 :Text;
  }

  struct Step {
    # Either a model turn (text content, possibly with tool_calls)
    # or a tool result (one per dispatched tool_call). Recorded in
    # the order events happened.
    kind         @0 :StepKind;
    # For kind=modelTurn, the text emitted that turn (often empty
    # when the turn is purely tool_calls). For kind=toolResult,
    # the JSON-serialized return value the tool produced (or the
    # error message).
    text         @1 :Text;
    # For kind=toolResult, the tool name (e.g. "wave_search").
    # Empty otherwise.
    toolName     @2 :Text;
    # For kind=toolResult, the JSON-serialized arguments the model
    # passed. Empty otherwise.
    toolArgs     @3 :Text;
    inputTokens  @4 :UInt32;
    outputTokens @5 :UInt32;
    # For kind=toolResult: binary payloads the result carried (each is
    # named by a `blob://N` placeholder inside `text`). Carried so the
    # UI can show the user what the model saw — a generated image, a
    # photo thumbnail, a fetched PDF.
    blobs        @6 :List(Blob);
    # Prompt-cache read tokens for kind=modelTurn, normalized across
    # vendors: the portion of `inputTokens` served from cache. 0 when
    # the vendor reports no cache usage.
    #
    # Normalization (done by the adapters): `inputTokens` is the
    # TOTAL prompt the model processed (cache hits included),
    # `cacheReadTokens` the portion served from cache,
    # `cacheWriteTokens` the portion newly written to it — so
    # cacheRead + cacheWrite <= inputTokens always, and
    # (inputTokens - cacheRead) is the full-price input on any
    # vendor. Adapters translate: Anthropic reports cache traffic
    # OUTSIDE usage.input_tokens, so its adapter adds reads+writes
    # back in; OpenAI already reports inclusively (cached_tokens is a
    # subset; no write figure exists, so 0). Both 0 when the vendor
    # reports no cache usage.
    cacheReadTokens  @7 :UInt32;
    cacheWriteTokens @8 :UInt32;
  }

  enum StepKind {
    modelTurn  @0;
    toolResult @1;
  }

  # Live tool-call event stream for agentStream (see the `events`
  # param there). One `started` per dispatched tool call, one
  # `finished` when it completes. Purely advisory: the canonical
  # record is still AgentResponse.steps.
  interface AgentEventSink {
    toolEvent @0 (ev :ToolEvent) -> ();
  }

  enum ToolEventPhase {
    started  @0;
    finished @1;
  }

  struct ToolEvent {
    # The model's stable tool_call id — the key a UI correlates the
    # started/finished pair on (calls can run concurrently).
    callId   @0 :Text;
    toolName @1 :Text;
    phase    @2 :ToolEventPhase;
    # JSON-serialized arguments (set on `started`).
    args     @3 :Text;
    # JSON-serialized result or error payload (set on `finished`).
    result   @4 :Text;
    # On `finished`: the result is an error payload.
    isError  @5 :Bool;
    # On `finished`: binary payloads the result carried (mirrors
    # Step.blobs), so a live UI can render them as they land.
    blobs    @6 :List(Blob);
  }
}
