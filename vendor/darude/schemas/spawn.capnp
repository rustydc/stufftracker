@0xeb3a517f9d4c2b80;
# spawn.v1 — platform-vended capability that lets a grain create
# instances of installed packages.
#
# Unlike interface caps that come from other grains, Spawn is
# implemented by platformd itself. A grain receives it via the
# normal `requires:` mechanism (its manifest declares
# `requires: { spawner: { interface: spawn.v1 } }`), but the cap on
# the wire is rooted in platformd's BootstrapApi rather than another
# instance's provides registry.
#
# Authorization: which packages a Spawn cap can instantiate is
# determined per-cap, not by the interface. Each issued Spawn cap
# carries a SCOPE, encoded in the broker grant's rights_json (parsed
# by SpawnScope::from_rights_json). Two vocabularies compose:
#
#   spawn:<package_id> — an EXPLICIT entry: this package, statically,
#                        no further checks. The sibling-only default.
#   spawn:*            — the DERIVED universe: whatever the grain's
#                        OWNING USER may currently launch, evaluated
#                        live per call against the launch policy
#                        (`published` = any real user, `restricted` =
#                        launch-grant holders; docs/spawn-policy.md).
#                        The owner is found by walking the instance
#                        tree to the nearest user-held ancestor — the
#                        same walk derived shell access uses. No owner
#                        (orphan subtree, suspended user) = the star
#                        grants nothing. Nothing is stored, so sharing
#                        an app with the user (or restricting one)
#                        changes every agent's spawnable set at the
#                        next call.
#
# spawn() is default-deny — it fails if the requested package is
# neither explicitly listed nor star-permitted, and an empty scope
# spawns nothing (fail-safe, never fail-open).
#
# A spawner requested from a manifest (`requires: { spawner:
# { interface: spawn.v1 } }`) defaults to SIBLING-ONLY: the platform
# scopes it to the grain's own package id (install.rs
# grant_spawn_caps). A manifest opts into the derived universe by
# declaring it on the require — `rights: ["spawn:*"]` — which is
# visible at install/launch time like any other declared authority.
# A SPAWNED grain's scope is additionally clamped by its parent's
# (see spawnMask below): delegation only narrows.
#
# Resource limits: every spawn is platform-checked against a depth
# cap, a per-subtree live-instance cap, and a per-spawner rate cap
# (env DARUDE_SPAWN_MAX_DEPTH / DARUDE_SPAWN_MAX_SUBTREE /
# DARUDE_SPAWN_RATE_PER_MIN), and recorded in the spawn ledger,
# where the owner can block a (grain, package) pair — a block on an
# ancestor covers the whole subtree. docs/spawn-policy.md.
#
# Ownership of spawned instances: the new instance's broker cap is
# parented to the calling grain's Spawn-cap grant, so revoking the
# Spawn cap (or any ancestor) cascades to every instance spawned
# through it. This is what gives controllers a clean tear-down
# path — sever the controller, all its workers go away.

# Spawn-time delegation: one capability the spawner hands the new
# instance at birth, so the child is born holding exactly the
# authority its parent chose — no post-launch offer dance, no window
# where the child runs unprovisioned.
#
# `cap` MUST be a broker caretaker the SPAWNER itself holds (a
# requires binding or a stashed powerbox cap). A raw, un-membraned
# cap is refused — fail closed, same rule as the membrane's rewrap.
# For each grant the platform mints a fresh broker row: subject =
# the new instance, object = the cap's provider, parent = the
# spawner's own cap row. That parenting is the point:
#   - severing the child's row revokes only the child's access;
#   - severing the spawner's row (or any ancestor) cascades to the
#     child transitively;
#   - the child's calls audit under the child's row, so actor and
#     authority stay distinct in the log.
#
# `slot` routes the grant. A non-empty slot names a `requires:`
# entry in the spawned grain's manifest — the grant fills that slot
# and the child's session handshake dials it like any launch-wired
# binding (this is how a spawned chat gets its `llm`). An empty
# slot delivers the cap through the child's `accept_cap.v1` inlet
# on readiness — the powerbox stash path (this is how it gets its
# tools). Both are re-delivered from the durable rows on every
# wake, so spawn-time grants survive hibernation.
#
# `rights` attenuates: the child row carries the intersection of
# the parent row's rights and this list (empty = inherit all).
# `label` is the tool handle / display label for stash deliveries.
struct SpawnGrant {
  cap    @0 :Capability;
  slot   @1 :Text;
  rights @2 :List(Text);
  label  @3 :Text;
}

interface Spawn {
  # Instantiate a previously-installed package and return a fresh
  # cap to the new instance, typed against one of the package's
  # provided interfaces. Default-deny: fails if `packageId` is
  # outside this cap's scope.
  #
  # `packageId` must name a row in the platform's app_packages
  # table; `interface` picks which of the package's provided
  # interfaces the returned cap is typed against (the package must
  # advertise it). An empty packageId is shorthand for "my own
  # package" when the cap's allowlist has exactly one entry (the
  # sibling-only default), so a grain can self-spawn without knowing
  # its package id.
  #
  # `params` are opaque grain-init bytes — written to the new
  # instance's state directory as `init.dat` before its
  # subprocess starts, so the grain can read them on first boot.
  # Pass empty bytes if you don't need init params.
  #
  # `grants` are processed BEFORE the instance boots (see
  # SpawnGrant above); a grant that fails validation fails the
  # whole spawn — no partially-provisioned orphan is left running.
  #
  # `name` labels the new instance in the shell (empty = the
  # package id).
  #
  # `grainId` disambiguates which grain template to instantiate when
  # a package has several grains advertising the requested `interface`
  # (e.g. the agent package's enforcer, router, and providers all
  # provide llm.chat.v1). Empty = select by interface alone, which is
  # an error if more than one grain matches. When set, the named grain
  # must still advertise `interface`.
  #
  # The returned cap is a fresh caretaker pointing at the new
  # instance's `interface` provider. Calls audit + sever-check
  # the same as any handshake-acquired cap.
  #
  # `keep` makes the spawner's hold on the child DURABLE: besides
  # returning the session-scoped cap, the platform records a
  # powerbox-style offer of the child's `interface` back to the
  # spawner (label = `name`, parented under the spawner's ownership
  # row so the sever walk covers it) and delivers it through the
  # spawner's accept_cap.v1 before spawn returns. The replay-on-ready
  # then re-delivers it on every wake, so the spawner still holds the
  # child after either side hibernates. Requires the spawner's grain
  # to provide accept_cap.v1 — validated before the instance is
  # created, so a keep-spawn that can't be kept fails with no orphan.
  # `spawnMask` attenuates the CHILD's own spawn authority. The
  # child's manifest-declared spawner (if any) is minted as: the
  # child's declared scope ∩ the parent's scope ∩ this mask. Entries
  # are package ids, or "*" for the parent's whole scope. An empty
  # list inherits (clamps to the parent's scope alone); the literal
  # entry "none" grants the child no spawn authority at all. An entry
  # outside the parent's own scope is REFUSED, not clamped — you
  # can't delegate what you don't hold, and the caller should learn.
  spawn @0 (
    packageId :Text,
    interface :Text,
    params    :Data,
    grants    :List(SpawnGrant),
    name      :Text,
    grainId   :Text,
    keep      :Bool,
    spawnMask :List(Text),
  ) -> (cap :Capability, instanceId :Text);

  # List what this cap can spawn, right now. This is the discovery
  # surface: an agent lists, picks a package and an api, then
  # spawns.
  #
  # Explicit scope entries appear verbatim, plus — for a star-scoped
  # cap — every visible (non-hidden) package the owning user may
  # currently launch, evaluated live (the same projection the user's
  # own shelf shows them). Blocked-in-the-ledger packages are
  # filtered out, so the list is honest about what a spawn would
  # allow.
  list @1 () -> (packages :List(SpawnablePackage));
}

# One row of list(): enough to pick a spawn target without any
# out-of-band knowledge. `grains` maps grain ids to the api each
# advertises — pass the api as spawn()'s `interface`, and the grain
# id as `grainId` when several grains advertise the same api.
struct SpawnablePackage {
  packageId @0 :Text;
  name      @1 :Text;
  version   @2 :Text;
  grains    @3 :List(GrainApi);
}

struct GrainApi {
  grainId @0 :Text;
  api     @1 :Text;
}
