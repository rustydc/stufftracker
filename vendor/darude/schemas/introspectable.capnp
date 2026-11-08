@0xa3f1e2c7d8b94f03;
# Introspectable + OpaqueCap — the bridge between the cap world and
# anything that wants to consume cap interfaces dynamically (most
# obviously: LLM agents that need a tool-use schema).
#
# Design contract:
#
#   1. (GRAVEYARD, stage 4: `interface Introspectable` / whatAmI is
#      dead — the membrane's `cap.v1` describe() answers "what am I
#      holding" for every platform-tracked cap, no mixin required.)
#
#   2. Caps that flow across an RPC boundary as "opaque" (the agent
#      case, dynamic shelves, ad-hoc sharing) are passed as OpaqueCap.
#      That struct carries the cap itself + its interfaceId + the
#      serialized capnp schema for the interface (a
#      CodeGeneratorRequest containing the interface and every type
#      it transitively references).
#
#   3. The sender includes `schema` bytes on the FIRST send of each
#      interfaceId on a connection, in each direction; on subsequent
#      sends of the same interfaceId, sender MAY omit the field. The
#      receiver maintains a per-(connection, direction) cache mapping
#      interfaceId → SchemaProto. On receive:
#
#        if schema.size() > 0 : parse, insert into cache, use
#        else if cached       : use the cached SchemaProto
#        else                 : PROTOCOL ERROR (sender bug; the cap
#                                is unusable because there's nothing
#                                to dispatch against)
#
#      No central SchemaRegistry — the bytes always reach the
#      receiver inline by the time they're needed. See SchemaCache
#      in darude-runtime for the helper implementation both sides
#      use.
#
#   4. Lowering schema → LLM tool-use JSON Schema is a pure function
#      (not an interface method): anyone with the schema bytes can run
#      it locally. No service round-trip needed when the agent provider
#      receives an OpaqueCap; it introspects the schema via
#      darude-runtime's `introspect` module, lowers each method to
#      OpenAI tool-definition JSON (see the llm-openai-provider grain's
#      tool_schema), and passes the result to the model.

# A capability paired with enough info to interpret it dynamically.
# Used as the return type of shelf.list() and anywhere else a method
# wants to hand back a cap whose interface the caller doesn't know
# at compile time.
struct OpaqueCap {
  cap         @0 :Capability;
  interfaceId @1 :UInt64;
  # Optional human-readable hint. Not authoritative — agents should
  # rely on the parsed `schema` for actual call construction; the
  # label is for UI rendering / debug printing.
  label       @2 :Text;
  # Serialized capnp::schema::CodeGeneratorRequest containing the
  # interface identified by `interfaceId` plus every type it
  # transitively references — enough to interpret and call the cap
  # dynamically.
  #
  # Required on first per-connection-direction send of this
  # interfaceId, MAY be empty thereafter (the receiver caches). See
  # SchemaCache helper for the bookkeeping.
  schema      @3 :Data;
}
