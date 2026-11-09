@0xca4b7d7de2f0c74a;
# homeassistant.v1 — capnp bridge to a REMOTE Home Assistant instance.
#
# Unlike the catalog's other bridges this grain bundles no upstream
# binary: Home Assistant's reason to exist is ambient LAN authority
# (mDNS/SSDP discovery, UDP, USB radios) that a darude sandbox is built
# to deny, so HA itself stays on the user's own box/HA-OS and this
# grain is the trust terminus for reaching it — the base URL + a
# long-lived access token live in this grain's private state and
# nowhere else, and every outbound call rides the per-grain egress
# proxy (audited, host-restricted). The provider-grain pattern
# (llm-openai-provider), pointed at the user's house.
#
# Wire model: what a cap can do is frozen from the session's rights at
# handshake. Reads (states, registries, service catalog + signatures,
# history, template render, stored-config reads, dashboard reads,
# change drains/watches) need `read`; the mutations — `callService`
# (device/entity actions), the config writes (`writeConfig`/
# `deleteConfig`, which author HA's stored automations/scripts/
# scenes), and dashboard edits — need `write` (or `owner`). What can
# break the house — creating/deleting dashboards, integration and
# registry administration (areas and floors included), HACS installs,
# restarting or stopping HA —
# needs `admin`, which `write` does NOT imply (an owner grant carries
# it). A `read` grant is a genuinely look-don't-touch view of the
# house.
#
# Transport: HA's REST API for states/services/history/stored-config,
# plus HA's WebSocket API (/api/websocket, same base URL + token) for
# the surfaces REST simply doesn't have: the entity/device/area/floor/
# label registries, Lovelace dashboards, and long-term statistics.
# Events are cursor-polled from /api/states (each entity carries
# last_updated), the same lossless drain model as silverbullet.v1 —
# while the grain is awake a WS push subscription pokes the poller so
# watches fire near-instantly, but the poll contract stays the source
# of truth (a dropped socket costs latency, never events). Entity
# *removal* is not observable (the row just stops appearing).

interface HomeassistantV1 {
  # === Reads ===

  # Instance metadata (GET /api/config) — doubles as a liveness/auth
  # probe: errors if the grain is unconfigured or HA is unreachable.
  info @0 () -> (info :Info);

  # List the area registry with entity assignments — areas are HA's
  # room model.
  #
  # Rides the WebSocket registry commands. Entities inherit their
  # device's area unless overridden — entityIds is the resolved join
  # (entity's own area, else its device's), disabled entities omitted.
  listAreas @1 () -> (areas :List(Area));

  # Service catalog (GET /api/services): every callable domain/service
  # pair, e.g. ("light", "turn_on"). The discovery surface for
  # callService — names only, so pair it with describeService before
  # calling anything whose data payload you don't already know.
  listServices @2 () -> (domains :List(ServiceDomain));

  # Fetch one service's full signature: its description plus the
  # field schema HA publishes, as raw JSON. Call before callService
  # for any service whose data payload you don't already know.
  #
  # The field schema (name, description, required, example, selector)
  # is what lets an agent build a correct callService data payload
  # instead of guessing key names — fetch the one service it's about
  # to call, right-sized for the model's context, rather than
  # dragging every service's schema through listServices. Errors
  # "not found" for an unknown domain/service.
  describeService @7 (domain :Text, service :Text)
    -> (description :Text, fieldsJson :Text);

  # Entity states with attributes, optionally filtered. All filters
  # are ANDed; an empty query returns everything (capped).
  getStates @3 (req :StateQuery) -> (resp :StateList);

  # Historical state timeline (GET /api/history/period) — the
  # diagnostic read: "when did the door last open?", "why is the
  # hallway cold?", "summarize overnight". Read-gated. Specify
  # entityIds — an unfiltered house-wide history is enormous.
  history @8 (req :HistoryQuery) -> (resp :HistoryResponse);

  # Idempotent drain: entities updated since a cursor (last_updated
  # millis), oldest-first. The Sink calls this after a `watch`
  # notification to learn WHAT changed (the notification carries only
  # the cursor).
  changesSince @4 (req :ChangesSinceRequest) -> (resp :ChangesSinceResponse);

  # Subscribe durably to entity state changes — fires the caller's
  # Trigger (with a cursor) on each coalesced change, surviving
  # hibernation. Returns a `Watch` to stop.
  #
  # Durable subscription (docs/durable-held-caps.md): `notifyMe` is a
  # persistable Trigger the CALLER owns; the bridge `save()`s it once
  # and fires it on each coalesced change, waking a hibernated Sink
  # per its wakeup policy. The optional domain / entityIds filter is
  # frozen into the subscription — an agent can watch just the motion
  # sensors without being woken by every thermostat tick.
  watch @5 (req :WatchRequest) -> (watch :Watch);

  # Render a Home Assistant Jinja template (POST /api/template) and
  # return the text result. Side-effect free and read-gated — use it
  # to push aggregation to HA instead of fetching many states.
  #
  # HA templates read states/attributes, they can't call services, so
  # this is a pure read that pushes aggregation to HA — "how many
  # windows are open" comes back as one token instead of 200 states.
  # A read cap can already see every state, so this widens context
  # economy, not authority.
  renderTemplate @9 (template :Text) -> (result :Text);

  # === Writes ===

  # Call a Home Assistant service (POST /api/services/<domain>/<service>)
  # — turn_on, set_temperature, activate a scene, run a script. HA's
  # one mutating primitive for device/entity actions.
  # Returns the states HA reports changed by the call. Needs `write`;
  # the services that take HA or its host down (homeassistant.restart/
  # stop, hassio.host_reboot/host_shutdown) need `admin`.
  callService @6 (req :ServiceCall) -> (resp :ServiceResult);

  # === Stored config (automations / scripts / scenes / helpers) ===
  #
  # HA's UI-editable config objects don't go through callService — they
  # live behind the config REST API (/api/config/<kind>/config/<id>),
  # the surface the automation editor uses. `kind` is the HA config
  # platform slug ("automation", "script", "scene", "input_boolean",
  # …); `objectId` the item's id. Only storage-mode (UI) objects are
  # reachable — automations hand-written in YAML aren't editable here.

  # Read one stored config item as raw JSON — the automation/script/
  # scene definition the editor sees. Read-gated (needed for read-
  # modify-write). Errors "not found" for an unknown id/kind.
  readConfig @10 (kind :Text, objectId :Text) -> (configJson :Text);

  # Create or replace a stored config item (automation/script/scene/
  # helper) from its full JSON definition. Write-gated; takes effect
  # immediately.
  #
  # `configJson` is the full definition object the editor would submit
  # ({alias, trigger, condition, action} for an automation); HA
  # reloads the platform on write. Severing this cap stops FURTHER
  # edits — it does not roll back an automation already written (same
  # as a revoked notebook cap doesn't un-write its prior edits); the
  # agent can delete what it made while it still holds the cap.
  writeConfig @11 (kind :Text, objectId :Text, configJson :Text) -> ();

  # Delete a stored config item. Write-gated. Errors if absent.
  deleteConfig @12 (kind :Text, objectId :Text) -> ();

  # === Registries (WebSocket-only surfaces) ===
  #
  # HA's structural model of the house, underneath the live states:
  # entities belong to devices, devices (and entities, by override)
  # belong to areas, areas belong to floors, and labels tag anything.
  # REST can't see any of this — /api/states shows only what's live —
  # so these ride the WebSocket config/*_registry/list commands. All
  # read-gated.

  # Floors group areas ("Upstairs"). Empty on instances that never
  # configured floors.
  listFloors @13 () -> (floors :List(Floor));

  # Physical devices: what integration/hardware an entity comes from,
  # and the device→area assignment entities inherit.
  #
  # Filter to the devices one integration owns (`configEntryId`, from
  # listIntegrations), and/or those with no entities left (`onlyEmpty`).
  listDevices @14 (configEntryId :Text, onlyEmpty :Bool) -> (devices :List(Device));

  # List the entity registry: one row per *known* entity, including
  # disabled/hidden ones REST never shows. Filter by domain/area
  # here, then getStates the survivors.
  #
  # Rows carry display name, device and resolved area, category
  # (diagnostic/config vs regular), and labels. The honest source for
  # "all lights in the kitchen".
  listEntities @15 (req :EntityRegistryQuery) -> (entries :List(EntityEntry));

  # User-defined labels (free-form tags on entities/devices/areas).
  listLabels @16 () -> (labels :List(Label));

  # === Dashboards (WebSocket-only surface) ===
  #
  # Lovelace dashboards have no REST API at all; these ride the
  # lovelace/* WebSocket commands. Listing/inspection is read-gated;
  # per-dashboard edits are write-gated and live on the Dashboard
  # sub-cap below; creating/deleting whole dashboards needs `admin`.
  #
  # HA-side caveat: the lovelace save/delete commands are admin-only,
  # so *writes* additionally require the grain's long-lived token to
  # belong to an admin HA user (the config UI's connection test
  # reports this). Reads work with any token.

  # List every dashboard, including a synthesized row for the default
  # dashboard (urlPath ""). `mode` tells the agent what's editable
  # BEFORE it tries.
  #
  # Modes: storage = editable; yaml = read-only (defined in
  # configuration.yaml); autoGenerated = no stored config yet (HA
  # renders it from a strategy) — reading its config errors, and
  # saving one (force=true) converts it to storage mode permanently.
  listDashboards @17 () -> (dashboards :List(DashboardInfo));

  # Registered frontend resources (custom card modules). The agent's
  # answer to "which card types exist beyond the builtins" — a
  # `custom:foo-card` type is only renderable if a resource provides
  # it.
  listResources @18 () -> (resources :List(Resource));

  # Mint a capability scoped to ONE dashboard ("" = the default) —
  # the handle for reading and editing that dashboard's config.
  #
  # This is the attenuation seam: hand an agent dashboard("tablet")
  # and it can iterate on that layout all afternoon while structurally
  # unable to touch any other dashboard (or anything else). The cap
  # inherits this session's read/write rights; it does not check that
  # the dashboard exists yet (get() will say so).
  dashboard @19 (urlPath :Text) -> (dashboard :Dashboard);

  # Create a new (storage-mode) dashboard. Needs `admin`: adding or
  # removing whole dashboards is house-topology, not layout-editing.
  # urlPath must contain a hyphen (an HA rule, e.g. "wall-tablet").
  createDashboard @20 (urlPath :Text, title :Text, icon :Text,
                       requireAdmin :Bool, showInSidebar :Bool) -> ();

  # Delete a dashboard and its stored config. Needs `admin`. The default
  # dashboard ("") cannot be deleted — clearing its config back to
  # auto-generated is Dashboard.save's business, not deletion.
  deleteDashboard @21 (urlPath :Text) -> ();

  # Fetch long-term statistics: HA's downsampled hourly/daily
  # aggregates, kept far beyond raw history's retention. Read-gated —
  # the right read for "graph last month's energy".
  #
  # Rides the WebSocket recorder/statistics_during_period command — a
  # handful of points per day instead of dragging raw history through
  # the model. statisticIds are usually entity ids
  # ("sensor.energy_meter"); external stats ids ("sensor:…") work too
  # where HA has them.
  statistics @22 (req :StatisticsQuery) -> (resp :StatisticsResponse);

  # Subscribe durably to ONE dashboard's stored config changing — by
  # anyone: the user in the HA UI, another agent, this cap. Returns a
  # `Watch` to stop.
  #
  # Same Trigger contract as `watch`; the fired payload carries the
  # urlPath as an extra, and the Sink re-reads via dashboard(urlPath).
  # Change detection is a content-hash probe (poll, accelerated to
  # near-instant by a lovelace_updated push subscription while the
  # grain is awake), so edits made during an outage are caught on the
  # next probe rather than lost.
  watchDashboard @23 (urlPath :Text, notifyMe :Capability) -> (watch :Watch);

  # Subscribe durably to a rendered template's RESULT changing —
  # "wake me when the count of open windows changes" as one watch.
  # Read-gated. Returns a `Watch` to stop.
  #
  # The aggregation-push counterpart to renderTemplate: one watch
  # instead of watching every window sensor and re-deriving. The
  # template is rendered at registration (a bad template fails here,
  # not later) and the fire comes when a re-render differs; the Sink
  # re-reads via renderTemplate. Same probe-plus-push shape as
  # watchDashboard (HA's streaming render_template subscription
  # accelerates the poll while the grain is awake). Read-gated, like
  # renderTemplate: templates read state, they can't call services.
  watchTemplate @24 (template :Text, notifyMe :Capability) -> (watch :Watch);

  # Recent warnings and errors from Home Assistant's own log — the
  # diagnostic read for "why did that automation fail?" or "what is
  # HA complaining about?". Needs `write`: log lines carry hosts, URLs
  # and integration detail a read-only share shouldn't see.
  #
  # Default: HA's grouped system log (Settings → System → Logs), one
  # entry per distinct problem with its count, first/last time, source
  # and traceback. `raw` instead returns the tail of the raw core log
  # (/api/error_log, or on HA OS the Supervisor's journal-backed copy),
  # one record per log line with its continuation (traceback) lines
  # attached.
  errorLog @25 (req :ErrorLogQuery) -> (resp :ErrorLogResponse);

  # === Administration (integrations and registries) ===
  #
  # The surfaces behind HA's Settings → Devices & services: integration
  # options, enabling/disabling and reloading integrations, and editing
  # the entity/device registries. Everything that changes state — or
  # opens an options flow — needs the `admin` right, which `write` does
  # NOT imply (an owner grant carries it). These changes can break the
  # house: disabling an integration drops every entity it provides.
  # The stored token must belong to an HA administrator.

  # The configured integrations (config entries), optionally one domain
  # ("unifi"). Read-gated.
  listIntegrations @26 (domain :Text) -> (integrations :List(Integration));

  # One entity's registry entry (name, area, disabled/hidden, device,
  # platform, unique id, options…) as JSON. Read-gated.
  getEntity @27 (entityId :Text) -> (entryJson :Text);

  # Open an integration's options flow — the form behind its Configure
  # button. Returns the first step: its form schema with the current
  # values as defaults (keep them: they are the only record of the
  # options before you change them), a menu, or an abort. Then call
  # submitOptionsStep until the flow returns `createEntry` (saved) or
  # `abort`; abortFlow discards an open flow without saving. Needs
  # `admin`.
  startOptionsFlow @28 (entryId :Text) -> (step :FlowStep);

  # Submit one step of an open options flow. `inputJson` is an object
  # keyed by the step schema's field names (for a menu step:
  # {"next_step_id": "<option>"}). A rejected input returns the same
  # step again with `errorsJson` set. Needs `admin`.
  submitOptionsStep @29 (flowId :Text, inputJson :Text) -> (step :FlowStep);

  # Discard an open options flow without saving. Needs `admin`.
  abortFlow @30 (flowId :Text) -> ();

  # Disable (true) or re-enable (false) an integration, as the UI's
  # Disable does. Needs `admin`.
  setIntegrationDisabled @31 (entryId :Text, disabled :Bool) -> (requireRestart :Bool);

  # Reload an integration (re-read its config, reconnect). Needs
  # `admin`.
  reloadIntegration @32 (entryId :Text) -> (requireRestart :Bool);

  # Edit an entity's registry entry (rename, disable, hide, area,
  # labels). Returns `previousJson` to pass back for a revert. Needs
  # `admin`.
  #
  # `changesJson` is an object with any of: name, icon, area_id,
  # disabled_by ("user" or null), hidden_by ("user" or null),
  # new_entity_id, labels, aliases (null clears name/icon/area_id).
  # Other keys are refused. `previousJson` holds those same keys'
  # values before the change.
  updateEntity @33 (entityId :Text, changesJson :Text)
    -> (entryJson :Text, previousJson :Text);

  # Edit a device's registry entry: name_by_user, area_id, disabled_by
  # ("user" or null), labels. Same `previousJson` revert contract as
  # updateEntity. Needs `admin`.
  updateDevice @34 (deviceId :Text, changesJson :Text)
    -> (deviceJson :Text, previousJson :Text);

  # Delete entities from the registry — irreversible. Only disabled
  # entities are removed unless `includeEnabled` is set. Up to 1000 per
  # call. Needs `admin`.
  #
  # Disable the source first (e.g. the integration option that creates
  # them): an integration re-creates on its next reload any entity it
  # still provides. Each id is removed independently; `failed` lists
  # the ones that weren't, with why.
  removeEntities @35 (entityIds :List(Text), includeEnabled :Bool)
    -> (removed :List(Text), failed :List(RemoveFailure));

  # Detach devices from one integration — irreversible; the device is
  # deleted once no integration owns it. Refuses devices that still
  # have entities unless `includeWithEntities`. Up to 1000 per call.
  # Needs `admin`.
  #
  # The integration is asked first and may refuse (UniFi keeps clients
  # it still sees). `configEntryId` comes from listIntegrations; a
  # device that integration doesn't own is refused.
  removeDevices @36 (deviceIds :List(Text), configEntryId :Text, includeWithEntities :Bool)
    -> (removed :List(Text), failed :List(DeviceRemoveFailure));

  # Restart Home Assistant. Checks the configuration first and refuses
  # (with HA's errors) if it's invalid. Returns once the restart is
  # under way — or, with `waitUntilBack`, once HA has gone down and
  # answers again (up to ~6 minutes; the error says which step didn't
  # happen). callService(homeassistant, restart|stop) also returns as
  # soon as HA starts going down. Needs `admin` — as do
  # callService's homeassistant.restart/stop and hassio.host_reboot/
  # host_shutdown.
  restartHomeAssistant @43 (waitUntilBack :Bool) -> (status :Text);

  # --- Areas and floors ---
  #
  # Areas are rooms; floors group areas. Read them with listAreas /
  # listFloors; put a device or entity in an area with updateDevice /
  # updateEntity (area_id). All of these need `admin`.

  # Create an area. `fieldsJson` ("" for none) may set: floor_id, icon
  # ("mdi:sofa"), aliases (voice-assistant names), labels, picture,
  # temperature_entity_id, humidity_entity_id. HA derives the area id
  # from the name; it's in the returned `areaJson`.
  createArea @44 (name :Text, fieldsJson :Text) -> (areaJson :Text);

  # Edit an area: any of name, floor_id, icon, aliases, labels, picture,
  # temperature_entity_id, humidity_entity_id (null clears floor_id,
  # icon, picture and the sensors). `previousJson` holds those keys'
  # values before, to pass back for a revert.
  updateArea @45 (areaId :Text, changesJson :Text)
    -> (areaJson :Text, previousJson :Text);

  # Delete an area. HA unassigns everything in it; the result lists the
  # devices that were assigned to it and the entities that overrode
  # their device's area with it — what to reassign to rebuild it.
  deleteArea @46 (areaId :Text)
    -> (areaJson :Text, deviceIds :List(Text), entityIds :List(Text));

  # Create a floor. `fieldsJson` may set icon, level (0 = ground,
  # negative = basement) and aliases.
  createFloor @47 (name :Text, fieldsJson :Text) -> (floorJson :Text);

  # Edit a floor: name, icon, level, aliases. Same revert contract as
  # updateArea.
  updateFloor @48 (floorId :Text, changesJson :Text)
    -> (floorJson :Text, previousJson :Text);

  # Delete a floor; its areas stay, on no floor. `areaIds` lists them.
  deleteFloor @49 (floorId :Text) -> (floorJson :Text, areaIds :List(Text));

  # --- HACS (the Home Assistant Community Store) ---
  #
  # Custom integrations, dashboard cards, themes and scripts installed
  # from GitHub through HACS, which must itself be installed in HA.
  # Categories: "integration", "plugin" (dashboard cards/resources),
  # "theme", "python_script", "appdaemon", "template". Repositories are
  # named by HACS's `id`, from hacsList. Listing is read-gated; anything
  # that changes what's installed needs `admin`.

  # Search HACS's store (thousands of repositories — filter): one
  # category ("" = all), installed ones only, and/or a case-insensitive
  # `query` over name, owner/repo, domain and description. Installed
  # first, then most-starred; `limit` 0 = 50, at most 500. `total`
  # counts every match. An installed repo with `pendingUpgrade` has a
  # newer `availableVersion`.
  hacsList @37 (category :Text, installedOnly :Bool, query :Text, limit :UInt32)
    -> (repositories :List(HacsRepository), total :UInt32);

  # One repository and its GitHub releases, newest first (empty for
  # one that ships from a branch). `refresh` makes HACS re-check GitHub
  # first — for "is there an update?". Read-gated.
  hacsRepository @38 (repositoryId :Text, refresh :Bool)
    -> (repository :HacsRepository, releases :List(HacsRelease));

  # Install a repository, or move an installed one to another version
  # (upgrade, downgrade, pin). `version` is a release tag from
  # hacsRepository, or "" for the latest HACS offers. Refused when HACS
  # says the running HA can't take it. `note` says what's left before it
  # takes effect — an integration needs `restartRequired` (a Home
  # Assistant restart); a card needs a browser reload. Needs `admin`.
  hacsInstall @39 (repositoryId :Text, version :Text)
    -> (repository :HacsRepository, restartRequired :Bool, note :Text);

  # Uninstall a repository: HACS deletes its files (it stays listed, so
  # hacsInstall can bring it back). An integration's config entries stay
  # behind and fail to load until deleted. Needs `admin`.
  hacsUninstall @40 (repositoryId :Text)
    -> (repository :HacsRepository, restartRequired :Bool, note :Text);

  # Add a custom repository — a GitHub repo outside HACS's default
  # store — as `category`. `url` is its GitHub URL or "owner/repo".
  # Only lists it; follow with hacsInstall. Needs `admin`.
  hacsAddRepository @41 (url :Text, category :Text) -> (repository :HacsRepository);

  # Drop a custom repository from HACS's list (uninstall it first).
  # Default-store repositories can't be dropped. Needs `admin`.
  hacsRemoveRepository @42 (repositoryId :Text) -> ();

  # One dashboard, held as a capability. All config reads return a
  # `hash` — sha256 of the canonical stored JSON — and every mutation
  # takes an `expectedHash`: the bridge re-reads the live config
  # immediately before writing and fails with "conflict" if it no
  # longer matches, so a user editing in the HA UI mid-conversation
  # turns a stale agent write into a clean re-read instead of a
  # silent clobber. Pass "" to skip the guard (create-from-scratch).
  #
  # Every successful mutation first snapshots the previous config into
  # the grain's private state (a small per-dashboard ring), so a bad
  # edit is one revert() away — severing the cap stops future edits,
  # the ring undoes past ones.
  interface Dashboard {
    # Read the stored config (plus its hash). summary=true returns a
    # compact skeleton instead of the full document — survey first,
    # fetch full only if a whole-document edit is really needed.
    #
    # The skeleton carries per-view title/path/type/icon, card counts,
    # card types, and referenced entity ids — dashboards are unbounded
    # and a full doc can flood the model's context (the card verbs
    # below never need the full doc client-side). Errors with
    # "auto-generated" for a strategy dashboard that has no stored
    # config yet.
    get @0 (summary :Bool) -> (configJson :Text, hash :Text);

    # Replace the whole dashboard document — the big hammer; prefer
    # the card verbs for card-level work. Write-gated.
    #
    # Rides lovelace/config/save. The bridge validates shape (an
    # object with a views list or a strategy; every card an object
    # with a non-empty type) and trips a guard if the new document
    # drops more than half the existing cards — both the "model
    # emitted half a document" tripwire and the take-control-of-an-
    # auto-generated-dashboard step require force=true to proceed.
    save @1 (configJson :Text, expectedHash :Text, force :Bool)
      -> (hash :Text);

    # Card-level verbs: the bridge does the read-modify-write, so the
    # wire op (and its audit row) is one card, not the whole document.
    # Addressing: `view` matches a view's path, then its title
    # (case-insensitive), then parses as a 0-based index. `section`
    # picks a section in a sections-type view (-1 = the view's own
    # cards list, the classic layout). `card` is the 0-based index
    # within that list (get(summary=true) shows the counts).

    # Insert a card. position -1 appends; otherwise inserts at that
    # index (clamped). Returns the index it landed at.
    addCard @2 (view :Text, section :Int32 = -1, cardJson :Text,
                position :Int32 = -1, expectedHash :Text)
      -> (hash :Text, index :UInt32);

    # Replace one card wholesale (fetch it via get, edit, put back).
    updateCard @3 (view :Text, section :Int32 = -1, card :UInt32,
                   cardJson :Text, expectedHash :Text) -> (hash :Text);

    removeCard @4 (view :Text, section :Int32 = -1, card :UInt32,
                   expectedHash :Text) -> (hash :Text);

    # Move a card, within or across views/sections. The card is
    # removed first, then inserted at `position` in the destination
    # (-1 = append; same-list moves index against the post-removal
    # list).
    moveCard @5 (view :Text, section :Int32 = -1, card :UInt32,
                 toView :Text, toSection :Int32 = -1,
                 position :Int32 = -1, expectedHash :Text)
      -> (hash :Text);

    # Restore the most recent snapshot from the grain's ring, pushing
    # the current config onto the ring first (so revert is itself
    # revertible). Write-gated. Errors when the ring is empty.
    revert @6 () -> (hash :Text);

    # Update the dashboard's sidebar metadata (title/icon/visibility).
    # Write-gated; errors for the default dashboard (it has no entry
    # in the dashboards collection).
    setMeta @7 (title :Text, icon :Text, requireAdmin :Bool,
                showInSidebar :Bool) -> ();
  }

  # Handle to a live subscription from `watch`.
  interface Watch {
    # End it: forget the saved Trigger + drop the bridge's watch state.
    # Idempotent.
    stop @0 ();
  }

  # === Types ===

  struct Info {
    version @0 :Text;       # HA core version, e.g. "2026.6.1"
    locationName @1 :Text;  # the instance's friendly name
    timeZone @2 :Text;
  }

  struct Area {
    id @0 :Text;             # area id ("living_room")
    name @1 :Text;           # display name ("Living Room")
    entityIds @2 :List(Text);
    floorId @3 :Text;        # "" when the area is on no floor
  }

  struct Floor {
    id @0 :Text;             # floor id ("upstairs")
    name @1 :Text;           # display name ("Upstairs")
    level @2 :Int32;         # ordering hint (ground = 0); 0 if unset
  }

  struct Device {
    id @0 :Text;
    name @1 :Text;           # user-given name, else the integration's
    manufacturer @2 :Text;
    model @3 :Text;
    areaId @4 :Text;         # "" when unassigned
    # The integrations (config entry ids) that own this device.
    configEntries @5 :List(Text);
    # The device's identifiers, as "domain:value" (e.g. "unifi:aa:bb:…").
    identifiers @6 :List(Text);
    # Registry entities attached to it, disabled ones included.
    entityCount @7 :UInt32;
  }

  struct EntityRegistryQuery {
    # Restrict to one domain ("light"). Empty = all.
    domain @0 :Text;
    # Area id or display name; matches the entity's RESOLVED area
    # (its own override, else its device's). Empty = no area filter.
    area @1 :Text;
    # Registry rows for disabled entities are omitted unless set —
    # they have no live state, so they only matter for "what could
    # exist" questions.
    includeDisabled @2 :Bool;
  }

  struct EntityEntry {
    entityId @0 :Text;
    name @1 :Text;           # display name (user override, else original)
    deviceId @2 :Text;       # "" for device-less entities
    areaId @3 :Text;         # resolved: own override, else device's, else ""
    disabled @4 :Bool;
    hidden @5 :Bool;
    # "" for a regular entity; "diagnostic" or "config" for the
    # housekeeping entities a generated dashboard should usually skip
    # (signal strength, firmware toggles).
    category @6 :Text;
    labels @7 :List(Text);   # label ids (see listLabels)
  }

  struct Label {
    id @0 :Text;
    name @1 :Text;
  }

  struct DashboardInfo {
    urlPath @0 :Text;        # "" = the default dashboard
    title @1 :Text;
    icon @2 :Text;
    mode @3 :DashboardMode;
    requireAdmin @4 :Bool;   # HA hides it from non-admin users
    showInSidebar @5 :Bool;
  }

  enum DashboardMode {
    storage @0;        # UI-editable — the normal case
    yaml @1;           # defined in YAML config; read-only here
    autoGenerated @2;  # strategy-rendered, no stored config yet;
                       # saving (force=true) takes control permanently
  }

  struct Resource {
    url @0 :Text;            # e.g. "/hacsfiles/mushroom/mushroom.js"
    kind @1 :Text;           # "module", "js", "css"
  }

  struct Integration {
    entryId @0 :Text;
    domain @1 :Text;
    title @2 :Text;
    # "loaded", "setup_error", "setup_retry", "not_loaded", …
    state @3 :Text;
    # Why it isn't loaded, when HA says.
    reason @4 :Text;
    # "" when enabled; otherwise who disabled it ("user", …).
    disabledBy @5 :Text;
    # Whether startOptionsFlow applies.
    supportsOptions @6 :Bool;
    source @7 :Text;
  }

  struct HacsRepository {
    # HACS's id for it — what the other hacs* methods take.
    id @0 :Text;
    fullName @1 :Text;          # "owner/repo" on GitHub
    name @2 :Text;
    category @3 :Text;
    description @4 :Text;
    installed @5 :Bool;
    installedVersion @6 :Text;
    availableVersion @7 :Text;
    # Installed and `availableVersion` is newer.
    pendingUpgrade @8 :Bool;
    # False when HACS won't install it here (e.g. too old an HA — see
    # minHomeAssistant).
    canDownload @9 :Bool;
    # Added with hacsAddRepository rather than from the default store.
    custom @10 :Bool;
    # An integration's HA domain ("yoto"), for listIntegrations.
    domain @11 :Text;
    stars @12 :UInt32;
    # HACS's status: "installed", "pending-upgrade", "new", "default"…
    status @13 :Text;
    minHomeAssistant @14 :Text;
    lastUpdated @15 :Text;
  }

  struct HacsRelease {
    tag @0 :Text;               # what hacsInstall's `version` takes
    name @1 :Text;
    publishedAt @2 :Text;
    prerelease @3 :Bool;
  }

  struct DeviceRemoveFailure {
    deviceId @0 :Text;
    reason @1 :Text;
  }

  struct RemoveFailure {
    entityId @0 :Text;
    reason @1 :Text;
  }

  struct FlowStep {
    flowId @0 :Text;
    # "form", "menu", "createEntry" (saved — the flow is done),
    # "abort", or HA's raw type for anything else.
    kind @1 :Text;
    stepId @2 :Text;
    # The form's fields as HA serializes them: [{name, type, required,
    # default?, description: {suggested_value?}, options?, …}].
    schemaJson @3 :Text;
    # Per-field errors from the last submit ({} when none).
    errorsJson @4 :Text;
    # A menu step's choices (submit {"next_step_id": <choice>}).
    menuOptions @5 :List(Text);
    # An abort's reason, or a saved entry's title.
    reason @6 :Text;
    # The whole flow result, for anything the fields above don't carry.
    rawJson @7 :Text;
  }

  struct ErrorLogQuery {
    # Lowest level to include: "debug", "info", "warning", "error",
    # "critical". "" = "warning".
    minLevel @0 :Text;
    # Case-insensitive substring filter over the logger name, message
    # and traceback (raw: the whole record). "" = no filter.
    contains @1 :Text;
    # Most entries (or raw records) to return, newest last. 0 = 50;
    # capped at 500.
    limit @2 :UInt32;
    # Tail the raw log file instead of the grouped system log.
    raw @3 :Bool;
  }

  struct ErrorLogResponse {
    # Grouped entries (when `raw` is false), oldest first.
    entries @0 :List(LogEntry);
    # Raw log records (when `raw` is true), oldest first.
    rawRecords @1 :List(Text);
    # How many entries/records matched before `limit` was applied.
    matched @2 :UInt32;
  }

  struct LogEntry {
    level @0 :Text;
    # The logger, e.g. "homeassistant.components.zha".
    logger @1 :Text;
    # The distinct messages grouped under this entry.
    messages @2 :List(Text);
    # "path/to/file.py:123".
    source @3 :Text;
    count @4 :UInt32;
    firstMillis @5 :Int64;
    lastMillis @6 :Int64;
    # Traceback text, if the entry carried one.
    exception @7 :Text;
  }

  struct StatisticsQuery {
    # Which series to fetch. Must be non-empty — an unfiltered pull
    # would be every statistic HA has ever recorded.
    statisticIds @0 :List(Text);
    # Window bounds (unix millis). 0 end = now. Start is required —
    # statistics exist to span long ranges, so there is no sane
    # implicit window.
    startMillis @1 :Int64;
    endMillis @2 :Int64;
    # Bucket size: "5minute", "hour", "day", "week", "month".
    # "" = "hour".
    period @3 :Text;
  }

  struct StatisticsResponse {
    # One series per requested id, in request order; a series HA has
    # no data for comes back with empty points.
    series @0 :List(StatisticsSeries);
  }

  struct StatisticsSeries {
    statisticId @0 :Text;
    points @1 :List(StatisticsPoint);
  }

  struct StatisticsPoint {
    startMillis @0 :Int64;   # bucket start
    endMillis @1 :Int64;     # bucket end
    # The aggregates HA recorded for this bucket, raw JSON — which
    # keys exist depends on the statistic's kind (a temperature has
    # mean/min/max, an energy meter has sum/state/change).
    statsJson @2 :Text;
  }

  struct ServiceDomain {
    domain @0 :Text;         # "light"
    services @1 :List(Text); # ["turn_on", "turn_off", "toggle", …]
  }

  struct StateQuery {
    # Restrict to one domain ("light", "sensor"). Empty = all.
    domain @0 :Text;
    # Exact entity ids ("light.kitchen"). Empty = no id filter.
    entityIds @1 :List(Text);
    # Area id or display name; resolved via the area registry. Empty =
    # no area filter.
    area @2 :Text;
    # Max states returned. 0 = bridge default (500), capped at 2000.
    limit @3 :UInt32;
    # Drop attributesJson (return "{}") — the cheap "what exists /
    # what's its state" pass; then getStates the handful you care
    # about with attributes.
    #
    # Entity attributes are unbounded — media artwork, whole weather
    # forecasts — and a full-house survey floods the model's context.
    omitAttributes @4 :Bool;
  }

  struct EntityState {
    entityId @0 :Text;
    state @1 :Text;              # "on", "22.5", "home", …
    friendlyName @2 :Text;       # attributes.friendly_name, "" if unset
    # Full attributes object as raw JSON — attribute shapes are
    # per-integration and unbounded, so they cross the wire opaque.
    attributesJson @3 :Text;
    lastChangedMillis @4 :Int64; # state VALUE last changed (unix millis)
    lastUpdatedMillis @5 :Int64; # state or attributes last updated
  }

  struct StateList {
    states @0 :List(EntityState);
    # True when `limit` cut the result off (more matches exist).
    truncated @1 :Bool;
  }

  struct ChangesSinceRequest {
    sinceMillis @0 :Int64;   # exclusive cursor; 0 = from the beginning
    limit @1 :UInt32;        # 0 = bridge default
    # Optional filter, same semantics as StateQuery (no area — the
    # cursor drain stays a single /api/states read).
    domain @2 :Text;
    entityIds @3 :List(Text);
    # Drop attributesChanged events, keeping only state-value changes
    # — most consumers care that the door OPENED, not that its
    # attributes wobbled.
    #
    # HA entities are internally chatty (a media_player's position, a
    # sensor's signal strength). Skipped events don't advance the
    # cursor and are simply re-skipped on the next drain — still
    # lossless for what's kept.
    stateChangesOnly @4 :Bool;
  }

  struct ChangesSinceResponse {
    changes @0 :List(ChangeEvent);
    # Pass back as the next `sinceMillis` (max last_updated kept, or
    # the request cursor when nothing changed) — lossless polling.
    nextCursor @1 :Int64;
  }

  struct ChangeEvent {
    kind @0 :EventKind;
    state @1 :EntityState;
  }

  enum EventKind {
    stateChanged @0;       # the state value changed (last_changed == last_updated)
    attributesChanged @1;  # attributes-only update
  }

  struct WatchRequest {
    # The caller's persistable Trigger (a bare Capability so this
    # schema needs no imports; the bridge casts it).
    notifyMe @0 :Capability;
    sinceMillis @1 :Int64;  # starting cursor; 0 = changes from now
    # Frozen filter: fire only for entities matching these (both
    # optional, ANDed — same semantics as ChangesSinceRequest).
    domain @2 :Text;
    entityIds @3 :List(Text);
    # Fire only on state-VALUE changes, sleeping through
    # attribute-only churn on the watched entities. Frozen into the
    # subscription; pass the same flag to changesSince when draining.
    #
    # "Wake me when the door opens, not when its signal strength
    # wobbles." Without the matching flag on changesSince, the drain
    # will show the attribute events the notification deliberately
    # slept through.
    stateChangesOnly @4 :Bool;
  }

  struct ServiceCall {
    domain @0 :Text;          # "light"
    service @1 :Text;         # "turn_on"
    # Target entities, sent as the call's entity_id. Empty is allowed —
    # some services take no target (script.reload) or target via
    # dataJson (area_id, device_id).
    entityIds @2 :List(Text);
    # Extra service data as a raw JSON object ("" = none), merged in as
    # the request body ({"brightness": 128, "area_id": "kitchen"}).
    # entityIds wins over any entity_id key in here.
    dataJson @3 :Text;
  }

  struct ServiceResult {
    # States HA reports were changed by the call. Empty doesn't mean
    # "no effect" — some services report nothing.
    changed @0 :List(EntityState);
  }

  struct HistoryQuery {
    # Entities to fetch. Empty = every entity — allowed by HA but
    # enormous; specify unless you truly want the whole house.
    entityIds @0 :List(Text);
    # Window bounds (unix millis). 0 start = HA's default (1 hour ago);
    # 0 end = now.
    startMillis @1 :Int64;
    endMillis @2 :Int64;
    # HA `minimal_response`: after each entity's first point, emit only
    # state + timestamp (no attributes) for unchanged-attribute points.
    # The context-lean default for "when did X change" questions.
    minimalResponse @3 :Bool;
    # HA `significant_changes_only`: collapse runs of insignificant
    # updates (per HA's own per-domain heuristic). Fewer points, still
    # the meaningful transitions.
    significantChangesOnly @4 :Bool;
  }

  struct HistoryResponse {
    # One timeline per entity, in the order HA returned them.
    series @0 :List(EntityHistory);
  }

  struct EntityHistory {
    entityId @0 :Text;
    points @1 :List(HistoryPoint);
  }

  struct HistoryPoint {
    state @0 :Text;
    atMillis @1 :Int64;       # last_changed (unix millis)
    # Attributes at this point, raw JSON. "{}" for the trimmed points
    # under minimalResponse (HA sends attributes only on changes).
    attributesJson @2 :Text;
  }
}
