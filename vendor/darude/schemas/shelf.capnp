@0xb7d8e9f1a2c34504;
# Shelf + Factory + Spawner — the user-instantiable-grains model.
#
# Replaces the special-cased "home page" and "auto-provisioned chat" in
# the TS shell. After this lands:
#
#   * Every user has a shelf grain (their "home") created at signup.
#   * Catalogs are also shelves, owned by whoever curated them, freely
#     shareable like any other grain.
#   * Each installed app-class is a factory grain; sharing an app is
#     just granting its factory cap into another user's shelf.
#   * Spawning an instance is `factory.spawn()` — the new grain's main
#     cap comes back, and the caller can `shelf.add(cap)` to pin it.
#
# Sandstorm shape, no Meteor/Mongo. The introspectable layer (above) is
# what's new: shelves return List(OpaqueCap) so an agent can walk the
# user's grain graph without knowing any of the inner interface types
# at compile time.

using Intro = import "introspectable.capnp";

interface Shelf {
  list    @0 () -> (entries :List(Entry));
  # `cap` rides as an OpaqueCap for the same lowering reason: the agent
  # tool layer synthesizes it from a cap-ref string.
  add     @1 (cap :Intro.OpaqueCap, label :Text) -> (entry :Entry);
  remove  @2 (entryId :Text) -> ();
  reorder @3 (entryIds :List(Text)) -> ();

  struct Entry {
    id          @0 :Text;
    label       @1 :Text;
    # The entry's cap as an OpaqueCap — could be a Factory, an
    # instance grain, another Shelf.
    #
    # OpaqueCap carries carrier + interfaceId + schema, so the agent
    # tool layer lowers list() and registers the returned caps as
    # callable cap-refs (a bare Capability is AnyPointer to the
    # introspector and would make list() unlowerable).
    cap         @2 :Intro.OpaqueCap;
    interfaceId @3 :UInt64;       # what the cap is, for the UI/agent to dispatch on
    # Frontend hints baked in at add() time (NOT used by agents, which go
    # through the registry instead).
    url         @4 :Text;
    appId       @5 :Text;
    iconUrl     @6 :Text;
  }
}

interface Factory {
  # Manifest-derived metadata for UI rendering + agent tool-use schema
  # generation. `permissions` and `requires` come straight from the
  # darude.app.yaml the package was installed from.
  describe @0 () -> (info :Info);

  # Spawn a new instance of this app; the caller becomes the owner
  # and gets back the new grain's main capability (normally then
  # added to a shelf).
  #
  # The factory cap is minted per user; instances land parented to
  # that user's root cap, NOT to the factory — uninstalling an app
  # never reaps the grains spawned from it. `instanceId` names the
  # user-facing (default) grain's instance for shells/tools that
  # address grains by id. `grain` is null for HTTP-only apps (nothing
  # callable to wrap — the shelf entry's url is the surface).
  # `initialConfig` is RETIRED (never read): first-run config forms are
  # gone — a grain serves its own config UI after launch, and a spawning
  # grain seeds a child via spawn.v1 `params` -> init.dat. The field
  # stays for wire-numbering stability.
  spawn @1 (displayName :Text, initialConfig :List(ConfigEntry))
        -> (grain :Capability, instanceId :Text);

  struct Info {
    appId       @0 :Text;
    appName     @1 :Text;
    version     @2 :Text;
    iconUrl     @3 :Text;
    permissions @4 :List(Text);
    requires    @5 :List(Requirement);
    # RETIRED (always empty): the first-run instantiation form is gone;
    # a grain's config lives behind its own web UI. Field stays for
    # wire-numbering stability.
    configSchema @6 :List(ConfigField);
  }

  struct Requirement {
    interfaceId @0 :UInt64;
    logical     @1 :Text;
    description @2 :Text;
  }

  struct ConfigField {
    key    @0 :Text;
    label  @1 :Text;
    secret @2 :Bool;
  }

  struct ConfigEntry {
    key   @0 :Text;
    value @1 :Text;
  }
}

# Platform-issued cap — held only by factory grains. The factory's
# spawn() is a thin wrapper around Spawner.spawnInstance(). Lives in
# this file because it's the one platform primitive that's
# fundamentally entangled with the factory pattern.
interface Spawner {
  spawnInstance @0 (
    packageHash :Text,
    ownerId     :Text,
    displayName :Text,
    initialConfig :List(Factory.ConfigEntry),
  ) -> (grain :Capability);
}
