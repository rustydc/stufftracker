// Grant-rights checks — THE shared spelling of "does this cap unlock
// mutation?". Every bridge froze the same three-line predicate (and the
// same read-only refusal) into its own main.rs, and one copy had
// already drifted; authorization logic is the one place copy-paste is
// never benign, so it lives here now.
//
// The contract (docs/grain-authoring.md): rights match *exactly* — no
// substring or prefix leniency — and `owner` (the supervisor's
// owner-session grant) implies every app-level right.

/// True iff the grant carries `right` or `owner`.
pub fn allows<S: AsRef<str>>(rights: &[S], right: &str) -> bool {
    rights
        .iter()
        .any(|r| r.as_ref() == right || r.as_ref() == "owner")
}

/// True iff the grant allows mutation via the conventional `write`
/// right. (An app whose schema names a different mutation right — e.g.
/// wave's `mutate` — uses `allows` with its own name.)
pub fn allow_write<S: AsRef<str>>(rights: &[S]) -> bool {
    allows(rights, "write")
}

/// True iff the grant is the owner's. Stricter than any app-level
/// right; gates owner-only surfaces (e.g. lifecycle operations).
pub fn owner<S: AsRef<str>>(rights: &[S]) -> bool {
    rights.iter().any(|r| r.as_ref() == "owner")
}

/// The standard read-only refusal for a mutating method: `None` when
/// the right is held, otherwise the error every bridge used to
/// hand-roll. `app` is the bridge's short name ("forgejo"), `right`
/// the right the method needs ("write", "mutate").
pub fn deny_if_readonly(app: &str, right: &str, granted: bool) -> Option<capnp::Error> {
    if granted {
        None
    } else {
        Some(capnp::Error::failed(format!(
            "{app}: read-only cap — this method needs the `{right}` right"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_write_needs_write_or_owner() {
        assert!(allow_write(&["write"]));
        assert!(allow_write(&["owner"]));
        assert!(allow_write(&["read", "write"]));
        assert!(!allow_write(&["read"]));
        assert!(!allow_write::<&str>(&[]));
        // No substring/prefix leniency: rights match exactly.
        assert!(!allow_write(&["writer", "read-write"]));
    }

    #[test]
    fn allows_matches_custom_right_names() {
        assert!(allows(&["mutate"], "mutate"));
        assert!(allows(&["owner"], "mutate"));
        assert!(!allows(&["write"], "mutate"));
    }

    #[test]
    fn owner_gating_is_stricter() {
        assert!(owner(&["owner"]));
        assert!(!owner(&["read", "write"]));
        assert!(!owner::<&str>(&[]));
    }

    #[test]
    fn deny_if_readonly_message_names_app_and_right() {
        assert!(deny_if_readonly("forgejo", "write", true).is_none());
        let err = deny_if_readonly("wave", "mutate", false).unwrap();
        assert!(err.to_string().contains("wave: read-only cap"));
        assert!(err.to_string().contains("`mutate` right"));
    }
}
