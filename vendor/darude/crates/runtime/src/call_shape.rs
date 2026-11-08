// The calling convention every caller-facing surface shares: agent tool
// calls, the shell console, and the in-app script engines (code-exec's
// Monty, Node-RED, Grist). One rule, here, so they can't drift:
//
//   A method whose parameter struct is a single struct field —
//   `foo (req :FooRequest)`, the common schema shape — is called with
//   that inner struct's fields: `foo(field=…)`, never `foo(req={…})`.
//   A result that is a single struct field (`-> (resp :T)`) reads back
//   as that inner struct. Any other shape maps field-for-field.
//
// The wrapper is a schema-authoring artifact; callers never see it.

use capnp::introspect::TypeVariant;
use capnp::schema::StructSchema;

use crate::introspect::{MethodInfo, SchemaCache, Value};

/// If `schema` is a struct with exactly one field that is itself a
/// struct, that field's name and the inner schema.
pub fn single_struct_field(schema: StructSchema) -> Option<(String, StructSchema)> {
    let fields = schema.get_fields().ok()?;
    if fields.len() != 1 {
        return None;
    }
    let f = fields.get(0);
    if let TypeVariant::Struct(inner) = f.get_type().which() {
        let name = f.get_proto().get_name().ok()?.to_str().ok()?.to_string();
        Some((name, inner.into()))
    } else {
        None
    }
}

/// The struct a caller fills for a method with `param_schema`.
pub fn visible_params(param_schema: StructSchema) -> StructSchema {
    single_struct_field(param_schema)
        .map(|(_, inner)| inner)
        .unwrap_or(param_schema)
}

/// The struct a caller reads back for a method with `result_schema`.
pub fn visible_result(result_schema: StructSchema) -> StructSchema {
    single_struct_field(result_schema)
        .map(|(_, inner)| inner)
        .unwrap_or(result_schema)
}

/// Turn args built against `visible_params` into the wire param struct.
pub fn wrap_args(param_schema: StructSchema, args: Value) -> Value {
    match single_struct_field(param_schema) {
        Some((field, _)) => Value::struct_of([(field, args)]),
        None => args,
    }
}

/// Turn a wire result into what a caller sees.
pub fn unwrap_result(result_schema: StructSchema, value: Value) -> Value {
    match single_struct_field(result_schema) {
        Some((field, _)) => value.get(&field).cloned().unwrap_or(Value::Null),
        None => value,
    }
}

/// Field names a caller passes, in schema order (non-union fields).
pub fn param_names(param_schema: StructSchema) -> Vec<String> {
    let Ok(fields) = visible_params(param_schema).get_non_union_fields() else {
        return Vec::new();
    };
    (0..fields.len())
        .filter_map(|i| {
            fields
                .get(i)
                .get_proto()
                .get_name()
                .ok()?
                .to_str()
                .ok()
                .map(str::to_string)
        })
        .collect()
}

/// Refuse argument names the method doesn't take (top level), naming the
/// signature — so a wrong call shape (e.g. a stray `req` wrapper) fails
/// loudly instead of its fields being silently dropped.
pub fn check_arg_names(
    method: &MethodInfo,
    cache: &SchemaCache,
    args: &serde_json::Value,
) -> Result<(), String> {
    let Some(obj) = args.as_object() else {
        return Ok(());
    };
    let Some(params) = cache.struct_schema(method.param_type_id) else {
        return Ok(());
    };
    let known = param_names(params);
    let unknown: Vec<&str> = obj
        .keys()
        .map(String::as_str)
        .filter(|k| !known.iter().any(|n| n == k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let sig = signature(method, cache).unwrap_or_else(|| method.name.clone());
    Err(format!(
        "{} doesn't take {} — call it as `{sig}`",
        method.name,
        unknown
            .iter()
            .map(|k| format!("`{k}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// A one-line call signature in the shared convention, e.g.
/// `getStates(domain: str, area: str, entityIds: list[str]) -> StateList`.
/// Shown verbatim in tool descriptions, the console and script listings,
/// so every surface tells the caller the same shape.
pub fn signature(method: &MethodInfo, cache: &SchemaCache) -> Option<String> {
    let params = visible_params(cache.struct_schema(method.param_type_id)?);
    let result_schema = cache.struct_schema(method.result_type_id)?;
    let args = field_list(params, ", ");
    let ret = match single_struct_field(result_schema) {
        Some((_, inner)) => struct_name(inner),
        None => {
            let fields = field_list(result_schema, ", ");
            if fields.is_empty() {
                "None".to_string()
            } else {
                format!("{{{fields}}}")
            }
        }
    };
    Some(format!("{}({args}) -> {ret}", method.name))
}

fn field_list(schema: StructSchema, sep: &str) -> String {
    let Ok(fields) = schema.get_fields() else {
        return String::new();
    };
    let mut out: Vec<String> = Vec::new();
    // The members of an unnamed union render as one `a: T | b: U` entry,
    // where the first of them sits.
    let mut union_at: Option<usize> = None;
    for i in 0..fields.len() {
        let f = fields.get(i);
        let Some(name) = f.get_proto().get_name().ok().and_then(|n| n.to_str().ok()) else {
            continue;
        };
        let item = format!("{name}: {}", type_name(f.get_type(), f));
        let in_union =
            f.get_proto().get_discriminant_value() != capnp::schema_capnp::field::NO_DISCRIMINANT;
        match (in_union, union_at) {
            (true, Some(at)) => out[at] = format!("{} | {item}", out[at]),
            (true, None) => {
                union_at = Some(out.len());
                out.push(item);
            }
            (false, _) => out.push(item),
        }
    }
    out.join(sep)
}

fn type_name(ty: capnp::introspect::Type, field: capnp::schema::Field) -> String {
    use TypeVariant::*;
    match ty.which() {
        Void => "None".into(),
        Bool => "bool".into(),
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 => "int".into(),
        Float32 | Float64 => "float".into(),
        Text => "str".into(),
        Data => "bytes".into(),
        List(el) => format!("list[{}]", type_name(el, field)),
        Enum(_) => "str".into(),
        Struct(s) => struct_name(s.into()),
        Capability => "cap".into(),
        AnyPointer if crate::tool_schema::is_bare_capability(field) => "cap".into(),
        AnyPointer => "any".into(),
    }
}

/// A struct's short name: "homeassistant-v1.capnp:HomeassistantV1.StateList"
/// → "StateList".
fn struct_name(s: StructSchema) -> String {
    let proto = s.get_proto();
    let Ok(Ok(full)) = proto
        .get_display_name()
        .map(|n| n.to_str().map(str::to_string))
    else {
        return "struct".into();
    };
    let short = &full[proto.get_display_name_prefix_length() as usize..];
    short.rsplit(['.', ':']).next().unwrap_or(short).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_union_renders_as_alternatives() {
        use capnp::introspect::TypeVariant;
        let (id, bytes) =
            darude_wire::compiled_schema_for_interface("llm.chat.v1").expect("compiled");
        let cache = crate::introspect::SchemaCache::new();
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
                panic!("{name}")
            };
            s = raw.into();
        }
        let got = field_list(s, ", ");
        assert!(got.starts_with("text: str | blob: "), "{got}");
        assert_eq!(got.matches(" | ").count(), 3, "{got}");
    }

    fn fs_cache() -> (SchemaCache, crate::introspect::InterfaceInfo) {
        let (id, bytes) = darude_wire::compiled_schema_for_interface("fs.v1").unwrap();
        let cache = SchemaCache::new();
        cache.load(bytes).unwrap();
        let info = cache.interface(id).unwrap();
        (cache, info)
    }

    #[test]
    fn a_single_struct_param_is_flattened_and_others_are_not() {
        let (cache, info) = fs_cache();
        // watch @16 (req :WatchRequest): callers pass WatchRequest's fields.
        let watch = info.method("watch").unwrap();
        let p = cache.struct_schema(watch.param_type_id).unwrap();
        assert_eq!(param_names(p), ["notifyMe"]);
        let wrapped = wrap_args(p, Value::struct_of([("notifyMe".to_string(), Value::Null)]));
        assert!(wrapped.get("req").is_some(), "wire shape keeps the wrapper");
        // stat @1 (path :Text): field-for-field.
        let stat = info.method("stat").unwrap();
        let p = cache.struct_schema(stat.param_type_id).unwrap();
        assert_eq!(param_names(p), ["path"]);
        let args = Value::struct_of([("path".to_string(), Value::Text("/".into()))]);
        assert_eq!(wrap_args(p, args.clone()), args);
    }

    #[test]
    fn unknown_argument_names_are_refused_with_the_signature() {
        let (cache, info) = fs_cache();
        let stat = info.method("stat").unwrap();
        assert!(check_arg_names(stat, &cache, &serde_json::json!({"path": "/"})).is_ok());
        let err =
            check_arg_names(stat, &cache, &serde_json::json!({"req": {"path": "/"}})).unwrap_err();
        assert!(err.contains("doesn't take `req`"), "{err}");
        assert!(err.contains("`stat(path: str) -> "), "{err}");
    }

    #[test]
    fn signatures_use_the_flat_shape() {
        let (cache, info) = fs_cache();
        let sig = signature(info.method("watch").unwrap(), &cache).unwrap();
        assert_eq!(sig, "watch(notifyMe: cap) -> {watch: cap}");
        let sig = signature(info.method("stat").unwrap(), &cache).unwrap();
        assert!(sig.starts_with("stat(path: str) -> "), "{sig}");
        let sig = signature(info.method("search").unwrap(), &cache).unwrap();
        assert!(!sig.contains("req"), "{sig}");
    }
}
