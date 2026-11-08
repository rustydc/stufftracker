// trigger.v1 payload <-> neutral `Value` conversion.
//
// `TriggerValue` (schemas/trigger-v1.capnp) is a self-describing,
// JSON-like union that can also carry a live capability. This module
// converts it to and from the `introspect::Value` tree the rest of the
// runtime already speaks, so a `trigger()` payload lowers to JSON for an
// agent / Monty script through the exact machinery that already handles
// OpaqueCap tool arguments.
//
// Why typed (not the generic `introspect::struct_to_value`): that
// reader gates every field on `has()`, which is false for a union arm
// sitting at its zero default — `int 0`, `uint 0`, `bool false` would
// silently vanish. Reading the union via `which()` reads the active arm
// unconditionally, so defaults survive the round trip.
//
// The `cap` arm round-trips as an OpaqueCap-shaped `Value::Struct`
// (`cap` / `interfaceId` / `label` / `schema`) — the same shape the
// agent's `is_opaque_cap` / `register_opaque_cap` path already
// recognises, so a cap in a trigger payload lowers to a `cap://<handle>`
// ref with no new code on the consumer side.

use anyhow::{anyhow, Result};
use capnp::capability::FromClientHook;

use darude_wire::trigger_v1_capnp::trigger_value;

use crate::introspect::{CapRef, Value};

fn ce(e: capnp::Error) -> anyhow::Error {
    anyhow!("capnp: {e}")
}
fn ue(e: std::str::Utf8Error) -> anyhow::Error {
    anyhow!("utf8: {e}")
}

/// Reads a `TriggerValue` into a neutral `Value`. The active union arm
/// determines the result; the `cap` arm becomes an OpaqueCap-shaped
/// `Value::Struct` so downstream JSON lowering treats it like any other
/// tool cap.
pub fn read_trigger_value(reader: trigger_value::Reader) -> Result<Value> {
    use trigger_value::Which;
    Ok(
        match reader
            .which()
            .map_err(|e| anyhow!("trigger value which: {e}"))?
        {
            Which::Null(()) => Value::Null,
            Which::Bool(b) => Value::Bool(b),
            Which::Int(i) => Value::Int(i),
            Which::Uint(u) => Value::UInt(u),
            Which::Float(f) => Value::Float(f),
            Which::Text(t) => Value::Text(t.map_err(ce)?.to_str().map_err(ue)?.to_string()),
            Which::Data(d) => Value::Data(d.map_err(ce)?.to_vec()),
            Which::List(l) => {
                let l = l.map_err(ce)?;
                let mut items = Vec::with_capacity(l.len() as usize);
                for el in l.iter() {
                    items.push(read_trigger_value(el)?);
                }
                Value::List(items)
            }
            Which::Object(o) => {
                let o = o.map_err(ce)?;
                let mut fields = Vec::with_capacity(o.len() as usize);
                for f in o.iter() {
                    let name = f.get_name().map_err(ce)?.to_str().map_err(ue)?.to_string();
                    fields.push((name, read_trigger_value(f.get_value().map_err(ce)?)?));
                }
                Value::Struct(fields)
            }
            Which::Cap(c) => read_opaque_cap(c.map_err(ce)?)?,
        },
    )
}

/// Reads an `OpaqueCap` into the `Value::Struct` shape the agent /
/// code-exec OpaqueCap detection expects.
fn read_opaque_cap(oc: darude_wire::introspectable_capnp::opaque_cap::Reader) -> Result<Value> {
    let interface_id = oc.get_interface_id();
    let label = oc
        .get_label()
        .map_err(ce)?
        .to_str()
        .map_err(ue)?
        .to_string();
    let schema = oc.get_schema().map_err(ce)?.to_vec();
    // Extract the bare `Capability` as a hook via any typed carrier.
    let carrier: darude_wire::accept_cap_capnp::accept_cap::Client =
        oc.get_cap().get_as_capability().map_err(ce)?;
    let hook = carrier.into_client_hook();
    Ok(Value::Struct(vec![
        (
            "cap".to_string(),
            Value::Cap(CapRef::new(hook, interface_id)),
        ),
        ("interfaceId".to_string(), Value::UInt(interface_id)),
        ("label".to_string(), Value::Text(label)),
        ("schema".to_string(), Value::Data(schema)),
    ]))
}

/// Builds a `TriggerValue` from a neutral `Value`. A `Value::Cap`, or a
/// `Value::Struct` shaped like an OpaqueCap, becomes the `cap` arm; any
/// other struct becomes the `object` arm.
pub fn build_trigger_value(mut builder: trigger_value::Builder, value: &Value) -> Result<()> {
    match value {
        Value::Null => builder.set_null(()),
        Value::Bool(b) => builder.set_bool(*b),
        Value::Int(i) => builder.set_int(*i),
        Value::UInt(u) => builder.set_uint(*u),
        Value::Float(f) => builder.set_float(*f),
        Value::Text(s) => builder.set_text(s.as_str()),
        // An enum has no native arm; carry it by name as text.
        Value::Enum(s) => builder.set_text(s.as_str()),
        Value::Data(d) => builder.set_data(d.as_slice()),
        Value::List(items) => {
            let mut list = builder.reborrow().init_list(items.len() as u32);
            for (i, item) in items.iter().enumerate() {
                build_trigger_value(list.reborrow().get(i as u32), item)?;
            }
        }
        Value::Cap(c) => {
            // Live cap with no wrapper: recover its schema bytes from the
            // wire registry by interface id (empty if unknown — the
            // receiver may already hold the schema).
            let schema = darude_wire::schema_for_interface_id(c.interface_id()).unwrap_or(&[]);
            build_cap_arm(
                builder.reborrow(),
                c.add_ref(),
                c.interface_id(),
                "",
                schema,
            )?;
        }
        Value::Struct(fields) => {
            if let Some(cap) = as_opaque_cap(fields) {
                build_cap_arm(
                    builder.reborrow(),
                    cap.hook,
                    cap.interface_id,
                    cap.label,
                    cap.schema,
                )?;
            } else {
                let mut obj = builder.reborrow().init_object(fields.len() as u32);
                for (i, (name, v)) in fields.iter().enumerate() {
                    let mut f = obj.reborrow().get(i as u32);
                    f.set_name(name.as_str());
                    build_trigger_value(f.init_value(), v)?;
                }
            }
        }
    }
    Ok(())
}

fn build_cap_arm(
    builder: trigger_value::Builder,
    hook: Box<dyn capnp::private::capability::ClientHook>,
    interface_id: u64,
    label: &str,
    schema: &[u8],
) -> Result<()> {
    let mut oc = builder.init_cap();
    oc.set_interface_id(interface_id);
    oc.set_label(label);
    oc.set_schema(schema);
    oc.reborrow().init_cap().set_as_capability(hook);
    Ok(())
}

/// The pieces of an OpaqueCap recovered from a `Value::Struct`, if it has
/// the `cap` / `interfaceId` / `schema` shape.
struct OpaqueCapParts<'a> {
    hook: Box<dyn capnp::private::capability::ClientHook>,
    interface_id: u64,
    label: &'a str,
    schema: &'a [u8],
}

fn as_opaque_cap(fields: &[(String, Value)]) -> Option<OpaqueCapParts<'_>> {
    let mut hook = None;
    let mut interface_id = None;
    let mut label = "";
    let mut schema: Option<&[u8]> = None;
    for (k, v) in fields {
        match (k.as_str(), v) {
            ("cap", Value::Cap(c)) => hook = Some(c.add_ref()),
            ("interfaceId", Value::UInt(u)) => interface_id = Some(*u),
            ("schema", Value::Data(d)) => schema = Some(d.as_slice()),
            ("label", Value::Text(s)) => label = s.as_str(),
            _ => {}
        }
    }
    Some(OpaqueCapParts {
        hook: hook?,
        interface_id: interface_id?,
        label,
        schema: schema?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(v: &Value) -> Value {
        let mut msg = capnp::message::Builder::new_default();
        build_trigger_value(msg.init_root::<trigger_value::Builder>(), v).unwrap();
        let reader = msg.get_root_as_reader::<trigger_value::Reader>().unwrap();
        read_trigger_value(reader).unwrap()
    }

    #[test]
    fn scalars_survive_including_defaults() {
        // The whole reason for the typed path: zero/false defaults that
        // the generic has()-gated reader would drop must round-trip.
        for v in [
            Value::Null,
            Value::Bool(false),
            Value::Bool(true),
            Value::Int(0),
            Value::Int(-42),
            Value::UInt(0),
            Value::UInt(9_000_000_000),
            Value::Float(3.5),
            Value::Text(String::new()),
            Value::Text("hi".into()),
            Value::Data(vec![]),
            Value::Data(vec![1, 2, 3]),
        ] {
            assert_eq!(round_trip(&v), v, "round trip failed for {v:?}");
        }
    }

    #[test]
    fn nested_object_and_list() {
        let v = Value::struct_of([
            ("kind".to_string(), Value::Text("notify".into())),
            ("count".to_string(), Value::UInt(0)),
            (
                "items".to_string(),
                Value::List(vec![
                    Value::Int(-3),
                    Value::Text("x".into()),
                    Value::Bool(false),
                ]),
            ),
            (
                "nested".to_string(),
                Value::struct_of([("a".to_string(), Value::Bool(true))]),
            ),
        ]);
        assert_eq!(round_trip(&v), v);
    }

    #[test]
    fn reinit_capability_field_does_not_panic() {
        // Regression: `OpaqueCap.cap` is an AnyPointer slot. Re-initializing
        // an OpaqueCap (as any rebuild of a cap-bearing value does) clears
        // that slot, whose wire pointer already holds a capability. Zeroing a
        // capability pointer used to abort with "Don't know how to handle
        // OTHER"; it must now release the old cap and continue.
        struct Stub;
        impl darude_wire::accept_cap_capnp::accept_cap::Server for Stub {}

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            let make_hook = || {
                let client: darude_wire::accept_cap_capnp::accept_cap::Client =
                    capnp_rpc::new_client(Stub);
                client.into_client_hook()
            };

            let mut msg = capnp::message::Builder::new_default();
            // A standalone builder has no cap table; imbue one so the
            // capabilities have somewhere to live (RPC requests/results do
            // this for the production callers of build_trigger_value).
            let mut cap_table: capnp::private::layout::CapTable = Vec::new();
            let mut root = msg.init_root::<trigger_value::Builder>();
            capnp::traits::ImbueMut::imbue_mut(&mut root, &mut cap_table);
            let mut oc = root.init_cap();
            oc.reborrow().init_cap().set_as_capability(make_hook());
            // Second init_cap() clears the `cap` slot that the line above
            // filled with a live capability — the path that used to panic.
            oc.reborrow().init_cap().set_as_capability(make_hook());
        });
    }
}
