@0xd6fbde8d8d83a53c;
# fs.v1 — a tree of files behind a capability: the lingua-franca face
# for "this thing's stuff is files" (docs/fs-caps.md).
#
# The first provider is a platform VOLUME (a plain directory under
# data/volumes/<id>, served platform-side with no grain process — the
# ShelfImpl pattern), but the interface is provider-blind: a grain
# whose state is files (silverbullet's space, photoprism's originals)
# can serve the same face from its bridge. Consumers range from LLM
# agents (these methods lower to tools automatically; the comments
# below ARE the tool descriptions the model reads) to the shell's
# file browser to other grains.
#
# This is the CONTROL/agent plane. Bulk data does not move through
# capnp: an app that needs kernel-speed access declares a manifest
# `requires` slot with `mount:` and the platform bind-mounts the
# volume into its sandbox (docs/fs-caps.md, "The mount"). The same
# grant backs both planes.
#
# Rights: `read` (or better) unlocks the observe surface — list /
# stat / readFile / readLines / search / glob / hash / du / quota /
# openDir. Mutations — writeFile / mkdir / remove / rename / copy —
# require `write` (or `owner`). Enforced per call against the live
# grant row, so attenuating or severing the grant re-shapes the
# surface on the next call.
#
# Paths are always relative to THIS capability's root, '/'-separated.
# Absolute paths, `..`, and symlink escapes are refused: a Directory
# cap on a subtree is a real boundary (resolution is
# openat2/RESOLVE_BENEATH-style — a symlink pointing outside the
# subtree fails the call rather than following). openDir() is
# therefore attenuation: handing someone a Directory on "complete/"
# is a narrower grant with no policy language.
#
# The read surface is lifted from workspace.v1, whose shapes are
# field-tested agent ergonomics: ranged byte reads with
# totalBytes/truncated, line-native reads with byteOffset seams,
# ripgrep-semantics search whose matches carry both line numbers and
# byte offsets.

using Blob = import "blob.capnp".Blob;

interface FsV1 {
  # === Observe surface ('read' rights) ===

  # List one directory's entries, name-sorted (not recursive; glob()
  # walks). Errors if `path` is missing or not a directory.
  #
  # `pageToken` is empty for the first page; a non-empty
  # `nextPageToken` in the response means more entries remain — pass
  # it back verbatim. `pageSize` 0 = the 500 default, capped at 2000.
  list @0 (path :Text, pageSize :UInt32, pageToken :Text)
      -> (entries :List(Entry), nextPageToken :Text);

  # Metadata for one file or directory. `allocatedBytes` is the real
  # disk usage (st_blocks), which differs from `sizeBytes` for sparse
  # files — a preallocated torrent file stats at full size while
  # mostly holes; the gap says how much data actually exists.
  stat @1 (path :Text) -> (entry :Entry);

  # Read a file's bytes from `offset`, at most `limit` bytes (0 = the
  # whole file — an error above the 16 MiB per-call ceiling; pass a
  # range to slice instead).
  #
  # `limit` > 0 returns up to that many bytes from `offset` (capped
  # at the ceiling per call). `totalBytes` is the file's full size
  # and `truncated` reports whether the blob carries less than the
  # whole file, so a caller knows to keep paging. The blob's mimeType
  # is guessed from the extension (UTF-8 sniff → text/plain
  # fallback); fileName is the path's final segment. Errors if
  # missing or a directory.
  readFile @2 (path :Text, offset :UInt64, limit :UInt32)
      -> (blob :Blob, totalBytes :UInt64, truncated :Bool);

  # Read a file by lines: up to `maxLines` lines (0 = the 500-line
  # default) starting at 1-based `startLine` — the line-native
  # sibling of readFile, ideal for context around a search() hit.
  #
  # A search() hit at line N becomes context with no byte math:
  # readLines(path, startLine = N - 5, maxLines = 10). `startLine` =
  # 0 reads from the top; `maxLines` is capped at 10000 per call.
  # `text` is those lines' exact content (terminators included),
  # decoded lossy-UTF-8; `byteOffset` is the first returned line's
  # offset in the file — the seam back to readFile's byte paging —
  # `lineCount` how many lines came back, and `moreLines` whether the
  # file continues past them. Responses are also byte-bounded
  # (1 MiB): a run of enormous lines returns fewer than maxLines —
  # possibly one clipped line — with moreLines = true. A startLine
  # past EOF returns empty, not an error.
  readLines @3 (path :Text, startLine :UInt64, maxLines :UInt32)
      -> (text :Text, byteOffset :UInt64, lineCount :UInt32, moreLines :Bool);

  # Regex search over the tree (ripgrep's engine: binary files
  # skipped), returning structured, bounded matches. Matches are
  # walk-ordered, not ranked; `truncated` = the cap cut the walk
  # short — narrow the pattern or glob rather than paging.
  search @4 (req :SearchRequest) -> (resp :SearchResponse);

  # List files under the tree matching a glob (e.g. "**/*.mkv" or
  # "complete/**"), paths sorted, relative to this capability's root.
  # `maxResults` 0 = the 200 default, capped at 1000; `truncated`
  # reports a cut-short walk.
  glob @5 (pattern :Text, path :Text, maxResults :UInt32)
      -> (paths :List(Text), truncated :Bool);

  # Content hash of one file (SHA-256, lowercase hex) without moving
  # the bytes: cheap change detection and tree diffing for sync or
  # backup callers — compare hashes, transfer only what differs.
  # Errors on a directory.
  hash @6 (path :Text) -> (sha256 :Text, sizeBytes :UInt64);

  # Recursive size of a subtree in one call (files + directories
  # walked server-side) — "what's eating the volume" without a crawl.
  #
  # `apparentBytes` sums file sizes; `allocatedBytes` sums real disk
  # usage (sparse-aware); `fileCount`/`dirCount` are the walk totals.
  du @7 (path :Text)
      -> (apparentBytes :UInt64, allocatedBytes :UInt64,
          fileCount :UInt64, dirCount :UInt64);

  # This capability's storage ceiling and usage. `limitBytes` = 0
  # means no quota is configured. `usedBytes` is the provider's
  # current accounting (for a volume: allocated bytes under the
  # root). React before the writer hits the ceiling.
  quota @8 () -> (usedBytes :UInt64, limitBytes :UInt64);

  # A Directory capability rooted at `path` — subtree attenuation:
  # the returned cap sees only that subtree and carries at most this
  # capability's rights. Hand it onward instead of the whole tree.
  #
  # The returned cap's paths are relative to the new root, and
  # resolution cannot escape it. Errors if `path` is missing or not a
  # directory.
  openDir @9 (path :Text) -> (dir :FsV1);

  # === Mutating surface ('write' rights) ===

  # Write blob.data to `path`, creating parent directories.
  # Overwrites. The blob's mimeType/fileName are advisory — the path
  # names the file. Per-call payloads are RPC-sized (16 MiB); bulk
  # writers use the mount plane, not this.
  writeFile @10 (path :Text, blob :Blob) -> ();

  # Create a directory (and parents). Idempotent on an existing
  # directory; errors if `path` exists as a file.
  mkdir @11 (path :Text) -> ();

  # Remove a file, or a directory with `recursive` (a non-empty
  # directory without `recursive` errors). Removing a missing path
  # errors — a caller that wants idempotence stats first.
  remove @12 (path :Text, recursive :Bool) -> ();

  # Atomically rename/move within this tree — THE publish primitive:
  # write into "incoming/", rename into "complete/" and readers never
  # see a half-written file.
  #
  # Creates the destination's parent directories; overwrites a
  # same-type destination (file over file, per rename(2)). Both paths
  # resolve inside this capability's root.
  rename @13 (from :Text, to :Text) -> ();

  # Server-side copy of one file; overwrites the destination and
  # creates its parent directories. Files only in v1 (copy a tree
  # with glob + copy).
  #
  # Runs the reflink ladder (FICLONE where the filesystem supports
  # it, byte copy elsewhere): duplicating a large file is O(metadata)
  # on btrfs/XFS/ZFS and never moves bytes through RPC.
  copy @14 (from :Text, to :Text) -> ();

  # === Change notification ('read' rights) — docs/fs-caps.md ===
  #
  # The reactive story a torrent→media-server flow needs: the torrent
  # writes files (through the bind-mount plane, never touching this
  # capnp face), and the media server learns to rescan. Because the
  # writer bypasses the cap, only DISK observation sees the change —
  # the platform watches the volume directory host-side and delivers
  # here. Scope is this capability's subtree (attenuation = scoping):
  # a cap from openDir("complete/") only reports changes under it.

  # List files created or modified since `cursor` (unix millis; 0 =
  # everything currently present), oldest-first by mtime, scoped to
  # this capability's root. Idempotent drain; deletes not enumerated.
  #
  # `maxResults` 0 = the 1000 default, capped at 5000; `truncated` =
  # the cap cut the scan short (call again with `nextCursor`).
  # `nextCursor` is the millis to pass next time. Deletes are NOT
  # enumerated in v1 (this is an mtime scan, like a poll — a `watch`
  # fire still signals that *something* changed, and a holder that
  # cares about removals re-lists). 'read'-scoped, like the other
  # observe methods.
  changesSince @15 (cursor :UInt64, maxResults :UInt32)
      -> (changes :List(Change), nextCursor :UInt64, truncated :Bool);

  # Durable change notification: fires the caller's persistable
  # Trigger — coalesced — whenever this capability's subtree changes
  # on disk, waking a hibernated holder, until Watch.stop().
  #
  # Same shape as workspace.v1 Process.watch
  # (docs/durable-held-caps.md). Pass a persistable Trigger the
  # CALLER owns; the platform save()s it and fires it per the
  # holder's wakeup policy. The fired payload is kind "fs.changed"
  # with nextCursor = the detection unix millis; the holder drains
  # specifics via changesSince(cursor). Registering is idempotent per
  # (holder, subtree) while live. 'read'-scoped.
  watch @16 (req :WatchRequest) -> (watch :Watch);

  # Handle to a live subscription from watch().
  interface Watch {
    # End it: forget the saved Trigger + drop the platform watch state.
    # Idempotent.
    stop @0 ();
  }

  struct WatchRequest {
    # The caller's persistable Trigger (a bare Capability so this schema
    # needn't import trigger.v1; the platform casts it), fired on change.
    notifyMe @0 :Capability;
  }

  struct Change {
    # Path relative to this capability's root.
    path @0 :Text;
    # "created" or "modified" (v1 is mtime-based; deletes aren't
    # enumerated — see changesSince).
    kind @1 :Text;
    # Last-modification time, unix millis (the ordering key).
    modifiedMs @2 :UInt64;
    sizeBytes @3 :UInt64;
  }

  struct Entry {
    # Name within its directory (list) or the queried path's final
    # segment (stat).
    name @0 :Text;
    # "file", "dir", or "symlink" (symlinks are listed but never
    # followed out of the tree; stat reports the link itself).
    kind @1 :Text;
    # Apparent size in bytes (0 for directories).
    sizeBytes @2 :UInt64;
    # Real disk usage (st_blocks * 512) — differs from sizeBytes for
    # sparse files.
    allocatedBytes @3 :UInt64;
    # Last-modification time, unix millis.
    modifiedMs @4 :UInt64;
  }

  struct SearchRequest {
    # The regex (ripgrep syntax). Required.
    pattern @0 :Text;
    # Optional glob filter on file paths, e.g. "*.md" or "notes/**".
    glob @1 :Text;
    # Subtree to search; empty = this capability's root.
    path @2 :Text;
    # Result ceiling; 0 = the 200 default. Capped server-side at 1000.
    maxResults @3 :UInt32;
    caseInsensitive @4 :Bool;
    # Treat `pattern` as a literal string, not a regex.
    fixedString @5 :Bool;
    # Context lines around each match (rg -B / -A): fill each Match's
    # `before`/`after` with up to this many neighboring lines. 0 =
    # none; capped server-side at 20 — context is a peek, readLines
    # is the paging read for more.
    contextBefore @6 :UInt32;
    contextAfter @7 :UInt32;
  }

  struct SearchResponse {
    matches @0 :List(Match);
    # True when maxResults cut the walk short.
    truncated @1 :Bool;

    struct Match {
      # Path relative to the searched root (as glob() returns them).
      path @0 :Text;
      # 1-based line number of the match.
      line @1 :UInt64;
      # The matching line's text (long lines clipped server-side).
      text @2 :Text;
      # Absolute byte offset of the matching LINE's first byte in the
      # file — the seam to readFile's offset/limit and readLines'
      # byteOffset.
      byteOffset @3 :UInt64;
      # Neighboring lines (contextBefore/contextAfter in the request;
      # clipped like `text`, empty when not requested). Per-match
      # copies, not merged blocks; edges are real.
      before @4 :List(Text);
      after @5 :List(Text);
    }
  }
}
