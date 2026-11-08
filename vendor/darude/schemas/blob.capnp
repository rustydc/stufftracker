@0x88958d009b0f5516;
# blob.capnp — the platform-shared binary payload: raw bytes typed by a
# MIME type, optionally carrying the original file name.
#
# One shape for every place binary data crosses the membrane: a chat
# message's attachment (llm.chat.v1 Part.blob), a fetched image or PDF
# (web.fetch.v1 FetchResponse.blob), a photo thumbnail or original
# (photoprism.v1 getThumbnail/download). The agent loop recognizes this
# shape structurally in tool results and shows supported types
# (image/*, application/pdf) to the model as vision/document input —
# so any interface that returns a Blob gets "the model can see it" for
# free, with no per-interface agent support.
#
# Conventions:
#   - `data` is raw bytes on the wire; base64 happens only at HTTP
#     boundaries (a provider's JSON body, the chat UI's send body).
#   - practical ceiling is the capnp message read limit (~64 MiB);
#     consumers enforce their own tighter caps in-band (the agent loop
#     attaches nothing over its per-blob cap, providers enforce their
#     vendor's per-image limits).

struct Blob {
  # MIME type, e.g. "image/jpeg", "application/pdf". Drives rendering
  # and support decisions; parameters ("; charset=...") are tolerated.
  mimeType @0 :Text;
  # Original file name, if known. Display + type hints only.
  fileName @1 :Text;
  data     @2 :Data;
}
