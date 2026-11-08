@0x9d7f3b2c4e5a6f81;
# TextWriter — shared streaming-write surface.
#
# Both wave.v1 (via openBlipWriter) and rustpad.v1 (via openWriter)
# vend caps to this interface. Same method shape, same semantics:
# holding the cap holds the underlying writer; drop it (or sever
# via broker) and the bridge tears the writer down.
#
# Why share: the LLM agent can lower this one interface to a single
# tool-use definition (TextWriter.append(text)) and dispatch through
# whatever cap it holds, without caring which backing store the
# writer points at. Adding a calendar-style "AppendableNote" later
# is a manifest declaration; no new tool schema needed.

interface TextWriter {
  # Append `text` at the writer's current insertion point. For wave
  # that's the end of the open blip; for rustpad that's the end of
  # the open pad. Resolves once the bridge has confirmed the write
  # landed (i.e. the receiver's broadcast/echo of our op).
  append @0 (text :Text) -> ();
}
