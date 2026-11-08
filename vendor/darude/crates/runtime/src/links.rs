//! Links a capability hands to a person.
//!
//! A grain's paths ("/doc/abc") mean nothing outside the grain: an agent
//! that drops one into a chat reply gets it resolved against the CHAT's
//! host. So a link meant to be opened in a browser leaves a grain
//! absolute, built on the grain's own public URL — which the platform
//! injects at launch as `DARUDE_GRAIN_URL` (e.g.
//! `https://outline-b8e25701.darude.rmel.io`). The grain is the only party
//! that knows both halves, so it does the join. Links that live INSIDE
//! the grain's own content (an Outline page embedding its attachment)
//! stay relative: they must keep working there.

/// `path` as an absolute link on this grain's public URL. Unchanged when
/// it's already absolute, or when the grain has no public URL (no route,
/// or a test run).
pub fn grain_url(path: &str) -> String {
    join(std::env::var("DARUDE_GRAIN_URL").ok().as_deref(), path)
}

fn join(base: Option<&str>, path: &str) -> String {
    let path = path.trim();
    if path.is_empty() || path.contains("://") {
        return path.to_string();
    }
    match base.map(str::trim).filter(|b| !b.is_empty()) {
        Some(base) => {
            let base = base.trim_end_matches('/');
            if path.starts_with('/') {
                format!("{base}{path}")
            } else {
                format!("{base}/{path}")
            }
        }
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::join;

    #[test]
    fn paths_become_links_on_the_grain_url() {
        let b = Some("https://outline-1.example.io/");
        assert_eq!(join(b, "/doc/x-1"), "https://outline-1.example.io/doc/x-1");
        assert_eq!(join(b, "doc/x-1"), "https://outline-1.example.io/doc/x-1");
        // Already absolute, empty, or no public URL: left alone.
        assert_eq!(join(b, "https://elsewhere.io/a"), "https://elsewhere.io/a");
        assert_eq!(join(b, ""), "");
        assert_eq!(join(None, "/doc/x-1"), "/doc/x-1");
        assert_eq!(join(Some(" "), "/doc/x-1"), "/doc/x-1");
    }
}
