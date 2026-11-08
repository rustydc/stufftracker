// Grain-side session machinery (ABI v2, docs/grain-abi-v2.md).
//
// The grain dials the platform-bound UDS at `DARUDE_PLATFORM_SOCKET`
// when its init is done and REGISTERS a Grain cap — registration is
// readiness. Sessions are objects: platformd calls initSession per
// (Principal, rights) tuple over the registered connection and pulls
// interfaces with Session.get; slot wiring arrives once in the
// registration response (plus best-effort slotBound/slotRevoked
// diffs), not per session.
//
// This is the only bootstrap path between platformd and a grain. The
// v1 shape — the grain binding `session.sock`, platformd dialing and
// running a SessionBootstrap.handshake per session — is retired
// (graveyard stage 3); `session.capnp` survives in darude-wire only
// for v1-era standalone dev tools that dial OTHER processes.

use anyhow::{anyhow, Context, Result};
use capnp::capability::{FromClientHook, Promise};
use capnp::private::capability::ClientHook;
use capnp_rpc::pry;
use capnp_rpc::{rpc_twoparty_capnp, twoparty, RpcSystem};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::rc::Rc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

pub use darude_wire::principal_capnp::principal;

/// One session's provides. The factory returns this from its
/// closure; serve_sessions feeds the entries into the handshake
/// response.
#[derive(Default)]
pub struct SessionProvides {
    /// (name, hook, interface_id, schema_bytes). The id + schema are
    /// attached so platformd can introspect/offer the interface without
    /// compiling it in; `add` auto-fills them from the wire schema
    /// registry by name (empty when the name is unknown to this grain's
    /// linked schemas — fine for platform-known interfaces).
    pub entries: Vec<(String, Box<dyn ClientHook>, u64, Vec<u8>)>,
}

impl SessionProvides {
    pub fn new() -> Self {
        Self::default()
    }
    /// Add a provided interface. The interface's type id + CGR schema
    /// bytes are looked up from the wire schema registry by `name` and
    /// shipped in the handshake, so the platform can register them.
    pub fn add(mut self, name: impl Into<String>, hook: Box<dyn ClientHook>) -> Self {
        let name = name.into();
        // Self-describe from the COMPILED schemas (the build artifact this
        // grain links), not the platform's runtime registry — a grain
        // knows its own interface's bytes regardless of what platformd has
        // registered. This is what lets an app interface that platformd
        // doesn't seed still ship its schema at handshake.
        let (id, schema) = darude_wire::compiled_schema_for_interface(&name)
            .map(|(id, bytes)| (id, bytes.to_vec()))
            .unwrap_or((0, Vec::new()));
        self.entries.push((name, hook, id, schema));
        self
    }
    /// Add a provided interface with explicit schema bytes — for a grain
    /// whose interface isn't in `darude_wire` (a true third party). The
    /// grain supplies its own compiled CGR bytes + type id.
    pub fn add_with_schema(
        mut self,
        name: impl Into<String>,
        hook: Box<dyn ClientHook>,
        interface_id: u64,
        schema: Vec<u8>,
    ) -> Self {
        self.entries.push((name.into(), hook, interface_id, schema));
        self
    }
}

/// Requires the grain receives from platformd at handshake time,
/// keyed by the manifest's logical name. Each cap is pre-bound to
/// the session's principal so outgoing calls auto-attribute.
///
/// A singular require has exactly one entry per logical name; a
/// `many:` require carries every current binding, each with a label
/// (offer label / providing sibling's grain id) to tell them apart.
/// Each handshake's set is COMPLETE — consumers replace any cached
/// bindings rather than appending, so a re-handshake after a new
/// offer (or a sever) converges on the platform's current truth.
#[derive(Default)]
pub struct SessionRequires {
    inner: HashMap<String, Vec<(String, Box<dyn ClientHook>)>>,
}

impl SessionRequires {
    /// Take ownership of one of the requires caps by logical name.
    /// Returns None if the grain manifested a require but platformd
    /// didn't supply it for this session (typically: the user
    /// hasn't been granted access to that backend). For a `many:`
    /// logical use `take_all`; this takes the first binding and
    /// warns if others were silently dropped.
    pub fn take<T: FromClientHook>(&mut self, logical: &str) -> Option<T> {
        let mut entries = self.inner.remove(logical)?;
        if entries.len() > 1 {
            tracing::warn!(
                logical,
                n = entries.len(),
                "SessionRequires::take on a multi-binding logical; \
                 dropping all but the first (use take_all)"
            );
        }
        if entries.is_empty() {
            return None;
        }
        Some(FromClientHook::new(entries.swap_remove(0).1))
    }
    /// Take every binding of a `many:` logical name, as (label, cap)
    /// pairs in delivery order. Empty vec = the slot exists but holds
    /// no bindings for this session.
    pub fn take_all<T: FromClientHook>(&mut self, logical: &str) -> Vec<(String, T)> {
        self.inner
            .remove(logical)
            .unwrap_or_default()
            .into_iter()
            .map(|(label, hook)| (label, FromClientHook::new(hook)))
            .collect()
    }
    /// Peek without consuming — useful when the same cap feeds
    /// multiple Server impls inside one session.
    pub fn get<T: FromClientHook>(&self, logical: &str) -> Option<T> {
        self.inner
            .get(logical)
            .and_then(|v| v.first())
            .map(|(_, h)| FromClientHook::new(h.add_ref()))
    }
    pub fn logical_names(&self) -> impl Iterator<Item = &str> {
        self.inner.keys().map(|s| s.as_str())
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// PULL the grain's slot bindings from `Self.held()` and merge them
    /// in, replacing any pushed entry of the same logical
    /// (docs/self-discovery.md step 5: initial bindings are a pull; the
    /// handshake pushes only the platform-synth bootstrap). Call from
    /// the factory right after taking the `self` require:
    ///
    ///   if let Some(sc) = ctx.requires.take::<self_::Client>("self") {
    ///       ctx.requires.pull_from_self(&sc).await;
    ///       // keep sc for watch/get/restore …
    ///   }
    ///
    /// Rules, mirroring the retired platform-side resolver:
    ///   - a binding whose rights don't cover the slot's declared
    ///     rights is skipped (never silently wire a downgraded
    ///     backend);
    ///   - the self.v1 slot is left alone (the caller already took it);
    ///   - a declared-but-empty slot REMOVES a stale entry, so a
    ///     rewire-to-unbound is honest.
    ///
    /// Best-effort: a discovery error leaves the map untouched.
    pub async fn pull_from_self(&mut self, sc: &darude_wire::self_v1_capnp::self_::Client) {
        use capnp::traits::HasTypeId;
        let self_id = <darude_wire::self_v1_capnp::self_::Client as HasTypeId>::TYPE_ID;
        let resp = match sc.held_request().send().promise.await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("pull_from_self: self.held: {e}");
                return;
            }
        };
        let slots = match resp.get().and_then(|r| r.get_slots()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("pull_from_self: slots: {e}");
                return;
            }
        };
        for s in slots.iter() {
            let name = match s.get_name().ok().and_then(|t| t.to_str().ok()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => continue,
            };
            let iface = s
                .get_interface()
                .ok()
                .and_then(|t| t.to_str().ok())
                .unwrap_or("");
            if s.get_interface_id() == self_id || iface == "self.v1" {
                continue;
            }
            let declared: Vec<String> = s
                .get_rights()
                .map(|r| {
                    r.iter()
                        .filter_map(|t| t.ok().and_then(|t| t.to_str().ok()).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let Ok(bindings) = s.get_bindings() else {
                continue;
            };
            let mut entries: Vec<(String, Box<dyn ClientHook>)> = Vec::new();
            for b in bindings.iter() {
                if !b.has_cap() {
                    continue;
                }
                let granted: Vec<String> = b
                    .get_rights()
                    .map(|r| {
                        r.iter()
                            .filter_map(|t| {
                                t.ok().and_then(|t| t.to_str().ok()).map(str::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(missing) = declared.iter().find(|r| !granted.iter().any(|g| &g == r)) {
                    tracing::warn!(
                        slot = %name, %missing,
                        "pull_from_self: binding lacks a declared right; skipping"
                    );
                    continue;
                }
                let label = b
                    .get_label()
                    .ok()
                    .and_then(|t| t.to_str().ok())
                    .filter(|l| !l.is_empty())
                    .unwrap_or(&name)
                    .to_string();
                // The phantom type is irrelevant — the hook is the cap.
                let Ok(carrier) = b
                    .get_cap()
                    .get_as_capability::<darude_wire::self_v1_capnp::self_::Client>()
                else {
                    continue;
                };
                entries.push((label, carrier.into_client_hook()));
            }
            tracing::debug!(slot = %name, n = entries.len(), "pull_from_self: slot merged");
            if entries.is_empty() {
                self.inner.remove(&name);
            } else {
                self.inner.insert(name, entries);
            }
        }
    }
}

/// The Principal struct the factory receives, as plain Rust strings.
/// Mirrors the schema's Principal — the helper does the wire
/// extraction once so each grain doesn't have to.
#[derive(Debug, Clone, Default)]
pub struct PrincipalInfo {
    pub username: String,
    pub display_name: String,
}

impl PrincipalInfo {
    /// `display_name`, falling back to `username` when blank. Useful
    /// for backends (wave) that need a human-readable handle.
    pub fn display(&self) -> &str {
        if self.display_name.is_empty() {
            &self.username
        } else {
            &self.display_name
        }
    }
}

/// Everything one session needs: identity, rights, and the
/// slot-delivered requires caps.
///
/// Two channels share this shape (and the one factory):
///   - the PLATFORM CHANNEL (`is_platform == true`): the session
///     platformd itself opens, once per boot, right after
///     registration — the same context the registration-time announce
///     pass runs the factory under;
///   - USER SESSIONS (`is_platform == false`), per (principal,
///     rights): an attribution + rights view.
///
/// Both see the same `requires`: the registered slot snapshot, cloned
/// per session (`requires_from`). Grains that stash bindings in
/// instance state (`if let Some(x) = requires.take(..)`) work
/// unchanged — every factory run sees the full snapshot.
///
/// (The push-era `restored` re-vend is gone: a grain re-derives its
/// saved caps via `Self.restore(token)` — `WatchSet::rearm_from_self`.)
pub struct SessionContext {
    pub principal: PrincipalInfo,
    pub rights: Vec<String>,
    pub requires: SessionRequires,
    pub is_platform: bool,
}

/// Factory closure: given a SessionContext, return a future that
/// yields the provides that session should see. The grain typically
/// constructs fresh Server impls here (one per provided interface),
/// each holding the principal + rights + requires as their own
/// state so method dispatch sees them as ambient data.
///
/// Async so grains that need to do setup work per session (open a
/// backend WebSocket, hit a sub-service, etc) can do it before the
/// handshake reply lands. The factory runs inside the
/// SessionBootstrap.handshake Promise so blocking it just delays
/// that one session's first method call — other sessions stay
/// independent.
///
/// Rights filtering is the factory's job — the grain knows which
/// interfaces honor which rights. The platform passes the grant's
/// rights verbatim; the factory decides what to include.
pub type SessionFactory =
    Rc<dyn Fn(SessionContext) -> Pin<Box<dyn Future<Output = Result<SessionProvides>>>> + 'static>;

/// `serve_grain` with an empty profile — the entry point for grains
/// with no runtime slot declarations. Dials `DARUDE_PLATFORM_SOCKET`
/// and registers; each initSession invokes `factory` with the
/// session's context.
///
/// Returns when the platform connection ends (typically when the
/// process is shutting down). Apps usually spawn_local this on their
/// LocalSet and let it run for the process's lifetime.
pub async fn serve_sessions(factory: SessionFactory) -> Result<()> {
    serve_grain(factory, GrainProfile::default()).await
}

/// A slot this grain declares at runtime (GrainCtl.declareSlot) — a
/// needs-claim beyond the manifest, persisted platform-side so the
/// powerbox can render and validate offers with the grain asleep.
/// `interface = ""` declares an UNTYPED slot: it accepts any
/// interface, and only an explicit powerbox pick lands in it (never
/// auto-routed).
#[derive(Clone, Debug, Default)]
pub struct SlotDecl {
    pub name: String,
    pub interface: String,
    /// Minimum rights a binding must carry.
    pub rights: Vec<String>,
    pub many: bool,
    /// Powerbox hint ("formula tools").
    pub doc: String,
}

/// A slot wiring diff, delivered to `GrainProfile.on_slots_changed`
/// AFTER the SlotSet already reflects it. Best-effort UX (a chat's
/// "a new tool appeared", grist's formula table): correctness never
/// depends on it — rows are the truth and per-factory/per-turn pulls
/// reconcile. This is `Self.watch`'s replacement (graveyard stage 2):
/// slot diffs cover every delivery now that the offer fork is closed.
#[derive(Clone, Debug)]
pub struct SlotEvent {
    pub slot: String,
    pub label: String,
    /// true = bound; false = revoked.
    pub bound: bool,
}

/// What a grain asserts about itself at registration, beyond its
/// manifest: runtime slot declarations plus the registration-time
/// callbacks. The declarations ride the registerGrain call itself, so
/// the platform records them before readiness fires.
#[derive(Clone, Default)]
pub struct GrainProfile {
    pub slots: Vec<SlotDecl>,
    /// Called once, right after registration, with the provisioned
    /// Principal — Some only when the manifest declares
    /// `embodied: true` (docs/grain-abi-v2.md, "Principal"): identity
    /// arrives at birth, not through wiring. None for everyone else.
    pub on_registered: Option<Rc<dyn Fn(Option<darude_wire::grain_v2_capnp::principal::Client>)>>,
    /// Called on each slotBound/slotRevoked diff (see SlotEvent).
    pub on_slots_changed: Option<Rc<dyn Fn(SlotEvent)>>,
}

/// `serve_sessions` with a GrainProfile: the entry point for grains
/// that declare runtime slots. ABI v2 (docs/grain-abi-v2.md): dial
/// the platform-bound platform.sock and REGISTER — registration is
/// readiness. The env var is set iff the platform saw a capnp surface
/// on this grain (api/exports/hooks/interface requires — see
/// grain_needs_rpc); running without it is a packaging error (the v1
/// session ABI is retired), reported as such instead of a silent
/// no-serve.
pub async fn serve_grain(factory: SessionFactory, profile: GrainProfile) -> Result<()> {
    let platform_sock = std::env::var("DARUDE_PLATFORM_SOCKET").map_err(|_| {
        anyhow!(
            "serve_grain: DARUDE_PLATFORM_SOCKET not set — the platform binds it \
             only for grains whose manifest shows a capnp surface (an `api:`, \
             `exports:`, `hooks:`, or an interface-typed require); this grain's \
             manifest shows none (docs/grain-abi-v2.md)"
        )
    })?;
    serve_grain_v2(Path::new(&platform_sock), factory, profile).await
}

// ---------------------------------------------------------------------------
// ABI v2: dial-in registration (docs/grain-abi-v2.md).
//
// The grain dials platform.sock when its init is done and registers a
// Grain cap; the register call arriving IS readiness. Slots (the
// resolved manifest requires) come back in the response and are held
// for every subsequent initSession — v2 delivers wiring ONCE at the
// sync point, not per session. The same SessionFactory drives both
// ABIs: initSession builds the v1-shaped SessionContext from the
// session's principal/rights plus the registered slots, and the
// factory's provides back the Session.get pull.

/// The registered slot set, shared between registration (writer) and
/// initSession (readers). logical → [(label, hook)].
type SlotSet = Rc<std::cell::RefCell<HashMap<String, Vec<(String, Box<dyn ClientHook>)>>>>;

/// Gate between handing the platform our Grain cap (inside the
/// registerGrain REQUEST) and having stored the slot snapshot (from
/// its RESPONSE). The platform's readiness fires inside its handler
/// — BEFORE the response reaches us — so an eager initSession (the
/// spawn path opens the platform channel the instant readiness
/// lands) can arrive while the SlotSet is still empty. Handlers wait
/// here so every factory run sees the snapshot.
struct RegGate {
    ready: std::cell::Cell<bool>,
    notify: tokio::sync::Notify,
}

impl RegGate {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            ready: std::cell::Cell::new(false),
            notify: tokio::sync::Notify::new(),
        })
    }
    fn open(&self) {
        self.ready.set(true);
        self.notify.notify_waiters();
    }
    async fn wait(&self) {
        while !self.ready.get() {
            self.notify.notified().await;
        }
    }
}

pub async fn serve_grain_v2(
    platform_sock: &Path,
    factory: SessionFactory,
    profile: GrainProfile,
) -> Result<()> {
    use darude_wire::grain_v2_capnp::{bootstrap, grain_ctl};
    let stream = tokio::net::UnixStream::connect(platform_sock)
        .await
        .with_context(|| format!("dial {}", platform_sock.display()))?;
    let (read, write) = stream.into_split();
    let network = twoparty::VatNetwork::new(
        futures::io::BufReader::new(read.compat()),
        futures::io::BufWriter::new(write.compat_write()),
        rpc_twoparty_capnp::Side::Client,
        Default::default(),
    );
    let mut rpc_system = RpcSystem::new(Box::new(network), None);
    let bootstrap: bootstrap::Client = rpc_system.bootstrap(rpc_twoparty_capnp::Side::Server);
    let driver = tokio::task::spawn_local(rpc_system);

    let slots: SlotSet = Rc::new(std::cell::RefCell::new(HashMap::new()));
    let gate = RegGate::new();
    let grain_impl = GrainV2Impl {
        factory: factory.clone(),
        slots: slots.clone(),
        gate: gate.clone(),
        on_slots_changed: profile.on_slots_changed.clone(),
    };
    let grain_client: darude_wire::grain_v2_capnp::grain::Client =
        capnp_rpc::new_client(grain_impl);

    let mut req = bootstrap.register_grain_request();
    req.get().set_grain(grain_client);
    {
        // The grain's needs-claims ride the register call itself (the
        // manifest exit): the platform records them BEFORE readiness
        // fires, so an offer that awaited this registration can route
        // against the slots immediately — one sync point.
        let mut decls = req.get().init_slot_decls(profile.slots.len() as u32);
        for (i, s) in profile.slots.iter().enumerate() {
            let mut d = decls.reborrow().get(i as u32);
            d.set_name(&s.name);
            d.set_interface(&s.interface);
            d.set_many(s.many);
            d.set_doc(&s.doc);
            let mut r = d.init_rights(s.rights.len() as u32);
            for (j, right) in s.rights.iter().enumerate() {
                r.set(j as u32, right.as_str());
            }
        }
    }
    let resp = req
        .send()
        .promise
        .await
        .map_err(|e| anyhow!("registerGrain: {e}"))?;
    let r = resp.get()?;
    let ctl: grain_ctl::Client = r.get_ctl()?;
    {
        let wire_slots = r.get_slots()?;
        let mut map = slots.borrow_mut();
        for i in 0..wire_slots.len() {
            let s = wire_slots.get(i);
            let slot = s.get_slot()?.to_str()?.to_string();
            let label = s.get_label()?.to_str()?.to_string();
            let label = if label.is_empty() {
                slot.clone()
            } else {
                label
            };
            let cap: darude_wire::accept_cap_capnp::accept_cap::Client =
                s.get_cap().get_as_capability()?;
            map.entry(slot)
                .or_default()
                .push((label, cap.into_client_hook()));
        }
    }
    // Snapshot stored — un-gate the handlers (initSession may already
    // be queued behind this: the platform's readiness fired inside
    // registerGrain, before we processed its response).
    gate.open();
    tracing::info!(
        slots = ?slots.borrow().keys().collect::<Vec<_>>(),
        "grain v2: registered"
    );

    // Identity at birth: an `embodied: true` manifest gets its
    // Principal in the registration response; everyone else gets a
    // null field. Delivered through the profile callback so the
    // grain's own state can capture it.
    if let Some(cb) = &profile.on_registered {
        let principal = if r.has_principal() {
            r.get_principal().ok()
        } else {
            None
        };
        cb(principal);
    }

    // Auto-announce this grain's serves-claims: run the factory once
    // with the grain-own (platform) context to learn its provides, and
    // claim each named one. This is what the v1 handshake's provides
    // reader did platform-side; in v2 the grain asserts for itself.
    {
        let ctx = SessionContext {
            principal: PrincipalInfo::default(),
            rights: Vec::new(),
            requires: requires_from(&slots),
            is_platform: true,
        };
        match (factory)(ctx).await {
            Ok(provides) => {
                for (name, _hook, id, schema) in provides.entries {
                    if name.is_empty() {
                        continue;
                    }
                    let mut req = ctl.announce_request();
                    {
                        let mut a = req.get();
                        a.set_name(&name);
                        a.set_interface_id(id);
                        a.set_schema(&schema);
                    }
                    if let Err(e) = req.send().promise.await {
                        tracing::warn!(interface = %name, "grain v2: announce: {e}");
                    }
                }
            }
            Err(e) => tracing::warn!("grain v2: announce factory run: {e}"),
        }
    }

    // Park: the registered connection IS the grain's liveness. If it
    // ends (platformd restart), surface the error so the grain exits
    // and gets relaunched into a fresh registration.
    match driver.await {
        Ok(Ok(())) => Err(anyhow!("grain v2: platform connection closed")),
        Ok(Err(e)) => Err(anyhow!("grain v2: platform connection: {e}")),
        Err(e) => Err(anyhow!("grain v2: driver: {e}")),
    }
}

/// Clone the registered slot set into a per-session SessionRequires —
/// the v1-shaped view factories already consume.
fn requires_from(slots: &SlotSet) -> SessionRequires {
    let mut requires = SessionRequires::default();
    for (logical, entries) in slots.borrow().iter() {
        for (label, hook) in entries {
            requires
                .inner
                .entry(logical.clone())
                .or_default()
                .push((label.clone(), hook.add_ref()));
        }
    }
    requires
}

struct GrainV2Impl {
    factory: SessionFactory,
    slots: SlotSet,
    gate: Rc<RegGate>,
    on_slots_changed: Option<Rc<dyn Fn(SlotEvent)>>,
}

impl darude_wire::grain_v2_capnp::grain::Server for GrainV2Impl {
    fn init_session(
        &mut self,
        params: darude_wire::grain_v2_capnp::grain::InitSessionParams,
        mut results: darude_wire::grain_v2_capnp::grain::InitSessionResults,
    ) -> Promise<(), capnp::Error> {
        let p = pry!(params.get());
        let principal_reader = pry!(p.get_principal());
        let username = pry!(pry!(principal_reader.get_username()).to_str()).to_string();
        let display_name = pry!(pry!(principal_reader.get_display_name()).to_str()).to_string();
        let principal = PrincipalInfo {
            username,
            display_name,
        };
        let rights = pry!(read_rights(pry!(p.get_rights())));
        let is_platform = p.get_is_platform();
        tracing::debug!(user = %principal.username, rights = ?rights, "grain v2: initSession");
        let factory = self.factory.clone();
        let slots = self.slots.clone();
        let gate = self.gate.clone();
        Promise::from_future(async move {
            // The registration snapshot must be stored before any
            // factory run — see RegGate.
            gate.wait().await;
            let ctx = SessionContext {
                principal,
                rights,
                requires: requires_from(&slots),
                is_platform,
            };
            let provides = (factory)(ctx)
                .await
                .map_err(|e| capnp::Error::failed(format!("session factory: {e}")))?;
            let session = SessionV2Impl {
                provides: provides
                    .entries
                    .into_iter()
                    .map(|(name, hook, _, _)| (name, hook))
                    .collect(),
            };
            let client: darude_wire::grain_v2_capnp::session::Client =
                capnp_rpc::new_client(session);
            results.get().set_session(client);
            Ok(())
        })
    }

    fn slot_bound(
        &mut self,
        params: darude_wire::grain_v2_capnp::grain::SlotBoundParams,
        _results: darude_wire::grain_v2_capnp::grain::SlotBoundResults,
    ) -> Promise<(), capnp::Error> {
        let p = pry!(params.get());
        let b = pry!(p.get_binding());
        let slot = pry!(pry!(b.get_slot()).to_str()).to_string();
        if slot.is_empty() {
            return Promise::err(capnp::Error::failed("slotBound needs a slot".into()));
        }
        let label = pry!(pry!(b.get_label()).to_str()).to_string();
        let label = if label.is_empty() {
            slot.clone()
        } else {
            label
        };
        // The phantom type is irrelevant — the hook is the cap.
        let cap: darude_wire::accept_cap_capnp::accept_cap::Client =
            pry!(b.get_cap().get_as_capability());
        let slots = self.slots.clone();
        let gate = self.gate.clone();
        let cb = self.on_slots_changed.clone();
        Promise::from_future(async move {
            // Diffs apply on top of the stored snapshot (RegGate) —
            // otherwise a pre-snapshot diff would duplicate under the
            // snapshot's own entry for the same binding.
            gate.wait().await;
            {
                let mut map = slots.borrow_mut();
                let entries = map.entry(slot.clone()).or_default();
                // Same label = a rebind: replace in place. A new label
                // accumulates (`many:` slots) — for a singular slot the
                // platform unbound the old row, and the next registration's
                // snapshot squares away any drift this best-effort diff left.
                entries.retain(|(l, _)| l != &label);
                entries.push((label.clone(), cap.into_client_hook()));
            }
            tracing::info!(slot = %slot, "grain v2: slot bound");
            if let Some(cb) = cb {
                cb(SlotEvent {
                    slot,
                    label,
                    bound: true,
                });
            }
            Ok(())
        })
    }

    fn slot_revoked(
        &mut self,
        params: darude_wire::grain_v2_capnp::grain::SlotRevokedParams,
        _results: darude_wire::grain_v2_capnp::grain::SlotRevokedResults,
    ) -> Promise<(), capnp::Error> {
        let p = pry!(params.get());
        let slot = pry!(pry!(p.get_slot()).to_str()).to_string();
        let label = pry!(pry!(p.get_label()).to_str()).to_string();
        let slots = self.slots.clone();
        let gate = self.gate.clone();
        let cb = self.on_slots_changed.clone();
        Promise::from_future(async move {
            gate.wait().await;
            {
                let mut map = slots.borrow_mut();
                if let Some(entries) = map.get_mut(&slot) {
                    if label.is_empty() {
                        entries.clear();
                    } else {
                        entries.retain(|(l, _)| l != &label);
                    }
                    if entries.is_empty() {
                        map.remove(&slot);
                    }
                }
            }
            tracing::info!(slot = %slot, label = %label, "grain v2: slot revoked");
            if let Some(cb) = cb {
                cb(SlotEvent {
                    slot,
                    label,
                    bound: false,
                });
            }
            Ok(())
        })
    }
}

struct SessionV2Impl {
    provides: HashMap<String, Box<dyn ClientHook>>,
}

impl darude_wire::grain_v2_capnp::session::Server for SessionV2Impl {
    fn get(
        &mut self,
        params: darude_wire::grain_v2_capnp::session::GetParams,
        mut results: darude_wire::grain_v2_capnp::session::GetResults,
    ) -> Promise<(), capnp::Error> {
        let name = pry!(pry!(pry!(params.get()).get_interface()).to_str()).to_string();
        let mut r = results.get();
        match self.provides.get(&name) {
            Some(hook) => {
                r.set_found(true);
                r.get_cap().set_as_capability(hook.add_ref());
            }
            None => r.set_found(false),
        }
        Promise::ok(())
    }
}

/// Read a `List(Text)` rights field into owned strings.
fn read_rights(rights_reader: capnp::text_list::Reader<'_>) -> capnp::Result<Vec<String>> {
    let mut rights = Vec::with_capacity(rights_reader.len() as usize);
    for i in 0..rights_reader.len() {
        rights.push(rights_reader.get(i)?.to_str()?.to_string());
    }
    Ok(rights)
}

#[cfg(test)]
mod tests {
    use super::*;
    use darude_wire::accept_cap_capnp::accept_cap;

    // Any Server impl works — we only need live hooks to store.
    struct NopAccept;
    impl accept_cap::Server for NopAccept {}

    fn hook() -> Box<dyn ClientHook> {
        let c: accept_cap::Client = capnp_rpc::new_client(NopAccept);
        c.into_client_hook()
    }

    #[test]
    fn take_all_returns_every_labeled_binding() {
        let mut r = SessionRequires::default();
        r.inner
            .entry("providers".to_string())
            .or_default()
            .push(("openai".to_string(), hook()));
        r.inner
            .entry("providers".to_string())
            .or_default()
            .push(("anthropic".to_string(), hook()));
        let all = r.take_all::<accept_cap::Client>("providers");
        assert_eq!(
            all.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(),
            vec!["openai", "anthropic"],
        );
        // Consumed: a second take_all is empty.
        assert!(r.take_all::<accept_cap::Client>("providers").is_empty());
        assert!(r.take_all::<accept_cap::Client>("absent").is_empty());
    }

    #[test]
    fn take_keeps_singular_semantics() {
        let mut r = SessionRequires::default();
        r.inner
            .entry("llm".to_string())
            .or_default()
            .push(("llm".to_string(), hook()));
        assert!(r.take::<accept_cap::Client>("llm").is_some());
        assert!(r.take::<accept_cap::Client>("llm").is_none());
        assert!(r.take::<accept_cap::Client>("absent").is_none());
    }

    // A stub Self whose held() serves a canned slots view: the shapes
    // pull_from_self must handle — a bound provider slot, a slot whose
    // only binding is under-righted, a declared-but-empty slot, and the
    // self.v1 slot itself.
    struct StubSelf;
    impl darude_wire::self_v1_capnp::self_::Server for StubSelf {
        fn held(
            &mut self,
            _: darude_wire::self_v1_capnp::self_::HeldParams,
            mut results: darude_wire::self_v1_capnp::self_::HeldResults,
        ) -> capnp::capability::Promise<(), capnp::Error> {
            use capnp::traits::HasTypeId;
            let mut out = results.get();
            let mut slots = out.reborrow().init_slots(4);
            {
                // llm: one properly-righted binding, labeled "fake".
                let mut s = slots.reborrow().get(0);
                s.set_name("llm");
                s.set_interface("llm.chat.v1");
                s.reborrow().init_rights(1).set(0, "complete");
                let mut b = s.init_bindings(1);
                let mut e = b.reborrow().get(0);
                e.set_label("fake");
                e.reborrow().init_rights(1).set(0, "complete");
                e.init_cap().set_as_capability(hook());
            }
            {
                // weak: its only binding lacks the declared right.
                let mut s = slots.reborrow().get(1);
                s.set_name("weak");
                s.set_interface("llm.chat.v1");
                s.reborrow().init_rights(1).set(0, "complete");
                let mut b = s.init_bindings(1);
                let mut e = b.reborrow().get(0);
                e.set_label("under");
                e.init_cap().set_as_capability(hook());
            }
            {
                // empty: declared, no bindings.
                let mut s = slots.reborrow().get(2);
                s.set_name("empty");
                s.set_interface("web.fetch.v1");
                s.init_bindings(0);
            }
            {
                // self: bound, but the caller already holds its Self.
                let mut s = slots.reborrow().get(3);
                s.set_name("self");
                s.set_interface("self.v1");
                s.set_interface_id(
                    <darude_wire::self_v1_capnp::self_::Client as HasTypeId>::TYPE_ID,
                );
                let mut b = s.init_bindings(1);
                let mut e = b.reborrow().get(0);
                e.set_label("self");
                e.init_cap().set_as_capability(hook());
            }
            out.init_entries(0);
            capnp::capability::Promise::ok(())
        }
    }

    #[test]
    fn pull_from_self_wires_slots_and_replaces_stale_entries() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            let sc: darude_wire::self_v1_capnp::self_::Client = capnp_rpc::new_client(StubSelf);
            let mut r = SessionRequires::default();
            // Stale pushed entries: llm (to be replaced), empty (to be
            // removed — the slot is now unbound), weak (to be removed —
            // its binding is under-righted).
            r.inner
                .entry("llm".into())
                .or_default()
                .push(("stale".into(), hook()));
            r.inner
                .entry("empty".into())
                .or_default()
                .push(("stale".into(), hook()));
            r.inner
                .entry("weak".into())
                .or_default()
                .push(("stale".into(), hook()));
            r.pull_from_self(&sc).await;
            let llm = r.take_all::<accept_cap::Client>("llm");
            assert_eq!(
                llm.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(),
                vec!["fake"],
                "the pulled binding replaces the stale entry"
            );
            assert!(
                r.take_all::<accept_cap::Client>("empty").is_empty(),
                "an unbound slot removes its stale entry"
            );
            assert!(
                r.take_all::<accept_cap::Client>("weak").is_empty(),
                "an under-righted binding never wires (no silent downgrade)"
            );
            assert!(
                r.take_all::<accept_cap::Client>("self").is_empty(),
                "the self.v1 slot is left to the caller"
            );
        });
    }
}
