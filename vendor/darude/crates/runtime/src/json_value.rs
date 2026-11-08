// JSON <-> neutral `Value` conversion.
//
// `value_to_json` is the easy direction: `Value` already carries every
// scalar arm, so it maps straight onto serde_json. Live caps have no
// JSON form, so they render as a "cap-ref" placeholder string — the
// console doesn't pass live caps in v1, and a returned cap is shown, not
// round-tripped.
//
// `json_to_value` is schema-directed, because JSON flattens
// int/uint/float into one `number` and we must marshal into the right
// capnp field width. We walk the param `StructSchema`, and for each
// field coerce the matching JSON node by the field's declared type —
// the same type dispatch `introspect::value_to_struct` uses on the way
// to the wire. Missing fields are simply omitted (they take capnp
// defaults); unknown JSON keys are ignored.

use anyhow::{anyhow, bail, Result};
use capnp::introspect::TypeVariant;
use capnp::schema::StructSchema;
use serde_json::{Map, Value as Json};

use crate::introspect::{CapRef, Value};

/// Resolves a cap-typed param: given the reference string the form
/// supplied and the interface id the field declares, produce a live
/// `CapRef` (or error). Implemented by the caller (platformd's console),
/// which alone can check access + mint a caretaker. `json_to_value`
/// passes `None` to forbid caps (scheduling, where a live cap can't be
/// stored); `json_to_typed`'s `Capability` arm errors if a cap is
/// supplied but no resolver is set.
pub type CapResolver<'a> = &'a mut dyn FnMut(&str, u64) -> Result<CapRef>;

/// Placeholder string emitted for a live cap in `value_to_json`. Not a
/// real cap-ref (the console has no cap registry yet) — a human-readable
/// marker so a returned cap is visible rather than dropped.
const CAP_PLACEHOLDER: &str = "<cap>";

/// Convert a neutral `Value` to JSON. Caps become a placeholder string;
/// everything else maps onto the obvious JSON shape. Object field order
/// is preserved (serde_json::Map is insertion-ordered with the default
/// features).
pub fn value_to_json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => Json::Number((*i).into()),
        Value::UInt(u) => Json::Number((*u).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Value::Text(s) => Json::String(s.clone()),
        Value::Enum(s) => Json::String(s.clone()),
        Value::Data(d) => {
            use base64::Engine;
            Json::String(base64::engine::general_purpose::STANDARD.encode(d))
        }
        Value::List(items) => Json::Array(items.iter().map(value_to_json).collect()),
        Value::Struct(fields) => {
            let mut obj = Map::new();
            for (k, val) in fields {
                obj.insert(k.clone(), value_to_json(val));
            }
            Json::Object(obj)
        }
        Value::Cap(_) => Json::String(CAP_PLACEHOLDER.to_string()),
    }
}

/// Convert a JSON object into a `Value::Struct` shaped by `schema`. The
/// schema disambiguates JSON scalars (int vs uint vs float) and tells us
/// each field's element type for lists / nested structs / enums. Fields
/// absent from the JSON are omitted (capnp default applies); JSON keys
/// not in the schema are ignored.
pub fn json_to_value(json: &Json, schema: StructSchema) -> Result<Value> {
    json_to_value_with(json, schema, &mut None)
}

/// Like `json_to_value`, but resolves cap-typed params via `resolver`.
/// A cap field whose JSON value is a non-empty string is handed to the
/// resolver along with the interface id the field declares; the result
/// is bound as a live `Value::Cap`. With `resolver = None`, a supplied
/// cap reference errors (so scheduling rejects caps) while an
/// absent/empty one is simply left unset.
pub fn json_to_value_with(
    json: &Json,
    schema: StructSchema,
    resolver: &mut Option<CapResolver>,
) -> Result<Value> {
    let Json::Object(obj) = json else {
        bail!("expected a JSON object for struct, got {json}");
    };
    let fields = schema.get_fields().map_err(ce)?;
    // An unnamed union holds one member: setting two would silently keep
    // the last, so refuse and name the alternatives.
    let union_members = crate::tool_schema::union_member_names(schema);
    let given: Vec<&String> = union_members
        .iter()
        .filter(|m| obj.get(m.as_str()).is_some_and(|v| !v.is_null()))
        .collect();
    if given.len() > 1 {
        bail!(
            "{} are alternatives — set exactly one of {}",
            given
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(" and "),
            union_members
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let mut out: Vec<(String, Value)> = Vec::new();
    for i in 0..fields.len() {
        let field = fields.get(i);
        let name = field
            .get_proto()
            .get_name()
            .map_err(ce)?
            .to_str()
            .map_err(ue)?;
        let Some(node) = obj.get(name) else { continue };
        if node.is_null() {
            continue;
        }
        if crate::tool_schema::is_bare_capability(field) {
            // An untyped cap slot: bind like a typed one, unconstrained.
            let Some(refstr) = node.as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            match resolver {
                Some(resolve) => out.push((name.to_string(), Value::Cap(resolve(refstr, 0)?))),
                None => bail!(
                    "field `{name}` is a capability; caps can't be supplied here \
                     (e.g. a scheduled call can't carry a live cap)"
                ),
            }
            continue;
        }
        let iface_id = cap_field_interface_id(field);
        if let Some(v) = json_to_typed(node, field.get_type(), name, iface_id, resolver)? {
            out.push((name.to_string(), v));
        }
    }
    Ok(Value::Struct(out))
}

/// The interface id a typed-cap field declares, or 0 for a bare
/// `:Capability` / non-cap field.
fn cap_field_interface_id(field: capnp::schema::Field) -> u64 {
    use capnp::schema_capnp::{field as f, type_};
    (|| {
        let f::Slot(slot) = field.get_proto().which().ok()? else {
            return None;
        };
        if let type_::Interface(i) = slot.get_type().ok()?.which().ok()? {
            return Some(i.get_type_id());
        }
        None
    })()
    .unwrap_or(0)
}

/// Coerce one JSON node to a `Value` per a declared capnp field type.
/// Returns None when the field should be left unset (an unresolved or
/// empty cap reference). `cap_iface_id` is the interface the field wants
/// (0 for a bare Capability); `resolver` turns a cap reference into a
/// live `CapRef`.
fn json_to_typed(
    node: &Json,
    ty: capnp::introspect::Type,
    ctx: &str,
    cap_iface_id: u64,
    resolver: &mut Option<CapResolver>,
) -> Result<Option<Value>> {
    use TypeVariant::*;
    Ok(Some(match ty.which() {
        Void => Value::Null,
        Bool => Value::Bool(node.as_bool().ok_or_else(|| mismatch(ctx, "bool", node))?),
        Int8 | Int16 | Int32 | Int64 => {
            Value::Int(as_i64(node).ok_or_else(|| mismatch(ctx, "integer", node))?)
        }
        UInt8 | UInt16 | UInt32 | UInt64 => {
            Value::UInt(as_u64(node).ok_or_else(|| mismatch(ctx, "unsigned integer", node))?)
        }
        Float32 | Float64 => {
            Value::Float(node.as_f64().ok_or_else(|| mismatch(ctx, "number", node))?)
        }
        Text => Value::Text(
            node.as_str()
                .ok_or_else(|| mismatch(ctx, "string", node))?
                .to_string(),
        ),
        Data => {
            use base64::Engine;
            let s = node
                .as_str()
                .ok_or_else(|| mismatch(ctx, "base64 string", node))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(s)
                .map_err(|e| anyhow!("field `{ctx}`: invalid base64: {e}"))?;
            Value::Data(bytes)
        }
        Enum(_) => {
            // Accept the enumerant name (string) or its ordinal (number).
            match node {
                Json::String(s) => Value::Enum(s.clone()),
                Json::Number(n) if n.as_u64().is_some() => Value::UInt(n.as_u64().unwrap()),
                _ => bail!("field `{ctx}` expects an enum name or ordinal, got {node}"),
            }
        }
        List(el) => {
            let arr = node
                .as_array()
                .ok_or_else(|| mismatch(ctx, "array", node))?;
            let mut items = Vec::with_capacity(arr.len());
            for (i, item) in arr.iter().enumerate() {
                // List elements have no per-element field, so a typed-cap
                // list carries the element interface id from `el`.
                let el_iface = cap_type_interface_id(el);
                if let Some(v) =
                    json_to_typed(item, el, &format!("{ctx}[{i}]"), el_iface, resolver)?
                {
                    items.push(v);
                }
            }
            Value::List(items)
        }
        Struct(rbs) => {
            let s: StructSchema = rbs.into();
            json_to_value_with(node, s, resolver)?
        }
        Capability => {
            // A cap reference is a non-empty string naming a cap the user
            // holds. Empty / non-string → leave the field unset.
            let Some(refstr) = node.as_str().filter(|s| !s.is_empty()) else {
                return Ok(None);
            };
            match resolver {
                Some(resolve) => Value::Cap(resolve(refstr, cap_iface_id)?),
                None => bail!(
                    "field `{ctx}` is a capability; caps can't be supplied here \
                     (e.g. a scheduled call can't carry a live cap)"
                ),
            }
        }
        // AnyPointer has no JSON form; leave unset.
        AnyPointer => return Ok(None),
    }))
}

/// Interface id declared by a cap `Type` (for list elements), or 0.
fn cap_type_interface_id(ty: capnp::introspect::Type) -> u64 {
    // The introspect Type doesn't expose the interface id directly for a
    // bare Capability variant; element caps are rare and resolve by the
    // value's own interface, so 0 (unconstrained) is acceptable here.
    let _ = ty;
    0
}

fn as_i64(node: &Json) -> Option<i64> {
    node.as_i64().or_else(|| node.as_f64().map(|f| f as i64))
}
fn as_u64(node: &Json) -> Option<u64> {
    node.as_u64()
        .or_else(|| node.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64))
}

fn mismatch(ctx: &str, want: &str, got: &Json) -> anyhow::Error {
    anyhow!("field `{ctx}` expects {want}, got {got}")
}

fn ce(e: capnp::Error) -> anyhow::Error {
    anyhow!("capnp: {e}")
}
fn ue(e: std::str::Utf8Error) -> anyhow::Error {
    anyhow!("utf8: {e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::introspect::{value_to_struct, SchemaCache};
    use serde_json::json;

    /// llm.chat.v1's Part — an unnamed union (text | blob | toolCall |
    /// toolResult) — reached through chat(req).messages[].parts[].
    fn part_schema() -> (SchemaCache, StructSchema) {
        use capnp::introspect::TypeVariant;
        let (id, bytes) =
            darude_wire::compiled_schema_for_interface("llm.chat.v1").expect("compiled");
        let cache = SchemaCache::new();
        cache.load(bytes).unwrap();
        let chat = cache
            .interface(id)
            .unwrap()
            .method("chat")
            .cloned()
            .unwrap();
        let mut s = cache.struct_schema(chat.param_type_id).unwrap();
        for name in ["req", "messages", "parts"] {
            let f = s.get_field_by_name(name).unwrap();
            let ty = match f.get_type().which() {
                TypeVariant::List(el) => el,
                _ => f.get_type(),
            };
            let TypeVariant::Struct(raw) = ty.which() else {
                panic!("{name} not a struct")
            };
            s = raw.into();
        }
        (cache, s)
    }

    #[test]
    fn two_alternatives_of_a_union_are_refused() {
        let (_cache, part) = part_schema();
        let e = json_to_value(&json!({"text": "hi", "blob": {"data": ""}}), part).unwrap_err();
        assert!(
            e.to_string()
                .contains("`text` and `blob` are alternatives — set exactly one of"),
            "{e}"
        );
        // One alternative (and nulls for the rest) is fine.
        assert!(json_to_value(&json!({"text": "hi", "blob": null}), part).is_ok());
    }

    // web.fetch.v1's FetchRequest { url :Text, maxBytes :UInt32 } is a
    // convenient real struct with a Text + an unsigned field.
    fn fetch_request_schema() -> (SchemaCache, StructSchema) {
        let (iface_id, bytes) =
            darude_wire::schema_for_interface("web.fetch.v1").expect("known interface");
        let cache = SchemaCache::new();
        cache.load(bytes).unwrap();
        let info = cache.interface(iface_id).unwrap();
        let fetch = info.method("fetch").expect("fetch method");
        // Params is fetch$Params { req :FetchRequest }; drill to the
        // FetchRequest struct via the param struct's `req` field.
        let params = cache.struct_schema(fetch.param_type_id).unwrap();
        let req_field = params.get_fields().unwrap().get(0);
        let TypeVariant::Struct(rbs) = req_field.get_type().which() else {
            panic!("req is a struct");
        };
        (cache, rbs.into())
    }

    #[test]
    fn json_to_value_coerces_by_schema() {
        let (_cache, schema) = fetch_request_schema();
        // JSON numbers are untyped; the schema makes maxBytes a UInt.
        let j = json!({ "url": "http://x", "maxBytes": 2048 });
        let v = json_to_value(&j, schema).unwrap();
        assert_eq!(v.get("url"), Some(&Value::Text("http://x".into())));
        assert_eq!(v.get("maxBytes"), Some(&Value::UInt(2048)));
    }

    #[test]
    fn json_to_value_marshals_into_capnp_struct() {
        let (_cache, schema) = fetch_request_schema();
        // maxBytes=0 exercises the zero-default surviving the coercion.
        let j = json!({ "url": "http://example.com", "maxBytes": 0 });
        let v = json_to_value(&j, schema).unwrap();
        // Marshal into the dynamic struct (the same init_dynamic path
        // dynamic_call uses) to prove the coerced Value actually fits
        // the capnp struct without erroring.
        let mut msg = capnp::message::Builder::new_default();
        let mut b = msg
            .init_root::<capnp::any_pointer::Builder>()
            .init_dynamic(schema)
            .unwrap();
        value_to_struct(&mut b, &v).expect("coerced Value fits the param struct");
        // (Reading dynamic structs back is covered by introspect's
        // dynamic_dispatch test; here we only assert the marshal path.)
    }

    // cap_echo.v1.echo(llm :Llm.LlmChatV1, …) — its param struct has a typed-cap
    // field, the real shape a cap resolver handles. We use cap_echo.v1 (a
    // platform-internal interface still compiled into darude_wire) since app
    // interfaces moved into their bridge crates.
    fn echo_params_schema() -> (SchemaCache, StructSchema) {
        let (iface_id, bytes) =
            darude_wire::compiled_schema_for_interface("cap_echo.v1").expect("compiled");
        let cache = SchemaCache::new();
        cache.load(bytes).unwrap();
        let info = cache.interface(iface_id).unwrap();
        let echo = info.method("echo").expect("echo method");
        let params = cache.struct_schema(echo.param_type_id).unwrap();
        (cache, params)
    }

    #[test]
    fn cap_param_without_resolver_errors_when_supplied() {
        let (_c, schema) = echo_params_schema();
        // A supplied cap reference with no resolver is rejected...
        let j = json!({ "llm": "grain://g/llm.chat.v1" });
        assert!(json_to_value(&j, schema).is_err());
        // ...but an absent/empty cap field is simply left unset.
        let v = json_to_value(&json!({}), schema).unwrap();
        assert_eq!(v, Value::Struct(vec![]));
    }

    #[test]
    fn cap_param_with_resolver_binds_cap() {
        use crate::introspect::CapRef;
        use capnp::capability::FromClientHook;
        // A stub hook to hand back as the resolved cap.
        // The stub is just an opaque hook handed back as the resolved cap — its
        // concrete type is irrelevant (the test checks the declared param
        // interface, llm.chat.v1, not the hook's type), so use a trivial one.
        struct Stub;
        impl darude_wire::change_listener_capnp::change_listener::Server for Stub {}
        let client: darude_wire::change_listener_capnp::change_listener::Client =
            capnp_rpc::new_client(Stub);
        let want = darude_wire::schema_for_interface("llm.chat.v1").unwrap().0;

        let (_c, schema) = echo_params_schema();
        let mut seen: Option<(String, u64)> = None;
        let mut resolver = |refstr: &str, iface: u64| -> anyhow::Result<CapRef> {
            seen = Some((refstr.to_string(), iface));
            Ok(CapRef::new(client.clone().into_client_hook(), iface))
        };
        let mut opt: Option<CapResolver> = Some(&mut resolver);
        let j = json!({ "llm": "grain://g/llm.chat.v1" });
        let v = json_to_value_with(&j, schema, &mut opt).unwrap();
        // The resolver saw the reference + the field's declared interface.
        assert_eq!(seen, Some(("grain://g/llm.chat.v1".to_string(), want)));
        // The field bound a live cap.
        assert!(matches!(v.get("llm"), Some(Value::Cap(_))));
    }

    #[test]
    fn value_to_json_maps_scalars_and_caps() {
        let v = Value::struct_of([
            ("n".to_string(), Value::UInt(7)),
            ("flag".to_string(), Value::Bool(false)),
            ("name".to_string(), Value::Text("x".into())),
            (
                "items".to_string(),
                Value::List(vec![Value::Int(-1), Value::Int(2)]),
            ),
        ]);
        let j = value_to_json(&v);
        assert_eq!(j.get("n").and_then(Json::as_u64), Some(7));
        assert_eq!(j.get("flag").and_then(Json::as_bool), Some(false));
        assert_eq!(j.get("name").and_then(Json::as_str), Some("x"));
        assert_eq!(
            j.get("items").and_then(Json::as_array).map(|a| a.len()),
            Some(2)
        );
    }
}
