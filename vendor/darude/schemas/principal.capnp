@0xa0b1c2d3e4f50617;
# Principal — the actor on whose behalf a call is being made.
#
# Set at session handshake time, ambient for every call routed
# through the connection. Replaces the per-cap `Personable.withIdentity`
# pattern. The grain's bridge constructs per-session Server impls
# parameterized by the Principal, then reads it as construction-time
# data on each method invocation. No per-call rebind, no marker
# interface, no returned-cap fragility.

struct Principal {
  # Stable handle (the platform user's `users.handle` column).
  # Empty for the anonymous / system session — used by the grain's
  # own startup handshake before any user-driven offer applies, and
  # for system-driven offers like the auto-offer-at-launch path.
  username    @0 :Text;
  # Human-readable form (the `users.display_name` column). Grains
  # use this for things like wave's auto-created participant names
  # in `X-Darude-Display-Name`. May differ from username; empty
  # means "fall back to username for display."
  displayName @1 :Text;
}
