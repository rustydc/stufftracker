@0xb6d51c420e944cf6;
# workspace.v1 — launch and supervise long-lived processes inside a
# workspace/devcontainer grain, handing back a live `Process` capability
# per launch.
#
# The workspace grain already runs serve_sessions to receive its `repos`
# requires; this interface is *provided* on that same session, so a holder
# of the workspace.v1 cap can start a command and then drive/observe it
# through the returned handle — no ambient shell, no extra socket.
#
# This is deliberately the small "launch + handle" surface: enough for a
# process-monitor grain to enumerate and read processes (list / info /
# attach) and for a launcher to start and steer one (write / signal /
# wait). Richer supervision (restart policy, resource caps) is layered on
# top later; it doesn't change this contract.
#
# Output streaming reuses TextWriter (text-writer.capnp), the same sink
# wave.v1 / rustpad.v1 / llm-chat.v1 vend — so process output can flow
# straight into a pad, a blip, or an LLM agent's context with no new tool
# schema. Output is therefore text (UTF-8, lossy on non-UTF-8 bytes);
# binary-clean streaming would need its own Data sink and is out of scope
# for v1.

using TextWriter = import "text-writer.capnp".TextWriter;
using ByteWriter = import "byte-writer.capnp".ByteWriter;
using Blob = import "blob.capnp".Blob;

interface WorkspaceV1 {
  # Start a process. `req.argv[0]` is the program; empty argv launches the
  # grain's default shell. Returns the live handle, already running.
  launch @0 (req :LaunchRequest) -> (process :Process);

  # Snapshot of every process this grain currently tracks (running and
  # recently-exited), newest first. The read side a monitor leans on so it
  # doesn't have to hold a Process cap per child.
  list   @1 () -> (processes :List(ProcessInfo));

  # Re-obtain the live `Process` handle for a process this grain
  # already tracks, by `id` from list(). Errors if the id is unknown.
  #
  # list() returns plain structs, so without this a fresh consumer (the
  # ttyd bridge after a page reload) could SEE a running shell but
  # never re-attach to it, and had to launch a new one — the terminal
  # looked ephemeral even though the process outlived the connection.
  # 'observe' rights, like list/attach; the returned handle's mutating
  # methods still enforce 'control' per call.
  open   @7 (id :Text) -> (process :Process);

  # File exchange as the platform-shared Blob (blob.capnp): the seam an
  # LLM agent's blob-refs flow through, so a chat attachment can land in
  # the working tree ("put this CSV in the repo") and a build artifact
  # can come back out as a viewable/forwardable payload. Relative paths
  # resolve against the repos dir; absolute paths (/tmp/scratch.json)
  # and "~/" (the persistent home) work too — the grain is the boundary,
  # and launch() can already touch anything inside it.
  #
  # Rights split: reading the tree (readFile / readLines / search /
  # glob / stat / listDir) is
  # 'observe'-scoped, like reading process output — so a read-only
  # holder (the monitor, or an explorer subagent delegated an
  # attenuated cap) can browse code without command-execution
  # authority. Mutation (writeFile) requires 'control', same as launch.

  # Read a file's bytes from `offset`, at most `limit` bytes (0 = the
  # whole file — an error above the size ceiling; pass a range to
  # slice instead).
  #
  # `offset`/`limit` page through big files the way Process.output()
  # pages the spool: limit > 0 returns up to that many bytes from
  # `offset` (capped at the ceiling per call). `totalBytes` is the
  # file's full size and `truncated` reports whether the blob carries
  # less than the whole file, so a caller knows to keep paging. The
  # blob's mimeType is guessed from the extension (UTF-8 sniff →
  # text/plain as fallback); fileName is the path's final segment.
  # Errors if missing or a directory.
  readFile  @2 (path :Text, offset :UInt64, limit :UInt32)
      -> (blob :Blob, totalBytes :UInt64, truncated :Bool);

  # Read a file by lines: up to `maxLines` lines (0 = the 500-line
  # default) starting at 1-based `startLine` — the line-native sibling
  # of readFile, ideal for context around a search() hit.
  #
  # A search() hit at line N becomes context with no byte math:
  # readLines(path, startLine = N - 5, maxLines = 10). Paths resolve
  # like readFile's. `startLine` = 0 reads from the top; `maxLines` is
  # capped at 10000 per call. `text` is those lines' exact content
  # (terminators included), decoded lossy-UTF-8; `byteOffset` is the
  # first returned line's offset in the file — the seam back to
  # readFile's byte paging — `lineCount` how many lines came back, and
  # `moreLines` whether the file continues past them. Responses are
  # also byte-bounded (1 MiB, like Process.output pages): a run of
  # enormous lines returns fewer than maxLines — possibly one clipped
  # line — with moreLines = true. A startLine past EOF returns empty,
  # not an error. 'observe'-scoped, same tier as readFile.
  readLines @8 (path :Text, startLine :UInt64, maxLines :UInt32)
      -> (text :Text, byteOffset :UInt64, lineCount :UInt32, moreLines :Bool);

  # Write blob.data to `path`, creating parent directories. Overwrites.
  # The blob's mimeType/fileName are advisory here — the path names the
  # file on disk. Requires 'control' rights.
  writeFile @3 (path :Text, blob :Blob) -> ();

  # Run a command to completion and return its exit status plus the
  # complete captured output (stdout+stderr merged) as a text Blob.
  # `timeoutMs` bounds the wait (0 = 300s default). Requires 'control'.
  #
  # Deliberately not a truncated tail — output is spooled in full (up
  # to the spool ceiling) so a failure can be interrogated after the
  # fact instead of re-running: an agent holding the blob reads slices
  # of it (the error summary at the end first), and `process.output()`
  # pages the byte-exact spool. The Blob's data is lossy-UTF-8 (a text
  # payload that is always viewable); raw bytes live on
  # process.output().
  #
  # On timeout nothing is lost: exec returns timedOut = true with the
  # output so far, and `process` is still running — attach to it, poll
  # process.output(), signal it, or wait() for it. On completion
  # `process` is the exited handle (info/output still work). The
  # returned Blob carries at most the LAST 16 MiB; `outputBytes` is
  # the full spooled size. Requires 'control' rights, same as launch.
  exec @4 (req :LaunchRequest, timeoutMs :UInt32)
      -> (process :Process, exit :ExitStatus, output :Blob, outputBytes :UInt64, timedOut :Bool);

  # Regex search over the working tree (ripgrep's engine: gitignore-
  # respecting walk, binary files skipped), returning structured,
  # bounded matches instead of shell-scraped exec output. 'observe'-
  # scoped — the read-only tier's code-browsing primitive.
  search @5 (req :SearchRequest) -> (resp :SearchResponse);

  # List files under the tree matching a glob (e.g. "**/*.rs" or
  # "src/**"), gitignore-respecting, paths sorted, relative to the
  # searched root. `includeIgnored` widens the walk exactly as
  # SearchRequest's flag does. 'observe'-scoped, same tier as search.
  glob @6 (pattern :Text, path :Text, maxResults :UInt32, includeIgnored :Bool)
      -> (paths :List(Text), truncated :Bool);

  # Stat one path: does it exist, and what is it. `exists` = false for
  # a missing path (checking is the point — not an error); `entry`
  # reports the link itself for symlinks, like lstat.
  #
  # Paths resolve like search/glob roots: empty = the repos dir,
  # otherwise readFile rules. 'observe'-scoped. This and listDir exist
  # because search/glob are relevance-filtered (gitignore + hidden),
  # so without them an observe-only holder — the tier the rights split
  # serves — could not check whether target/debug/foo was built, or
  # look inside node_modules at all.
  stat @9 (path :Text) -> (exists :Bool, entry :Entry);

  # List one directory, name-sorted, non-recursive — the plain-readdir
  # sibling of glob: every child is listed, hidden and gitignored
  # alike. Errors if missing or not a directory (stat first to
  # distinguish).
  #
  # Enumeration is the point; glob is the relevance-filtered recursive
  # walk. `maxEntries` 0 = the 1000 default, capped at 10000;
  # `truncated` reports a cut. Unreadable children are skipped, not
  # errors. Paths resolve like search/glob roots: empty = the repos
  # dir. 'observe'-scoped.
  listDir @10 (path :Text, maxEntries :UInt32)
      -> (entries :List(Entry), truncated :Bool);

  # One filesystem object, as stat/listDir report it. Field names
  # match fs.v1's Entry (docs/fs-caps.md) so a holder of both caps
  # sees one vocabulary.
  struct Entry {
    # Name within its directory (listDir) or the queried path's final
    # segment (stat).
    name @0 :Text;
    # "file", "dir", or "symlink" (the link itself, never followed).
    kind @1 :Text;
    # 0 for directories.
    sizeBytes @2 :UInt64;
    # mtime, unix epoch milliseconds; 0 when unavailable.
    modifiedMs @3 :UInt64;
  }

  struct SearchRequest {
    # The regex (ripgrep syntax). Required.
    pattern @0 :Text;
    # Optional glob filter on file paths, e.g. "*.rs" or "src/**/*.ts".
    glob @1 :Text;
    # Subtree to search: resolves like readFile paths (relative →
    # repos dir, absolute and "~/" pass through). Empty = repos dir.
    path @2 :Text;
    # Result ceiling; 0 = the 200 default. Capped server-side at 1000.
    maxResults @3 :UInt32;
    caseInsensitive @4 :Bool;
    # Treat `pattern` as a literal string, not a regex.
    fixedString @5 :Bool;
    # Context lines around each match (rg -B / -A): fill each Match's
    # `before`/`after` with up to this many neighboring lines. 0 =
    # none; capped server-side at 20 — context is a peek, readLines is
    # the paging read for more.
    contextBefore @6 :UInt32;
    contextAfter @7 :UInt32;
    # Walk everything: gitignore/.ignore filters off AND hidden
    # entries included (.github, dotfiles). `.git` itself stays
    # pruned. Off = the default relevance-filtered walk.
    #
    # rg's -uu, minus the binary override; ask git via exec for repo
    # state. Not an access switch — readFile already opens any path;
    # this only widens *enumeration*, which is why an observe-scoped
    # flag is fine. (remote-host caveat: its grep/find fallbacks never
    # filtered, so there the flag only steers rg.)
    includeIgnored @8 :Bool;
  }

  struct SearchResponse {
    matches @0 :List(Match);
    # True when maxResults cut the walk short — narrow the pattern or
    # the glob rather than paging (matches are walk-ordered, not
    # ranked, so "the rest" isn't a meaningful page).
    truncated @1 :Bool;

    struct Match {
      # Path relative to the searched root (as glob() returns them).
      path @0 :Text;
      # 1-based line number of the match.
      line @1 :UInt64;
      # The matching line's text (long lines clipped server-side).
      text @2 :Text;
      # Absolute byte offset of the matching LINE's first byte in the
      # file — feed it to readFile's `offset` to read from the match
      # onward.
      #
      # The seam to readFile's offset/limit: `readFile(path, offset =
      # byteOffset, limit = N)` reads from the match onward, and
      # offset = max(0, byteOffset - N) pages context before it. Lines
      # are how a human reads a match; bytes are how the ranged
      # readers address one — a match carries both so no caller has to
      # count newlines to convert.
      byteOffset @3 :UInt64;
      # Neighboring lines around the match (contextBefore/contextAfter
      # in the request; clipped like `text`, empty when not requested).
      #
      # Per-match, not merged blocks: matches closer than the span
      # each carry their own copy, so a line may repeat across
      # neighbors' context — and a match line can itself appear in
      # another match's context. Edges are real: fewer lines come back
      # at file boundaries.
      before @4 :List(Text);
      after @5 :List(Text);
    }
  }

  # A single launched process. Holding the cap holds the handle; dropping
  # it does NOT kill the process (lifetime is explicit via signal()) — but
  # severing the workspace session tears the grain, and everything in it,
  # down.
  interface Process {
    # Current snapshot (pid, argv, running/exit). Cheap; safe to poll.
    info   @0 () -> (info :ProcessInfo);

    # Stream stdout+stderr to `out` as text as it arrives — replays
    # output already buffered since launch, then tails live until the
    # process exits or the sink is dropped.
    #
    # Output is UTF-8, lossy on non-UTF-8 bytes, one append per chunk.
    # Multiple attaches fan out independently. Resolves once attached.
    # Use this for display / LLM context (TextWriter is the shared,
    # tool-lowerable sink); use attachRaw when byte fidelity matters.
    attach @1 (out :TextWriter) -> ();

    # Byte-exact variant of attach: streams the same merged output to a
    # ByteWriter with no UTF-8 lossiness. For terminals (the ttyd bridge),
    # binary output, or faithful logs. Same replay-then-tail + fan-out
    # semantics as attach.
    attachRaw @6 (out :ByteWriter) -> ();

    # Feed `data` to the process's stdin.
    write  @2 (data :Data) -> ();

    # Close the process's stdin, delivering EOF — the final step after
    # write(): a command that reads stdin to its end (sort, wc, cat >
    # file) cannot finish until this is called. Idempotent; errors on a
    # PTY process (write the Ctrl-D byte 0x04 there instead).
    #
    # Killing a read-to-EOF consumer to unblock it discards its work
    # (sort emits nothing until EOF), which is why this exists. A later
    # write() errors with "stdin closed". A PTY master genuinely has no
    # half-close — closing it hangs up the whole terminal — hence the
    # in-band refusal steering to the terminal-native VEOF byte at the
    # start of a line. Requires 'control', same as write.
    #
    # exec() closes the child's stdin itself right after spawn: its
    # Process cap is only handed back at completion or timeout, so no
    # caller could ever have fed it, and an open stdin only makes
    # stdin-readers hang the full timeout. Batch commands therefore run
    # with `< /dev/null` semantics, and a write() on the handle exec
    # returns after a timeout errors with "stdin closed". (Pipe-backed
    # exec only; a PTY exec keeps its terminal open until exit.)
    closeStdin @10 () -> ();

    # Resize the controlling terminal (PTY launches only; a no-op error on
    # a pipe-backed process).
    resize @3 (cols :UInt16, rows :UInt16) -> ();

    # Send signal `sig` (e.g. 15 SIGTERM, 9 SIGKILL) to the process.
    signal @4 (sig :Int32) -> ();

    # Resolve when the process exits, with its final status. Returns
    # immediately if it has already exited.
    wait   @5 () -> (exit :ExitStatus);

    # Page through the process's captured stdout+stderr spool: up to
    # `limit` bytes (0 = 64 KiB default) from `offset`, readable at
    # any time — including while the process runs.
    #
    # The pull-style sibling of attach: a caller can poll progress
    # without a sink cap. The spool is byte-exact stdout+stderr since
    # launch, retained in full up to a ceiling (64 MiB;
    # `spoolTruncated` reports overflow — the spool keeps the HEAD,
    # the live attach ring keeps the tail). `limit` is capped at 1 MiB
    # per call. Reads are 'observe'-scoped, same as attach.
    output @8 (offset :UInt64, limit :UInt32)
        -> (data :Data, totalBytes :UInt64, running :Bool, spoolTruncated :Bool);

    # Regex search over this process's spooled output — find the
    # failure in a big log without paging the whole spool through
    # output(). Same pattern semantics as WorkspaceV1.search.
    #
    # maxResults 0 = the 200 default, capped at 1000; matches are
    # spool-ordered, truncated = the cap cut the scan short. Searches
    # output bytes rather than a file tree: the spool legitimately
    # carries NULs and terminal escapes, so the scan is lossy-UTF-8
    # line matching with NO binary bail-out — unlike tree search,
    # binary-ish output still matches. Each match carries the 1-based
    # line number, the clipped line text, and `byteOffset` — the
    # matching line's first byte in the spool, the seam to
    # output(offset, limit) for byte-exact context around a hit.
    # totalBytes/running/spoolTruncated mirror output(): only the
    # retained head is searchable after overflow (the live tail exists
    # only in the attach ring), and while `running` a later call may
    # see more. `contextBefore`/`contextAfter` fill each match's
    # neighboring lines, with SearchRequest's semantics (0 = none,
    # capped at 20, per-match copies). 'observe'-scoped, same as
    # output.
    searchOutput @9 (pattern :Text, maxResults :UInt32,
                     caseInsensitive :Bool, fixedString :Bool,
                     contextBefore :UInt32, contextAfter :UInt32)
        -> (matches :List(OutputMatch), truncated :Bool,
            totalBytes :UInt64, running :Bool, spoolTruncated :Bool);

    struct OutputMatch {
      # 1-based line number within the spool.
      line @0 :UInt64;
      # Absolute byte offset of the matching line's first byte —
      # feed it to output(offset, …), as Match.byteOffset feeds
      # readFile.
      byteOffset @1 :UInt64;
      # The matching line's text (lossy UTF-8, clipped server-side).
      text @2 :Text;
      # Neighboring lines, as Match.before/after.
      before @3 :List(Text);
      after @4 :List(Text);
    }

    # === Durable exit notification (docs/durable-held-caps.md) ===

    # Watch this process's exit durably: fires the caller's
    # persistable Trigger ONCE when the process exits, waking a
    # hibernated holder — the durable counterpart to wait().
    #
    # wait() is a live promise that dies with the session when either
    # grain hibernates. `notifyMe` is a persistable Trigger the CALLER
    # owns (it implements restorable.v1 so the platform can rebuild it
    # on wake). The grain save()s it once — getting a durable token +
    # the caller's wakeup policy — and fires it once on exit, waking a
    # hibernated holder per that policy. The fired payload is kind
    # "workspace.exited" with nextCursor = the detection unix millis,
    # plus extra text fields processId, exitCode and signal. Processes
    # are in-memory: if the workspace grain itself restarts before the
    # exit was observed, the child is dead by construction and its id
    # unknown — the watch fires once with exitCode -1, signal 0 and
    # lost = "true", so the holder still gets its wakeup and can
    # relaunch. After firing, the watch is spent (silent); call stop()
    # to drop the row. Registering after the process already exited is
    # fine — it fires on the next poll.
    watch @7 (req :WatchRequest) -> (watch :Watch);

    # Handle to a live subscription from `watch`.
    interface Watch {
      # End it: forget the saved Trigger + drop the grain's watch state.
      # Idempotent.
      stop @0 ();
    }

    struct WatchRequest {
      # The caller's persistable Trigger (a bare Capability so this
      # schema needn't import trigger.v1; the grain casts it).
      notifyMe @0 :Capability;
    }
  }

  struct LaunchRequest {
    # Program + args. Empty → the grain's default shell (bash). argv[0]
    # is resolved through PATH, so bare names ("bash", "git", "node")
    # work.
    #
    # The grain is a stock Ubuntu devcontainer running as root INSIDE a
    # sandbox, so ordinary user-space work is fine — but the sandbox is
    # the security boundary, not the uid: anything that switches users
    # (apt, which drops to _apt; su; sudo -u) fails unless the package
    # grants the privilege-drop caps, and tar's ownership preservation
    # fails for the same reason (use --no-same-owner). To add a
    # toolchain the portable route is a prebuilt tarball unpacked under
    # $HOME with ~/.local/bin on PATH — that also survives hibernation,
    # whereas anything written outside $HOME does not.
    argv @0 :List(Text);
    # Working directory. Empty → the grain's HOME; a nonexistent cwd
    # fails the launch.
    #
    # HOME is the durable dir (it survives hibernation; the rest of
    # the image rootfs does not). Bound repos are pre-cloned into
    # ~/repos/<name>, and the authoritative list — names + clone URLs
    # — is the JSON at $DARUDE_REPOS (also served at
    # http://127.0.0.1:9419/_darude/repos). Read it instead of
    # guessing a path: the names come from how the caps were bound.
    cwd  @1 :Text;
    # Extra environment, layered on top of the grain's inherited env.
    env  @2 :List(EnvVar);
    # Allocate a controlling PTY (interactive shell) instead of pipes.
    # When false, stdio is plumbed through pipes.
    pty  @3 :Bool;
    # Initial PTY dimensions (PTY launches only; ignored for pipes).
    cols @4 :UInt16;
    rows @5 :UInt16;
  }

  struct EnvVar {
    name  @0 :Text;
    value @1 :Text;
  }

  struct ProcessInfo {
    # Grain-scoped handle id (stable for the life of the grain).
    id        @0 :Text;
    # OS pid (advisory — meaningful only inside the grain).
    pid       @1 :Int32;
    argv      @2 :List(Text);
    cwd       @3 :Text;
    # The session principal that launched it (attribution).
    principal @4 :Text;
    # RFC3339 launch time.
    startedAt @5 :Text;
    running   @6 :Bool;
    # Valid only when running = false.
    exitCode  @7 :Int32;
  }

  struct ExitStatus {
    # Exit code (0 on clean exit; -1 if killed by signal).
    code   @0 :Int32;
    # Terminating signal number, or 0 if it exited normally.
    signal @1 :Int32;
  }
}
