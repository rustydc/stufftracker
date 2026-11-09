Client copies (at Darude commit e8c2c83) of two Darude bridge interfaces StuffTracker calls through
optional slots, unchanged from https://github.com/rustydc/darude:

- `homeassistant-v1.capnp` from `platformd-rs/crates/apps/ha-bridge/`
- `outline-v1.capnp` from `platformd-rs/crates/apps/outline-bridge/`

Both keep their file IDs, so their generated types match the bridges'
exactly. Cap'n Proto evolution keeps these copies compatible as the bridges
add methods and fields; refresh them to use something new. Only the client
side is used (getStates and listAreas; getDocument).
