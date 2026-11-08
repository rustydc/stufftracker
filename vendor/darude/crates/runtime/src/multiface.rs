//! Multi-interface local servers, post the stage-4 mixin sweep
//! (docs/grain-abi-v2.md graveyard).
//!
//! `extends` is gone from the schemas, but a provider object may still
//! answer two interfaces on ONE cap: its primary face plus the
//! provider-facing `Recreatable` recreate contract the membrane's
//! crossing probe calls (`restorable.capnp`). capnp dispatches by
//! (interfaceId, methodId), so the composition is a dispatcher that
//! routes by interface id — no schema-level inheritance needed, and
//! the two faces never collide.

use capnp::any_pointer;
use capnp::capability::{DispatchCallResult, FromClientHook, Params, Results, Server};

/// Route calls between two `capability::Server`s by interface id:
/// `second_id` goes to `second`, everything else to `primary` (whose
/// generated dispatch answers unimplemented for ids it doesn't serve).
pub struct TwoFace<A, B> {
    pub primary: A,
    pub second: B,
    pub second_id: u64,
}

impl<A: Server, B: Server> Server for TwoFace<A, B> {
    fn dispatch_call(
        &mut self,
        interface_id: u64,
        method_id: u16,
        params: Params<any_pointer::Owned>,
        results: Results<any_pointer::Owned>,
    ) -> DispatchCallResult {
        if interface_id == self.second_id {
            self.second
                .dispatch_call(interface_id, method_id, params, results)
        } else {
            self.primary
                .dispatch_call(interface_id, method_id, params, results)
        }
    }
}

/// A `C`-typed client over `primary` that ALSO answers
/// `Recreatable.key` from `rec` — the standard shape for a provider
/// vending a rebuildable sub-object. Callers typically pass the same
/// (cloned) server twice, wrapped in each interface's generated
/// dispatcher:
///
/// ```ignore
/// let t: trigger::Client = client_with_recreatable(
///     trigger::ServerDispatch { server: st.clone() },
///     st,
/// );
/// ```
pub fn client_with_recreatable<C, A, T>(primary: A, rec: T) -> C
where
    A: Server + 'static,
    T: darude_wire::restorable_capnp::recreatable::Server + 'static,
    C: FromClientHook,
{
    use capnp::traits::HasTypeId;
    let hook = Box::new(capnp_rpc::local::Client::new(TwoFace {
        primary,
        second: darude_wire::restorable_capnp::recreatable::ServerDispatch { server: rec },
        second_id: <darude_wire::restorable_capnp::recreatable::Client as HasTypeId>::TYPE_ID,
    }));
    C::new(hook)
}
