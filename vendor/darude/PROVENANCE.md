Darude grain SDK snapshot from https://github.com/rustydc/darude
commit 5bbecbeeca84bfd1c09eedc6a86d5cf4bbc391ad.

Includes runtime, wire, schema-docs, wire schemas, and the platform's patched
capnp/capnp-rpc dependencies. Darude crate manifests declare Apache-2.0;
capnp dependencies retain their upstream MIT licenses. Sources are unchanged apart from formatting;
the small workspace manifest selects only grain SDK dependencies. Vendored so
StuffTracker builds independently of a neighboring Darude checkout. Update this
snapshot together with its patched dependencies when upgrading the grain ABI.
