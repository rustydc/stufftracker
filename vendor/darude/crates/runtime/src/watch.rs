//! Source-side durable-watch helper (docs/durable-held-caps.md).
//!
//! The app-visible model for a Source grain is "receive a persistable
//! Trigger, `save()` it, fire it on change, `forget()` it on stop" — but
//! doing that hibernation-correctly still means ~250 lines of identical
//! bookkeeping per bridge: persist the watch rows to `run/watches.json`,
//! reload them cap-less on restart, re-arm each from `Self.restore`
//! (rearm_from_self), poll for changes while awake, fire with a
//! cursor payload, advance + re-persist. memos-bridge and outline-bridge
//! each hand-rolled it; this module is that machinery once, generic over
//! the app-specific watch *scope* (whatever the bridge needs to run its
//! change probe: filters, principal headers, a content hash…).
//!
//! A bridge supplies three things:
//! - a scope type `S: Serialize + DeserializeOwned + Clone`,
//! - a probe closure `Fn(S, cursor) -> Future<Result<Option<Hit<S>>>>`
//!   ("did anything change past this cursor?"),
//! - its capnp `watch`/`Watch.stop` methods, lowered onto
//!   [`WatchSet::register`] / [`WatchSet::stop`], plus a call to
//!   [`WatchSet::rearm`] in its session factory.
//!
//! The fired payload is always the edge-triggered
//! `{ kind: <kind>, nextCursor: <i64> }` object — the notification says
//! only "you have changes up to N"; the Sink drains the specifics through
//! the bridge's own `changesSince`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use capnp::capability::FromClientHook;
use capnp::private::capability::ClientHook;
use darude_wire::cap_v1_capnp::cap as cap_v1;
use darude_wire::trigger_v1_capnp::{trigger, trigger_value};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// One durable subscription. `fire` is the Trigger to call on a change:
/// the captured live wrapper at `watch()` time, upgraded to the re-vended
/// durable wrapper at the next handshake; `None` until a handshake
/// re-arms a watch loaded from disk after a restart.
pub struct WatchEntry<S> {
    pub scope: S,
    pub cursor: i64,
    pub fire: Option<trigger::Client>,
}

/// On-disk form (no live caps — re-vended at handshake).
#[derive(Serialize, Deserialize)]
struct Row<S> {
    token: String,
    cursor: i64,
    scope: S,
}

/// The durable subscriptions of one Source grain, keyed by the saved
/// Trigger's SturdyRef token, persisted to `<run_dir>/watches.json`.
/// Clone-cheap (Rc); share one between the session factory and the
/// poller.
pub struct WatchSet<S> {
    kind: &'static str,
    run_dir: Rc<PathBuf>,
    entries: Rc<RefCell<HashMap<String, WatchEntry<S>>>>,
}

impl<S> Clone for WatchSet<S> {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            run_dir: self.run_dir.clone(),
            entries: self.entries.clone(),
        }
    }
}

/// A probe's "something changed" outcome: the new cursor to fire with,
/// optionally a replacement scope (for probes that carry derived state
/// in the scope, e.g. a content hash), and optional extra text fields
/// for the fired payload (e.g. outline's `collectionId`).
pub struct Hit<S> {
    pub next_cursor: i64,
    pub scope: Option<S>,
    pub extras: Vec<(String, String)>,
}

impl<S> Hit<S> {
    /// The common case: a new cursor, scope unchanged, no extras.
    pub fn cursor(next_cursor: i64) -> Self {
        Self {
            next_cursor,
            scope: None,
            extras: Vec::new(),
        }
    }

    pub fn with_extra(mut self, name: &str, value: &str) -> Self {
        self.extras.push((name.to_string(), value.to_string()));
        self
    }
}

impl<S: Serialize + DeserializeOwned + Clone + 'static> WatchSet<S> {
    /// Load persisted watches from `<run_dir>/watches.json` (cap-less —
    /// each entry's Trigger is re-armed via `rearm_from_self` at the
    /// next handshake). `kind` is the payload tag fired on change, e.g.
    /// `"memos.changed"`.
    pub fn load(kind: &'static str, run_dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&run_dir);
        let mut entries = HashMap::new();
        let rows: Vec<Row<S>> = std::fs::read_to_string(watches_file(&run_dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        for row in rows {
            entries.insert(
                row.token.clone(),
                WatchEntry {
                    scope: row.scope,
                    cursor: row.cursor,
                    fire: None,
                },
            );
        }
        let set = Self {
            kind,
            run_dir: Rc::new(run_dir),
            entries: Rc::new(RefCell::new(entries)),
        };
        tracing::info!(kind, watches = set.len(), "watch: loaded");
        set
    }

    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.borrow().is_empty()
    }

    /// Snapshot of every entry's scope — for bridges that pre-filter a
    /// push event stream against what's actually being watched before
    /// poking the poll loop.
    pub fn scopes(&self) -> Vec<S> {
        self.entries
            .borrow()
            .values()
            .map(|w| w.scope.clone())
            .collect()
    }

    /// Register a new watch from the caller's persistable Trigger:
    /// `save()` it (the caretaker forwards to the Sink, which answers
    /// with its wakeup policy and a durable token), record the entry
    /// with the still-live captured wrapper as `fire`, persist, and
    /// return the token (the bridge builds its `Watch` cap around it).
    pub async fn register(
        &self,
        notify_me: Box<dyn ClientHook>,
        scope: S,
        cursor: i64,
    ) -> Result<String, capnp::Error> {
        let pclient: cap_v1::Client = FromClientHook::new(notify_me.add_ref());
        let saved = pclient.save_request().send().promise.await?;
        let token = std::str::from_utf8(saved.get()?.get_token()?)
            .map_err(|e| capnp::Error::failed(format!("watch: token not utf8: {e}")))?
            .to_string();
        // save() flipped the platform-side wrapper to durable routing, so
        // fires through this handle already survive the Sink's hibernation
        // (policy + restore-based delivery). The next handshake swaps in
        // the re-vended durable wrapper — equivalent, just restated.
        let fire: trigger::Client = FromClientHook::new(notify_me);
        self.entries.borrow_mut().insert(
            token.clone(),
            WatchEntry {
                scope,
                cursor,
                fire: Some(fire),
            },
        );
        self.persist();
        Ok(token)
    }

    /// End a watch: `forget()` the saved Trigger (idempotent) and drop
    /// the entry. The bridge's `Watch.stop` lowers to this.
    pub async fn stop(&self, token: &str) {
        let entry = self.entries.borrow_mut().remove(token);
        if let Some(w) = entry {
            if let Some(fire) = w.fire {
                let p: cap_v1::Client = FromClientHook::new(fire.into_client_hook());
                let mut req = p.forget_request();
                req.get().set_token(token.as_bytes());
                let _ = req.send().promise.await; // idempotent; ignore errors
            }
        }
        self.persist();
    }

    /// Re-arm persisted watches (docs/self-discovery.md step 5 — the
    /// handshake's pushed `restored` re-vend is retired; this pull is
    /// the only re-arm): re-derive each persisted watch's Trigger from
    /// its saved-cap TOKEN via `Self.restore`. Call from the session
    /// factory after taking the `self` require. A token that no longer
    /// restores (severed/forgotten) leaves its entry un-armed (the
    /// poll loop skips fireless watches), and the returned Trigger is
    /// the wakeup-policy-aware durable one. Best-effort per token —
    /// one dead token doesn't stop the rest.
    pub async fn rearm_from_self(&self, sc: &darude_wire::self_v1_capnp::self_::Client) {
        let tokens: Vec<String> = self.entries.borrow().keys().cloned().collect();
        for t in tokens {
            let mut req = sc.restore_request();
            req.get().set_token(&t);
            let resp = match req.send().promise.await {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(token = %t, "watch rearm: self.restore: {e}");
                    continue;
                }
            };
            let Ok(r) = resp.get() else { continue };
            if !r.get_found() {
                continue;
            }
            let Ok(cap) = r.get_cap().get_as_capability::<trigger::Client>() else {
                continue;
            };
            if let Some(w) = self.entries.borrow_mut().get_mut(&t) {
                w.fire = Some(cap);
            }
        }
    }

    /// Run the poll-and-fire loop while the grain is awake: every
    /// `interval`, run `probe(scope, cursor)` for each armed watch; on a
    /// `Hit`, fire the watch's Trigger with `{kind, nextCursor}`, advance
    /// the cursor (and scope, if the probe replaced it), and persist.
    /// The platform's wakeup policy — not the Source — decides
    /// whether/when a fire actually wakes the Sink. Spawn with
    /// `tokio::task::spawn_local(set.clone().poll_loop(interval, probe))`.
    pub async fn poll_loop<F, Fut>(self, interval: Duration, probe: F)
    where
        F: Fn(S, i64) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Option<Hit<S>>>>,
    {
        // A Notify nobody pokes: poll_loop is the pure-poll special case.
        self.poll_loop_poked(interval, Rc::new(tokio::sync::Notify::new()), probe)
            .await
    }

    /// `poll_loop`, but a push signal can cut the wait short: `poke`
    /// triggers an immediate probe round on top of the steady
    /// `interval`. For bridges whose upstream can push change events
    /// (a websocket, an SSE stream): the poll keeps its lossless
    /// cursor semantics and stays the fallback when the push channel
    /// is down or the event was missed — the push just collapses
    /// worst-case latency from `interval` to near-zero. Debounce
    /// bursts on the poker's side; consecutive pokes during a probe
    /// round coalesce into one extra round (Notify keeps one permit).
    pub async fn poll_loop_poked<F, Fut>(
        self,
        interval: Duration,
        poke: Rc<tokio::sync::Notify>,
        probe: F,
    ) where
        F: Fn(S, i64) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Option<Hit<S>>>>,
    {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = poke.notified() => {}
            }
            let jobs: Vec<(String, S, i64, trigger::Client)> = self
                .entries
                .borrow()
                .iter()
                .filter_map(|(t, w)| {
                    w.fire
                        .clone()
                        .map(|fire| (t.clone(), w.scope.clone(), w.cursor, fire))
                })
                .collect();
            for (token, scope, cursor, fire) in jobs {
                let hit = match probe(scope, cursor).await {
                    Ok(Some(h)) if h.next_cursor > cursor || h.scope.is_some() => h,
                    Ok(_) => continue,
                    Err(e) => {
                        tracing::warn!(token = %token, kind = self.kind, "watch probe: {e}");
                        continue;
                    }
                };
                let mut req = fire.trigger_request();
                build_change_payload_with(
                    req.get().init_payloads(1).get(0),
                    self.kind,
                    hit.next_cursor,
                    &hit.extras,
                );
                if let Err(e) = req.send().promise.await {
                    tracing::warn!(token = %token, kind = self.kind, "fire watch trigger: {e}");
                    continue;
                }
                if let Some(w) = self.entries.borrow_mut().get_mut(&token) {
                    w.cursor = hit.next_cursor;
                    if let Some(s) = hit.scope {
                        w.scope = s;
                    }
                }
                self.persist();
            }
        }
    }

    fn persist(&self) {
        let rows: Vec<Row<S>> = self
            .entries
            .borrow()
            .iter()
            .map(|(token, w)| Row {
                token: token.clone(),
                cursor: w.cursor,
                scope: w.scope.clone(),
            })
            .collect();
        match serde_json::to_string_pretty(&rows) {
            Ok(json) => {
                if let Err(e) = std::fs::write(watches_file(&self.run_dir), json) {
                    tracing::warn!("persist watches: {e}");
                }
            }
            Err(e) => tracing::warn!("serialize watches: {e}"),
        }
    }
}

fn watches_file(run_dir: &std::path::Path) -> PathBuf {
    run_dir.join("watches.json")
}

/// The edge-triggered change payload fired through a watch's Trigger:
/// `{ kind: <kind>, nextCursor: <cursor> }`. The Sink reacts by calling
/// the bridge's `changesSince(nextCursor)` to drain the actual events.
pub fn build_change_payload(builder: trigger_value::Builder, kind: &str, next_cursor: i64) {
    build_change_payload_with(builder, kind, next_cursor, &[]);
}

/// `build_change_payload` plus extra app-specific text fields (e.g.
/// outline's `collectionId` scope hint).
pub fn build_change_payload_with(
    builder: trigger_value::Builder,
    kind: &str,
    next_cursor: i64,
    extras: &[(String, String)],
) {
    use crate::introspect::Value;
    let mut fields = vec![
        ("kind".to_string(), Value::Text(kind.to_string())),
        ("nextCursor".to_string(), Value::Int(next_cursor)),
    ];
    for (name, value) in extras {
        fields.push((name.clone(), Value::Text(value.clone())));
    }
    // Can't fail for a text/int-only struct; keep the signature infallible.
    let _ = crate::trigger::build_trigger_value(builder, &Value::Struct(fields));
}

/// The grain's `run/` dir (under `DARUDE_STATE_DIR`), where watch state
/// lives: per-instance, survives the grain's own hibernation, and never
/// sealed into a state template — a fresh clone starts with no watches.
pub fn default_run_dir() -> PathBuf {
    PathBuf::from(std::env::var("DARUDE_STATE_DIR").unwrap_or_else(|_| ".".into())).join("run")
}

/// Unix millis now — the conventional starting cursor when a watch is
/// registered with `sinceMillis = 0` ("changes from now").
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
    struct Scope {
        filter: String,
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("darude-watchset-{tag}-{}", now_millis()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn load_of_missing_or_garbage_file_is_empty() {
        let dir = tmp_dir("empty");
        let set: WatchSet<Scope> = WatchSet::load("t.changed", dir.clone());
        assert!(set.is_empty());
        std::fs::write(watches_file(&dir), "not json").unwrap();
        let set: WatchSet<Scope> = WatchSet::load("t.changed", dir);
        assert!(set.is_empty());
    }

    #[test]
    fn persist_round_trips_rows_capless() {
        let dir = tmp_dir("roundtrip");
        let set: WatchSet<Scope> = WatchSet::load("t.changed", dir.clone());
        set.entries.borrow_mut().insert(
            "tok-1".into(),
            WatchEntry {
                scope: Scope { filter: "a".into() },
                cursor: 42,
                fire: None,
            },
        );
        set.persist();
        let re: WatchSet<Scope> = WatchSet::load("t.changed", dir);
        assert_eq!(re.len(), 1);
        let entries = re.entries.borrow();
        let w = entries.get("tok-1").unwrap();
        assert_eq!(w.cursor, 42);
        assert_eq!(w.scope, Scope { filter: "a".into() });
        // Caps never persist; the fire side re-arms via Self.restore.
        assert!(w.fire.is_none());
    }

    #[test]
    fn change_payload_round_trips() {
        let mut msg = capnp::message::Builder::new_default();
        build_change_payload(
            msg.init_root::<trigger_value::Builder>(),
            "memos.changed",
            9999,
        );
        let reader = msg.get_root_as_reader::<trigger_value::Reader>().unwrap();
        let v = crate::trigger::read_trigger_value(reader).unwrap();
        let crate::introspect::Value::Struct(fields) = v else {
            panic!("expected object payload");
        };
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);
        assert!(matches!(
            get("kind"),
            Some(crate::introspect::Value::Text(s)) if s == "memos.changed"
        ));
        assert!(matches!(
            get("nextCursor"),
            Some(crate::introspect::Value::Int(9999))
        ));
    }
}
