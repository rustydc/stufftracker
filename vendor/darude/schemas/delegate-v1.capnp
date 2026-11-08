@0xf3a2b1c4d5e6f708;
# delegate.v1 — share a saved sub-cap you hold with another grain.
#
# Platform-served (like spawn/schedule/saver): the cap on the wire is
# rooted in platformd, not another grain. A grain receives it via the
# normal `requires:` mechanism (its manifest declares
# `requires: { delegate: { interface: delegate.v1 } }`).
#
# The primitive it exposes is held-cap delegation: you hold a durable
# sub-cap (a specific attenuated sub-object you got from someone and
# `save()`d into a SturdyRef token), and you want a DIFFERENT grain to
# hold it too — canonically an agent handing one of its saved tool
# sub-caps to a subagent it spawned. A live cap can't cross into a grain
# you don't share a session with, and a raw token isn't authority on its
# own (restore is owner-scoped), so delegation is a platform act: it
# mints the recipient its OWN grant + token for the SAME sub-object.
#
# The recipient picks the cap up on its next handshake through the same
# durable re-vend path as any saved cap (keyed by the returned token).
# The recipient's grant is chained under YOUR hold, so:
#   - severing the recipient's grant revokes only the recipient;
#   - severing your own hold (or any ancestor) cascades to it;
#   - the recipient's calls audit under the recipient's own row.
# This is delegation that only ever narrows — you can't hand out more
# than you hold, and you can always take it back.
interface Delegate {
  # Delegate the saved sub-cap named by `token` (a SturdyRef you got
  # from `save()`) to `recipient` (a grain instance id). `rights`
  # attenuates (empty = inherit the source grant's rights). Returns
  # the recipient's own re-vend token.
  #
  # `recipient` is e.g. the `instanceId` a `spawn()` returned. The
  # recipient's grant carries at most the given rights. Errors if you
  # don't actually hold `token`.
  #
  # The returned token is the recipient's OWN re-vend token: the string
  # it will see keyed in its `restored` handshake map, so you can tell
  # it which of its saved caps this is. It is an identifier to pass
  # along, not extra authority — restore stays owned by the recipient.
  delegate @0 (token :Data, recipient :Text, rights :List(Text))
    -> (recipientToken :Data);
}
