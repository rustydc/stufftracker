// Lower a capnp param/result struct to JSON Schema.
//
// Pure functions over a runtime-loaded `StructSchema`: walk the fields
// and emit the JSON Schema that describes them. Two consumers share this:
// the LLM provider (which wraps the result in an OpenAI tool definition)
// and the platform console (which renders an HTML form from it). Keeping
// the lowering here means both agree on how each capnp type maps to JSON.
//
// Type mapping: scalars → boolean/integer/number/string; Data → a
// base64 string; Enum → a string with an `enum` list; List(T) →
// array+items; nested Struct → recursive object. Capabilities and
// `OpaqueCap` (matched by struct type id) lower to a "cap-ref" string —
// a placeholder the caller resolves to a real cap; so does a bare
// `:Capability` field. A method whose params or results transitively
// contain a true AnyPointer can't be lowered (no JSON shape for an
// untyped pointer); `struct_has_unsupported` detects that so callers can
// skip the method.

use std::collections::HashSet;

use anyhow::Result;
use capnp::introspect::{Type, TypeVariant};
use capnp::schema::{EnumSchema, StructSchema};
use serde_json::{json, Map, Value};

use crate::introspect::SchemaCache;
use crate::opaque_cap;

/// Schema id of `introspectable.OpaqueCap`. Anywhere this struct appears
/// in a param/result, it lowers to a cap-ref string rather than its wire
/// fields. (Structs implement HasTypeId on Reader/Builder, not Owned, so
/// we go through Reader with an arbitrary 'static lifetime.)
pub fn opaque_cap_type_id() -> u64 {
    use capnp::traits::HasTypeId;
    <opaque_cap::Reader<'static> as HasTypeId>::TYPE_ID
}

/// Schema id of the platform-shared `Blob` (schemas/blob.capnp). Blob
/// params keep their full object shape but get an `x-blob: true`
/// annotation, so the console renders a file picker instead of three
/// bare fields (models just fill the object; the annotation is inert
/// to them).
pub fn blob_type_id() -> u64 {
    use capnp::traits::HasTypeId;
    <crate::blob::Reader<'static> as HasTypeId>::TYPE_ID
}

/// JSON Schema for a struct: an object with one property per field, all
/// listed as required (capnp has no "optional field" concept at this
/// layer — absent fields take their defaults). Pass the `SchemaCache`
/// the struct was loaded from to carry field doc comments (injected
/// into the schema bytes' sourceInfo at build time) into per-property
/// `description`s; None lowers without docs.
pub fn lower_struct(schema: StructSchema, docs: Option<&SchemaCache>) -> Result<Value> {
    lower_struct_guarded(schema, docs, &mut HashSet::new())
}

/// First paragraph of a schema doc comment, reflowed: schemas
/// hard-wrap at source width, and neither tool definitions nor
/// completion details want the line breaks. Blank line ends the
/// paragraph — the rest belongs to hover/reference surfaces, not to
/// every prompt.
pub fn first_paragraph(doc: &str) -> String {
    let para = doc.split("\n\n").next().unwrap_or(doc);
    para.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `lower_struct` with a cycle guard: `in_progress` tracks the struct
/// ids on the current walk, so a RECURSIVE type (trigger.v1's
/// TriggerValue holds a List(TriggerValue)) truncates to a permissive
/// stub instead of recursing forever. The stub only loosens the
/// advisory JSON Schema — dispatch-side arg coercion walks the real
/// capnp schema against actual data, which always terminates.
fn lower_struct_guarded(
    schema: StructSchema,
    docs: Option<&SchemaCache>,
    in_progress: &mut HashSet<u64>,
) -> Result<Value> {
    let id = schema.get_proto().get_id();
    if !in_progress.insert(id) {
        return Ok(json!({
            "type": "object",
            "description": "recursive occurrence of the enclosing struct type; \
                            same shape as its outer definition",
        }));
    }
    let out = lower_struct_fields(schema, docs, in_progress);
    in_progress.remove(&id);
    out
}

fn lower_struct_fields(
    schema: StructSchema,
    docs: Option<&SchemaCache>,
    in_progress: &mut HashSet<u64>,
) -> Result<Value> {
    let struct_id = schema.get_proto().get_id();
    let fields = schema.get_fields()?;
    let union_members = union_member_names(schema);
    let mut properties = Map::new();
    let mut required = Vec::new();
    for i in 0..fields.len() {
        let field = fields.get(i);
        let name = field.get_proto().get_name()?.to_str()?.to_string();
        let mut lowered = if is_bare_capability(field) {
            cap_ref_schema("a capability from a prior result (e.g. a Trigger)")
        } else {
            lower_type_guarded(field.get_type(), docs, in_progress)?
        };
        // A typed-cap field knows which interface it wants (the id lives
        // on the field's slot, not the Type), so annotate the cap-ref
        // with the interface name — the console form uses it to offer
        // only grains that provide that interface.
        if matches!(field.get_type().which(), TypeVariant::Capability) {
            if let Some(iface) = cap_field_interface_name(field) {
                if let Value::Object(m) = &mut lowered {
                    m.insert("x-cap-interface".into(), Value::String(iface));
                }
            }
        }
        // Field doc comment (first paragraph) → property description.
        // A type-level description (the cap-ref/blob protocol notes)
        // stays — the doc prepends, since both matter to a caller.
        if let Some(doc) = docs.and_then(|d| d.member_doc(struct_id, &name)) {
            let para = first_paragraph(&doc);
            if let Value::Object(m) = &mut lowered {
                let merged = match m.get("description").and_then(Value::as_str) {
                    Some(existing) => format!("{para} — {existing}"),
                    None => para,
                };
                m.insert("description".into(), Value::String(merged));
            }
        }
        // An unnamed-union member is one alternative: never required,
        // and its description names the others.
        if union_members.contains(&name) {
            let note = format!(
                "Set exactly one of {}.",
                union_members
                    .iter()
                    .map(|m| format!("`{m}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if let Value::Object(m) = &mut lowered {
                let merged = match m.get("description").and_then(Value::as_str) {
                    Some(existing) => format!("{note} {existing}"),
                    None => note,
                };
                m.insert("description".into(), Value::String(merged));
            }
            properties.insert(name, lowered);
            continue;
        }
        properties.insert(name.clone(), lowered);
        required.push(Value::String(name));
    }
    Ok(json!({
        "type": "object",
        "properties": Value::Object(properties),
        "required": Value::Array(required),
    }))
}

/// The members of a struct's unnamed union, in declaration order (empty
/// when it has none). Each is an alternative: a value sets exactly one.
pub fn union_member_names(schema: StructSchema) -> Vec<String> {
    let Ok(fields) = schema.get_fields() else {
        return Vec::new();
    };
    (0..fields.len())
        .map(|i| fields.get(i))
        .filter(|f| {
            f.get_proto().get_discriminant_value() != capnp::schema_capnp::field::NO_DISCRIMINANT
        })
        .filter_map(|f| Some(f.get_proto().get_name().ok()?.to_str().ok()?.to_string()))
        .collect()
}

/// True for a bare `:Capability` field. capnp's introspect layer reports
/// it as `AnyPointer` (its `Capability` variant is typed interfaces only),
/// but unlike a true AnyPointer it has a JSON form: a cap-ref string. The
/// durable `watch(notifyMe :Capability)` methods every bridge exposes are
/// this shape, so treating it as unsupported hid them from agents.
pub fn is_bare_capability(field: capnp::schema::Field) -> bool {
    use capnp::schema_capnp::{field as f, type_};
    (|| {
        let f::Slot(slot) = field.get_proto().which().ok()? else {
            return None;
        };
        let type_::AnyPointer(ap) = slot.get_type().ok()?.which().ok()? else {
            return None;
        };
        let type_::any_pointer::Unconstrained(u) = ap.which().ok()? else {
            return None;
        };
        Some(matches!(
            u.which().ok()?,
            type_::any_pointer::unconstrained::Capability(())
        ))
    })()
    .unwrap_or(false)
}

/// The platform-known interface name a typed-cap field wants, or None
/// for a bare `:Capability` (no declared interface) or an unknown one.
fn cap_field_interface_name(field: capnp::schema::Field) -> Option<String> {
    use capnp::schema_capnp::{field as f, type_};
    let f::Slot(slot) = field.get_proto().which().ok()? else {
        return None;
    };
    if let type_::Interface(i) = slot.get_type().ok()?.which().ok()? {
        let id = i.get_type_id();
        if id != 0 {
            return darude_wire::interface_name_for_id(id);
        }
    }
    None
}

/// JSON Schema for one field type.
pub fn lower_type(ty: Type, docs: Option<&SchemaCache>) -> Result<Value> {
    lower_type_guarded(ty, docs, &mut HashSet::new())
}

fn lower_type_guarded(
    ty: Type,
    docs: Option<&SchemaCache>,
    in_progress: &mut HashSet<u64>,
) -> Result<Value> {
    use TypeVariant::*;
    Ok(match ty.which() {
        Void => json!({}),
        Bool => json!({ "type": "boolean" }),
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => {
            json!({ "type": "integer" })
        }
        Float32 | Float64 => json!({ "type": "number" }),
        Text => json!({ "type": "string" }),
        Data => json!({ "type": "string", "contentEncoding": "base64" }),
        List(el) => json!({ "type": "array", "items": lower_type_guarded(el, docs, in_progress)? }),
        Enum(es) => {
            let schema: EnumSchema = es.into();
            let enumerants = schema.get_enumerants()?;
            let mut names = Vec::new();
            for i in 0..enumerants.len() {
                names.push(Value::String(
                    enumerants
                        .get(i)
                        .get_proto()
                        .get_name()?
                        .to_str()?
                        .to_string(),
                ));
            }
            json!({ "type": "string", "enum": Value::Array(names) })
        }
        Struct(rbs) => {
            let s: StructSchema = rbs.into();
            // OpaqueCap lowers as a cap-ref string by struct id, so
            // `List(OpaqueCap)` naturally becomes an array of strings.
            if s.get_proto().get_id() == opaque_cap_type_id() {
                cap_ref_schema("a cap the caller holds")
            } else {
                let mut lowered = lower_struct_guarded(s, docs, in_progress)?;
                if s.get_proto().get_id() == blob_type_id() {
                    if let Value::Object(m) = &mut lowered {
                        m.insert("x-blob".into(), json!(true));
                    }
                }
                lowered
            }
        }
        Capability => cap_ref_schema("a capability from a prior result"),
        AnyPointer => json!({ "type": "null", "x-error": "anyPointer unsupported" }),
    })
}

fn cap_ref_schema(what: &str) -> Value {
    json!({
        "type": "string",
        "x-cap-ref": true,
        "description": format!("Cap-ref id (e.g. 'cap://rustpad') of {what}"),
    })
}

/// True if a struct transitively contains an AnyPointer field — i.e. it
/// can't be fully lowered to JSON Schema. `OpaqueCap` is exempt (its
/// bare-Capability field is an AnyPointer at the wire level, but it
/// lowers to a cap-ref string, so the pointer is irrelevant here).
pub fn struct_has_unsupported(schema: StructSchema, visited: &mut HashSet<u64>) -> Result<bool> {
    let id = schema.get_proto().get_id();
    if id == opaque_cap_type_id() {
        return Ok(false);
    }
    if !visited.insert(id) {
        return Ok(false); // cycle: assume safe
    }
    let fields = schema.get_fields()?;
    for i in 0..fields.len() {
        let field = fields.get(i);
        if is_bare_capability(field) {
            continue;
        }
        if type_has_unsupported(field.get_type(), visited)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn type_has_unsupported(ty: Type, visited: &mut HashSet<u64>) -> Result<bool> {
    Ok(match ty.which() {
        TypeVariant::AnyPointer => true,
        TypeVariant::List(el) => type_has_unsupported(el, visited)?,
        TypeVariant::Struct(rbs) => {
            let s: StructSchema = rbs.into();
            struct_has_unsupported(s, visited)?
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::introspect::SchemaCache;

    #[test]
    fn blob_params_are_annotated() {
        // llm.chat.v1's chat(req).messages[].parts[].blob is the shared
        // Blob; the lowering keeps its object shape (models fill the
        // fields) but tags it x-blob so the console renders a file
        // picker.
        let (id, bytes) =
            darude_wire::compiled_schema_for_interface("llm.chat.v1").expect("compiled");
        let cache = SchemaCache::new();
        cache.load(bytes).unwrap();
        let info = cache.interface(id).unwrap();
        let chat = info.method("chat").expect("chat method");
        let params = cache.struct_schema(chat.param_type_id).unwrap();
        let j = lower_struct(params, None).unwrap();
        let blob = &j["properties"]["req"]["properties"]["messages"]["items"]["properties"]
            ["parts"]["items"]["properties"]["blob"];
        assert_eq!(
            blob["x-blob"],
            serde_json::json!(true),
            "blob schema: {blob}"
        );
        assert_eq!(blob["type"], "object");
        assert!(blob["properties"].get("mimeType").is_some());
        // Data lowers as a base64 string.
        assert_eq!(blob["properties"]["data"]["contentEncoding"], "base64");
    }

    #[test]
    fn union_members_are_alternatives_not_required() {
        // llm.chat.v1's Part is an unnamed union (text | blob | toolCall |
        // toolResult): none is required, and each says so.
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
        let params = cache.struct_schema(chat.param_type_id).unwrap();
        let j = lower_struct(params, None).unwrap();
        let part = &j["properties"]["req"]["properties"]["messages"]["items"]["properties"]
            ["parts"]["items"];
        assert_eq!(part["required"], serde_json::json!([]), "part: {part}");
        let d = part["properties"]["text"]["description"].as_str().unwrap();
        assert!(
            d.starts_with("Set exactly one of `text`, `blob`, `toolCall`, `toolResult`."),
            "{d}"
        );
    }

    // trigger.v1's TriggerValue is RECURSIVE (list/object arms hold
    // TriggerValue again). Lowering trigger()'s params must terminate —
    // the cycle guard truncates the inner occurrence to a stub instead
    // of overflowing the stack (which is exactly what happened the first
    // time a Trigger cap was registered as an agent tool).
    #[test]
    fn recursive_struct_lowering_terminates() {
        let (id, bytes) =
            darude_wire::compiled_schema_for_interface("trigger.v1").expect("compiled");
        let cache = SchemaCache::new();
        cache.load(bytes).unwrap();
        let info = cache.interface(id).unwrap();
        let trig = info.method("trigger").expect("trigger method");
        let params = cache.struct_schema(trig.param_type_id).unwrap();
        let j = lower_struct(params, None).unwrap();
        // The outer payloads list lowered with real arms...
        let payload = &j["properties"]["payloads"]["items"];
        assert!(payload["properties"].get("list").is_some(), "schema: {j}");
        // ...and the recursive inner occurrence is a truncated stub, not
        // an infinite expansion.
        let inner = &payload["properties"]["list"]["items"];
        assert!(
            inner["properties"].as_object().is_none(),
            "inner should be a stub: {inner}"
        );
    }
}
