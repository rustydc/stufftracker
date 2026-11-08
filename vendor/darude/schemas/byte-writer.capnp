@0xc3fc093348dbfe51;
# ByteWriter — shared binary streaming-write sink.
#
# The byte-exact sibling of TextWriter (text-writer.capnp): same
# hold-the-cap-holds-the-writer / drop-to-tear-down semantics, but the
# payload is arbitrary `Data`, with no UTF-8 validation. Use it wherever a
# faithful byte stream matters and a Text field would corrupt it — e.g.
# workspace.v1 Process.attachRaw streaming a terminal's output (escape
# sequences, non-UTF-8 locales, binary) to the ttyd bridge.
#
# Why a separate interface from TextWriter rather than one polymorphic
# sink: TextWriter is the LLM-lowerable one (process output can stream into
# a wave blip / pad / agent context as a tool); ByteWriter is pure
# transport — not something a model writes — so keeping them distinct keeps
# the text sink's tool-lowering clean. A consumer picks the attach variant
# that matches what it can handle.

interface ByteWriter {
  # Append `data` at the writer's current position. Resolves once the
  # write has landed, giving the producer natural backpressure (the
  # forwarder awaits each write before sending the next chunk).
  write @0 (data :Data) -> ();
}
