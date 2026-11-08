@0xf86913a5051b3b18;
# code.exec.v1 — run an (LLM-authored) Python script inside a Monty
# sandbox, with the grain's capabilities injected as callable objects.
#
# The script reaches its tools only through injected cap objects
# (e.g. `await web.fetch(url=...)`), each call routed back through the
# broker caretaker — so the membrane extends into the script: audit,
# sever, and quota apply to whatever the script does. The grain holds
# no ambient authority; Monty enforces memory/stack/time limits.
#
# This is the "agent scripts its tools" surface: instead of one
# model-turn per tool call, the agent writes a program that
# orchestrates its caps with real control flow.

using Intro = import "introspectable.capnp";
using Blob = import "blob.capnp".Blob;

interface CodeExecV1 {
  # Run a Python `script` to completion in the sandbox, with `tools`
  # (forwarded caps) injected as callable script globals and `blobs` as
  # binary inputs. Failures come back in-band as `error`, not a capnp
  # error, so the caller gets a result it can debug.
  #
  # The per-call CapSet is the grain's static `requires` set plus any
  # caps the caller forwards in `tools` (the agent's currently-held
  # caps that it chose to delegate to this script). Each tool becomes a
  # script global named by its OpaqueCap.label. Returns the repr of the
  # script's final expression plus captured stdout; failures
  # (exceptions, limit exceeded, timeout) come back as `error` rather
  # than an exception that aborts the loop.
  #
  # `blobs` carries binary payloads into the sandbox: each appears in
  # the script's `blobs` global dict under its name, as a dict
  # {"mimeType", "fileName", "data"} with `data` as Python bytes —
  # the same shape a cap method returning a Blob produces, so a
  # payload can flow attachment → script → cap param unchanged. The
  # agent lowers this so the model passes blob:// refs; the loop
  # resolves them to bytes before the call.
  run @0 (script :Text, tools :List(Intro.OpaqueCap), blobs :List(NamedBlob)) -> (resp :RunResponse);

  struct NamedBlob {
    # Key in the script's `blobs` global.
    name @0 :Text;
    blob @1 :Blob;
  }

  struct RunResponse {
    ok     @0 :Bool;
    # Repr of the script's final value. Caps embedded in the value
    # tree show up as "<cap:N>" placeholders, where N indexes into
    # `returnCaps` below; binary payloads as "<blob:N>" indexing into
    # `returnBlobs`.
    result @1 :Text;
    stdout @2 :Text;   # captured print() output
    error  @3 :Text;   # non-empty on exception / limit / timeout
    # Caps the script returned (or that appeared anywhere in the
    # return value's structure), still callable after the script
    # exits.
    #
    # The agent receives these the same way it receives caps from any
    # typed-cap method return: the provider mints fresh cap-refs and
    # lowers each cap's interface to new tools, so the model can keep
    # calling them after the script exits.
    returnCaps @4 :List(Intro.OpaqueCap);
    # Binary payloads the script returned: any bytes value in the
    # return tree, or any dict shaped {"mimeType", "fileName", "data"}.
    #
    # Bare bytes are packaged as an untyped octet-stream Blob; the
    # dict shape is a typed Blob — e.g. one that came from a cap call,
    # or that the script built to give its output a real type. The
    # agent registers each as a blob-ref the model can view or
    # forward, same as any Blob-typed method return.
    returnBlobs @5 :List(Blob);
  }
}
