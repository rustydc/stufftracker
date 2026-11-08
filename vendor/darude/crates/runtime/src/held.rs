// Held<T> — the typed-Rust face of "one object, two facets"
// (docs/grain-abi-v2.md, "Cap"). Every platform-tracked cap answers
// the membrane-implemented cap.v1 surface alongside its own
// interface; dynamic surfaces (Monty, agent lowering, the console)
// see both on one object for free, and this wrapper gives grain code
// the same ergonomics without ever spelling a cast: `.client()` for
// the interface facet, `.edge()` for the cap.v1 facet.
//
// A cast in capnp is not an operation — pointers are untyped, calls
// carry (interfaceId, ordinal), the receiver serves or answers
// unimplemented — which is exactly why there is no wire-level
// `getInterface`: it would be a round trip returning the cap you
// already hold. Held<T> is where that non-operation gets a name.

use capnp::capability::FromClientHook;
use capnp::private::capability::ClientHook;
use std::marker::PhantomData;

pub struct Held<T> {
    hook: Box<dyn ClientHook>,
    _t: PhantomData<T>,
}

impl<T: FromClientHook> Held<T> {
    pub fn new(hook: Box<dyn ClientHook>) -> Self {
        Self {
            hook,
            _t: PhantomData,
        }
    }

    /// The interface facet: the cap as its own type.
    pub fn client(&self) -> T {
        T::new(self.hook.add_ref())
    }

    /// The edge facet: the membrane-implemented cap.v1 surface
    /// (describe / attenuate / save / forget / drop).
    pub fn edge(&self) -> darude_wire::cap_v1_capnp::cap::Client {
        FromClientHook::new(self.hook.add_ref())
    }

    pub fn hook(&self) -> Box<dyn ClientHook> {
        self.hook.add_ref()
    }
}

impl<T> Clone for Held<T> {
    fn clone(&self) -> Self {
        Self {
            hook: self.hook.add_ref(),
            _t: PhantomData,
        }
    }
}
