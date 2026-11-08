@0xb1c2d3e4f5061718;
# SessionBootstrap — per-session connection bootstrap, served by the
# grain on its `session.sock` listener. platformd dials per
# (Principal, rights) tuple, calls handshake, and gets back the
# provides scoped to that session.
#
# Per-cap state (cursor, identity, etc.) lives on the per-session
# Server impls the grain constructs in response to handshake. Two
# users of the same grain → two connections → two distinct Server
# impl trees → no shared state, no races. capnp-rpc lets calls on
# any connection's caps proceed concurrently.
#
# Rights model: rights are load-bearing. An interface is in the
# response if the grant's rights overlap with what the interface's
# manifest entry declares it honors. Within an interface, the
# grain reads `self.session.rights` to gate individual methods.
# Grain is the single authority on what each right means.
#
# This is the ONLY bootstrap path between platformd and a grain.
# Requires flow the other way — platformd hands the grain the caps
# it manifest-declared a need for, scoped to the session's
# principal, so the grain's outgoing calls auto-attribute.

using Principal = import "principal.capnp".Principal;

interface SessionBootstrap {
  # Negotiate a session. platformd dials, calls this with the
  # session's Principal + the grant's rights + the requires caps
  # the grain manifest-declared a need for. Grain returns the
  # provides scoped to (principal, rights).
  #
  # Rights filtering: each provided interface in the grain's
  # manifest declares the rights it honors; an interface appears in
  # `provides` if `rights` intersects it (any-of, not all-of).
  # Within an interface, the grain decides per-method whether the
  # right is sufficient and errors with "right required" otherwise.
  #
  # An empty `rights` list means the holder has the grant but no
  # rights — typically yields an empty `provides` list, useful as
  # a placeholder.
  #
  # Requires caps come pre-bound to the session's principal so the
  # grain's outgoing calls attribute correctly without any
  # per-method shim — calling `requires.get("llm.chat.v1")` from
  # alice's session yields a cap that talks to llm.chat.v1 as
  # alice.
  handshake @0 (
    principal :Principal,
    rights    :List(Text),
    requires  :List(Required),
    restored  :List(Restored),
  ) -> (provides :List(Provided));

  # The PLATFORM CHANNEL — dialed once per process boot by platformd
  # itself, before any user session: delivers instance-lifecycle
  # facts and collects the grain's exports. Completing it with every
  # declared export present IS readiness.
  #
  # It carries the facts a grain needs exactly once per boot:
  #   - `requires`: the manifest bindings, bound to the INSTANCE (an
  #     anonymous principal — background work attributes as the grain,
  #     which is the honest actor; per-user attribution rides user
  #     sessions and offered caps, not instance bindings);
  #   - `rights`: the grain's full owner vocabulary, so rights-gated
  #     factories construct their complete surface.
  # The grain returns its EXPORTS — the declared `api:` plus each
  # `hooks:` interface — which platformd validates against the
  # manifest. Platform work (spawn provisioning, wakes) lands on
  # this channel; it no longer borrows an owner's principal.
  #
  # User sessions still call handshake() above, now carrying ONLY
  # principal + rights (their `requires` list is empty): a user
  # session is an attribution + rights-filtered view, cheap to open,
  # never a delivery vehicle.
  platformHandshake @1 (
    rights    :List(Text),
    requires  :List(Required),
    restored  :List(Restored),
  ) -> (exports :List(Provided));

  struct Provided {
    name @0 :Text;
    cap  @1 :Capability;
    # The interface's capnp type id, paired with `schema` — its
    # serialized CodeGeneratorRequest (CGR) bytes — so platformd can
    # introspect/offer the interface without compiling it in. Both
    # MAY be empty/0 for interfaces platformd already knows.
    #
    # The grain fills these from its own schema source; platformd
    # registers them in the wire schema registry on handshake. The
    # registry's first-wins keeps its built-in for known interfaces —
    # the pair only matters the first time a third-party interface
    # is seen.
    interfaceId @2 :UInt64;
    schema      @3 :Data;
  }
  # RETIRED (docs/self-discovery.md step 5): the pushed re-vend of a
  # holder's saved caps. Both `restored` lists above are always left
  # unset and never read — a grain re-derives its saved tokens itself
  # via `Self.restore(token)` (WatchSet::rearm_from_self). The struct
  # and fields stay only for wire-numbering stability.
  struct Restored {
    token @0 :Text;
    cap   @1 :Capability;
  }
  struct Required {
    name @0 :Text;
    cap  @1 :Capability;
    # Per-binding label, for `many:` requires where several caps
    # share one logical name. Empty for singular requires — readers
    # fall back to `name`.
    #
    # The label is the offer label (powerbox) or the providing
    # sibling's grain id (install-time `from:` wiring). The grain
    # uses it to tell bindings apart (e.g. the LLM router groups a
    # model catalog by it).
    label @2 :Text;
  }
}
