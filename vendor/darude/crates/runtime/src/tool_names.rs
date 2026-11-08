// Shared tool-naming: one place that turns an offered cap (its label +
// interface) into a stable, identifier-safe handle, used by every
// surface that exposes caps to a caller — the agent's LLM tools
// (agent-core), Monty scripts (code-exec), and Grist `=CALL` / `TOOLS`
// formulas (grist-bridge). Keeping the rule here means the *same* cap
// gets the *same* name everywhere: a cap labelled "My Notes" is
// `my_notes` to the model, in a script, and in a formula.
//
// The handle is constrained to the intersection of what all the
// surfaces accept:
//   - Python identifier (Grist `TOOLS.<handle>`, Monty global): ASCII
//     `[a-z0-9_]`, not starting with a digit.
//   - LLM function name (OpenAI / Anthropic both require
//     `^[a-zA-Z0-9_-]{1,64}$`): our subset satisfies this. Note dots
//     are NOT allowed by the LLM APIs, so the separator is `_`, never
//     `.` — and the agent's flat `<handle>_<method>` tool names stay
//     valid.
//
// Collision: when several caps share a base handle (the common case is
// one grain offering several interfaces, all carrying that grain's name
// as their label), the contenders are qualified by interface name
// (`docs_web_fetch_v1`) rather than numbered (`docs2`) — stable and
// self-describing. A numeric suffix remains only as a last resort.

use crate::introspect::SchemaCache;
use std::collections::HashSet;

/// Sanitize an arbitrary label into a handle: lowercase, runs of
/// non-alphanumerics collapse to a single `_`, leading/trailing `_`
/// trimmed, and a leading digit gets a `t_` prefix (identifiers can't
/// start with a digit). Returns None if nothing usable remains, so the
/// caller can fall back to the interface name.
pub fn sanitize_handle(label: &str) -> Option<String> {
    let mut out = String::new();
    let mut prev_us = false;
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_us = false;
        } else if !prev_us {
            out.push('_');
            prev_us = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        return None;
    }
    let s = trimmed.to_string();
    Some(if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("t_{s}")
    } else {
        s
    })
}

/// Human-meaningful name for an interface: the darude registry name
/// (e.g. "web.fetch.v1") when known, else the capnp type name from the
/// loaded schema's display name (e.g. "web-fetch-v1.capnp:WebFetchV1" →
/// "WebFetchV1"). Raw (un-sanitized); callers run it through
/// `sanitize_handle`.
pub fn interface_label(cache: &SchemaCache, interface_id: u64) -> String {
    if let Some(name) = darude_wire::interface_name_for_id(interface_id) {
        return name;
    }
    let Some(node) = cache.loader().get_node(interface_id) else {
        return "cap".to_string();
    };
    let Ok(name) = node.get_display_name() else {
        return "cap".to_string();
    };
    let Ok(s) = name.to_str() else {
        return "cap".to_string();
    };
    let prefix = node.get_display_name_prefix_length() as usize;
    if prefix > 0 && prefix <= s.len() {
        return s[prefix..].to_string();
    }
    match s.rfind(':') {
        Some(pos) => s[pos + 1..].to_string(),
        None => s.to_string(),
    }
}

/// Allocates unique handles across a set of caps, qualifying collisions
/// by interface. Drive it one cap at a time (`assign`); it tracks what's
/// taken. For an order-independent batch, call `from_batch`, which
/// qualifies *every* member of a shared base (not just the latecomers).
#[derive(Default)]
pub struct HandleAllocator {
    taken: HashSet<String>,
}

impl HandleAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    fn base(&self, label: Option<&str>, cache: &SchemaCache, interface_id: u64) -> String {
        label
            .and_then(sanitize_handle)
            .or_else(|| sanitize_handle(&interface_label(cache, interface_id)))
            .unwrap_or_else(|| "tool".to_string())
    }

    /// Mark a fixed name as taken (so later `assign`ed handles don't
    /// collide with it). For caps whose name is chosen out-of-band — e.g.
    /// code-exec's static `requires` caps named "web" / "grist".
    pub fn reserve(&mut self, name: &str) {
        self.taken.insert(name.to_string());
    }

    fn claim(&mut self, candidate: String) -> String {
        if self.taken.insert(candidate.clone()) {
            return candidate;
        }
        for n in 2.. {
            let c = format!("{candidate}{n}");
            if self.taken.insert(c.clone()) {
                return c;
            }
        }
        unreachable!()
    }

    /// Assign one handle. If the base is free, take it; otherwise qualify
    /// by interface; numeric suffix only if that too is taken. Use for
    /// caps discovered after the initial batch (e.g. a script's
    /// `return_cap`) — the first cap on a base keeps the bare name.
    pub fn assign(
        &mut self,
        label: Option<&str>,
        cache: &SchemaCache,
        interface_id: u64,
    ) -> String {
        let base = self.base(label, cache, interface_id);
        if !self.taken.contains(&base) {
            return self.claim(base);
        }
        let iface = sanitize_handle(&interface_label(cache, interface_id));
        let qualified = match iface {
            Some(i) => format!("{base}_{i}"),
            None => base.clone(),
        };
        self.claim(qualified)
    }

    /// Assign handles to a whole batch up front (order-independent):
    /// every cap whose base is shared by more than one member is
    /// qualified by interface, so no single cap "wins" the bare name.
    /// Returns one handle per item, in input order, and leaves the
    /// allocator seeded so later `assign` calls don't collide.
    pub fn from_batch(items: &[(Option<&str>, u64)], cache: &SchemaCache) -> (Self, Vec<String>) {
        let mut alloc = Self::new();
        let bases: Vec<String> = items
            .iter()
            .map(|(label, id)| alloc.base(*label, cache, *id))
            .collect();
        let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for b in &bases {
            *counts.entry(b.as_str()).or_default() += 1;
        }
        let handles = items
            .iter()
            .zip(&bases)
            .map(|((label, id), base)| {
                let candidate = if counts.get(base.as_str()).copied().unwrap_or(0) > 1 {
                    match sanitize_handle(&interface_label(cache, *id)) {
                        Some(i) => format!("{base}_{i}"),
                        None => base.clone(),
                    }
                } else {
                    base.clone()
                };
                let _ = label;
                alloc.claim(candidate)
            })
            .collect();
        (alloc, handles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_cases() {
        assert_eq!(sanitize_handle("My Notes").as_deref(), Some("my_notes"));
        assert_eq!(
            sanitize_handle("web.fetch.v1").as_deref(),
            Some("web_fetch_v1")
        );
        assert_eq!(sanitize_handle("web-search").as_deref(), Some("web_search"));
        assert_eq!(sanitize_handle("  --- ").as_deref(), None);
        assert_eq!(sanitize_handle("2pad").as_deref(), Some("t_2pad"));
        // Identifier-safe everywhere: lowercase, single underscores, no
        // leading digit. (Also satisfies the LLM `[A-Za-z0-9_-]` rule.)
        assert_eq!(sanitize_handle("A  B__c").as_deref(), Some("a_b_c"));
    }
}
