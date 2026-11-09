@0xb7e3f1a9c4d20568;
# outline.v1 — capnp wrapper around the Outline REST API.
#
# Wire model: each cap is principal-bound at session-handshake time.
# The bridge acts as the session's principal: at handshake it mints
# an Outline session token by calling the grain's own /auth/proxy
# route (the same trusted-header path the browser uses — see the
# `proxy` auth plugin), then stamps `Authorization: Bearer <token>`
# on every /api/* call. So the bridge and the browser provision and
# authenticate the same Outline user through one code path; no admin
# token, no API-key bookkeeping.
#
# Multi-user model: this is a SHARED, per-WORKSPACE grain — one
# Outline team, many users. Each principal sees and edits what their
# Outline role allows. The cap's grant rights gate mutations on TOP
# of that: a read-only cap rejects every write method in the bridge,
# regardless of the principal's Outline role.
#
# Encoding: document bodies are Markdown (Outline's native format);
# timestamps are Int64 unix millis; ids are Outline UUIDs as Text.

using Blob = import "/blob.capnp".Blob;

interface OutlineV1 {
  # === Reads (require the `read` right) ===

  # All collections in the workspace the principal can see.
  listCollections @0 () -> (collections :List(Collection));

  # Page through documents, optionally scoped to one collection.
  # `pageToken` is opaque (carry it back verbatim); empty starts at
  # the first page, and an empty `nextPageToken` means no more.
  listDocuments @1 (req :ListDocumentsRequest) -> (resp :ListDocumentsResponse);

  # Fetch one document (Markdown body included) by id.
  getDocument @2 (id :Text) -> (doc :Document);

  # Full-text search across the workspace; results are ranked, each
  # with a highlighted `context` snippet.
  search @3 (req :SearchRequest) -> (resp :SearchResponse);

  # === Writes (require the `write` right; the bridge rejects
  #     these on a read-only cap before any REST call) ===

  # Create a document in a collection — under `parentDocumentId` when
  # set, which nests it in the collection's tree. `publish=false`
  # leaves it as a draft (nested under the user, not in the collection
  # structure).
  createDocument @4 (req :CreateRequest) -> (doc :Document);

  # Update a document. With `append=true`, `text` is appended to the
  # existing body (the agent-friendly "add a section" verb); other-
  # wise `title`/`text`, when non-empty, replace.
  updateDocument @5 (req :UpdateRequest) -> (doc :Document);

  # Delete a document (and its nested children) to Outline's trash,
  # restorable from the UI for 30 days. `permanent=true` purges one
  # that is already in the trash.
  deleteDocument @11 (id :Text, permanent :Bool) -> ();

  # Move a document: into another collection, and/or under another
  # document (nesting it). `parentDocumentId` empty = top level of the
  # collection; `collectionId` empty = stay in the current one. Children
  # move with it. Returns the moved document.
  moveDocument @12 (id :Text, collectionId :Text, parentDocumentId :Text)
    -> (doc :Document);

  # Create a collection (a top-level section of the workspace).
  createCollection @8 (name :Text, description :Text) -> (collection :Collection);

  # Rename a collection and/or replace its description (empty = leave
  # unchanged).
  updateCollection @9 (id :Text, name :Text, description :Text)
    -> (collection :Collection);

  # Delete a collection AND every document in it — irreversible from
  # here. Move documents out first to keep them.
  deleteCollection @10 (id :Text) -> ();

  # Upload a file (a photo, a PDF) as an attachment of document
  # `documentId`. With `embed`, it's also appended to the document — an
  # image inline (`caption` its alt text), anything else as a link;
  # otherwise place `attachment.markdown` yourself with updateDocument.
  uploadAttachment @13 (documentId :Text, file :Blob, embed :Bool, caption :Text)
    -> (attachment :Attachment);

  # Fetch an attachment's bytes — to look at an image a document embeds.
  # `id` is the attachment id, or the link as it appears in the
  # document's Markdown (/api/attachments.redirect?id=…). Read-gated.
  getAttachment @14 (id :Text) -> (file :Blob);

  # === Change feed (read) ===

  # List document change events since a cursor (createdAt millis). Call
  # after a `watch` notification to learn what changed.
  #
  # Idempotent drain: the changes since a cursor, newest-relevant first
  # filtered. Maps onto Outline's `POST /api/events.list` (sorted by
  # createdAt; cursor = createdAt millis). The Sink calls this after a
  # `watch` notification to fetch the actual events the cursor advanced
  # past — the notification itself is edge-triggered (carries only the
  # cursor), so changesSince is how a woken Sink learns *what* changed.
  # At-least-once: a caller dedupes by (documentId, at) if it must.
  changesSince @6 (req :ChangesSinceRequest) -> (resp :ChangesSinceResponse);

  # === Durable change notifications (write-ish: needs the `watch` right) ===

  # Subscribe durably to change notifications; `notifyMe` fires
  # (coalesced, with a cursor) on each change until the returned
  # `Watch` is stopped.
  #
  # `notifyMe` is a persistable Trigger the CALLER owns (it must
  # implement restorable.v1 so the platform can rebuild it on wake).
  # The bridge `save()`s it once — getting a durable token + the
  # caller's wakeup policy — and on each (coalesced) change fires it
  # with a cursor payload, waking the caller per that policy. This is
  # the durable-held-caps "receive, save, call, forget" flow
  # (platformd-rs/docs/durable-held-caps.md); the bridge needs no
  # Deliver or Saver cap. Returns a `Watch` to stop the subscription.
  watch @7 (req :WatchRequest) -> (watch :Watch);

  # Handle to a live subscription created by `watch`.
  interface Watch {
    # End the subscription: forget the saved Trigger (drops its SturdyRef +
    # the bridge's saved_caps row) and the bridge's watch state. Idempotent.
    stop @0 ();
  }

  # === Types ===

  struct ChangesSinceRequest {
    sinceMillis @0 :Int64;      # exclusive cursor; 0 = from the beginning
    collectionId @1 :Text;      # empty = whole workspace
    limit @2 :UInt32;           # 0 = bridge default
  }

  struct ChangesSinceResponse {
    changes @0 :List(Change);
    # Pass back as the next `sinceMillis`. Equals the max `at` seen (or the
    # request cursor when nothing changed), so polling is lossless.
    nextCursor @1 :Int64;
  }

  struct Change {
    kind @0 :ChangeKind;
    documentId @1 :Text;
    collectionId @2 :Text;
    at @3 :Int64;               # event createdAt, unix millis
    actor @4 :Text;            # display name of who made the change
  }

  enum ChangeKind {
    other @0;
    created @1;
    updated @2;
    deleted @3;
  }

  struct WatchRequest {
    # The caller's persistable Trigger (a bare Capability so this schema
    # needn't import trigger.v1; the bridge casts it). The membrane captures
    # it; the bridge saves + later fires it.
    notifyMe @0 :Capability;
    collectionId @1 :Text;      # empty = whole workspace
    sinceMillis @2 :Int64;      # starting cursor; 0 = changes from now
  }

  struct Attachment {
    id @0 :Text;
    # The attachment's in-Outline link (relative, as Outline pages embed
    # it); getAttachment takes it.
    url @1 :Text;
    name @2 :Text;
    contentType @3 :Text;
    size @4 :UInt64;
    # Ready to place in a document: ![caption](url) for an image,
    # [name](url) otherwise.
    markdown @5 :Text;
  }

  struct Collection {
    id @0 :Text;
    name @1 :Text;
    description @2 :Text;
  }

  struct Document {
    id @0 :Text;
    title @1 :Text;
    collectionId @2 :Text;
    text @3 :Text;          # Markdown body
    # The document's link — absolute, on this Outline's address, so it
    # opens from anywhere (a chat reply, another page).
    url @4 :Text;
    updatedAt @5 :Int64;    # unix millis
    parentDocumentId @6 :Text;  # "" at the top level of its collection
  }

  struct ListDocumentsRequest {
    collectionId @0 :Text;  # empty = all collections
    pageSize @1 :UInt32;    # 0 = bridge default
    pageToken @2 :Text;     # opaque; empty = first page
  }

  struct ListDocumentsResponse {
    documents @0 :List(Document);
    nextPageToken @1 :Text;
  }

  struct SearchRequest {
    query @0 :Text;
    pageSize @1 :UInt32;    # 0 = bridge default
  }

  struct SearchResponse {
    results @0 :List(SearchResult);
  }

  struct SearchResult {
    documentId @0 :Text;
    title @1 :Text;
    context @2 :Text;       # highlighted snippet around the match
  }

  struct CreateRequest {
    collectionId @0 :Text;
    title @1 :Text;
    text @2 :Text;          # Markdown body
    publish @3 :Bool;
    parentDocumentId @4 :Text;  # "" = top level of the collection
  }

  struct UpdateRequest {
    id @0 :Text;
    title @1 :Text;         # empty = leave unchanged
    text @2 :Text;          # empty = leave unchanged
    append @3 :Bool;        # true = append `text` to the body
  }
}
