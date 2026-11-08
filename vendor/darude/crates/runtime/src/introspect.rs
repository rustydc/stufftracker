// Introspectable-cap dispatch — the receiver half of the OpaqueCap
// protocol (schemas/introspectable.capnp), built on the vendored capnp
// crate's runtime `SchemaLoader`.
//
// Given a capability whose interface is only known at runtime (its
// serialized `CodeGeneratorRequest` arrives alongside it), this lets a
// consumer enumerate the interface's methods, marshal a neutral `Value`
// tree into the method's parameter struct, dispatch the call, and read
// the result struct back out as `Value`s — without any compile-time
// binding to the interface. The code-exec grain uses it to expose
// arbitrary tool caps to a Monty script; the same machinery is what an
// agent provider would lower to LLM tool definitions.
//
// `Value` is the neutral, capnp-shaped bridge type: a host (Monty,
// JSON, ...) converts to/from `Value`, and the marshalling here turns
// `Value` into the dynamically-typed capnp struct the schema describes.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};
use capnp::any_pointer;
use capnp::capability::Client;
use capnp::dynamic_list;
use capnp::dynamic_struct;
use capnp::dynamic_value;
use capnp::introspect::TypeVariant;
use capnp::private::capability::ClientHook;
use capnp::schema::StructSchema;
use capnp::schema_capnp::node;
use capnp::schema_loader::SchemaLoader;

fn ce(e: capnp::Error) -> anyhow::Error {
    anyhow!("capnp: {e}")
}

/// A live capability surfaced from (or bound into) a dynamically-typed
/// struct. Carries the underlying client hook plus the interface id the
/// schema declared for the field it came from (0 when the field is a
/// bare `Capability`, e.g. `OpaqueCap.cap`). Cloning bumps the hook's
/// refcount; caps are never equal (RPC identity isn't comparable here).
pub struct CapRef {
    hook: Box<dyn ClientHook>,
    interface_id: u64,
}

impl CapRef {
    pub fn new(hook: Box<dyn ClientHook>, interface_id: u64) -> Self {
        Self { hook, interface_id }
    }

    /// The interface id the schema declared for this cap, or 0 if the
    /// field was a bare `Capability`.
    pub fn interface_id(&self) -> u64 {
        self.interface_id
    }

    /// A fresh owned reference to the underlying hook.
    pub fn add_ref(&self) -> Box<dyn ClientHook> {
        self.hook.add_ref()
    }
}

impl Clone for CapRef {
    fn clone(&self) -> Self {
        Self {
            hook: self.hook.add_ref(),
            interface_id: self.interface_id,
        }
    }
}

impl std::fmt::Debug for CapRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CapRef(interface_id={:#x})", self.interface_id)
    }
}

impl PartialEq for CapRef {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

/// A neutral, capnp-shaped value. The bridge between a host value system
/// and the dynamically-typed capnp struct world.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Void, or an absent/unset field.
    Null,
    Bool(bool),
    /// Signed integer (Int8..Int64 fields).
    Int(i64),
    /// Unsigned integer (UInt8..UInt64 fields).
    UInt(u64),
    Float(f64),
    Text(String),
    Data(Vec<u8>),
    /// Enum, by enumerant name.
    Enum(String),
    List(Vec<Value>),
    /// Struct, as ordered (field-name, value) pairs.
    Struct(Vec<(String, Value)>),
    /// A live capability (a cap-typed field's value). Surfaced when
    /// reading a result struct; consumed when building a param struct.
    Cap(CapRef),
}

impl Value {
    /// Convenience constructor for a struct value.
    pub fn struct_of(fields: impl IntoIterator<Item = (String, Value)>) -> Self {
        Value::Struct(fields.into_iter().collect())
    }

    /// Looks up a field of a struct value by name.
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Struct(fields) => fields.iter().find(|(k, _)| k == name).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(x) => Some(*x),
            Value::UInt(x) => i64::try_from(*x).ok(),
            Value::Float(x) => Some(*x as i64),
            _ => None,
        }
    }

    fn as_u64(&self) -> Option<u64> {
        match self {
            Value::UInt(x) => Some(*x),
            Value::Int(x) => u64::try_from(*x).ok(),
            Value::Float(x) if *x >= 0.0 => Some(*x as u64),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(x) => Some(*x),
            Value::Int(x) => Some(*x as f64),
            Value::UInt(x) => Some(*x as f64),
            _ => None,
        }
    }
}

/// One method of an interface, as introspected from a loaded schema.
#[derive(Clone, Debug)]
pub struct MethodInfo {
    pub name: String,
    /// The method's index in the interface, i.e. its `@ordinal` — this
    /// is the `method_id` passed to `Client::new_call`.
    pub ordinal: u16,
    pub param_type_id: u64,
    pub result_type_id: u64,
    /// Doc comment from the schema's sourceInfo (the build step
    /// injects the source-extracted docs there — darude-schema-docs).
    /// None when the schema carried none for this method.
    pub doc: Option<String>,
}

/// An interface introspected from a loaded schema.
#[derive(Clone, Debug)]
pub struct InterfaceInfo {
    pub id: u64,
    pub methods: Vec<MethodInfo>,
    /// The interface's own doc comment, when the schema carried one.
    pub doc: Option<String>,
}

impl InterfaceInfo {
    pub fn method(&self, name: &str) -> Option<&MethodInfo> {
        self.methods.iter().find(|m| m.name == name)
    }
}

/// Loads and caches interface schemas received over the wire. Mirrors
/// the per-(connection, direction) cache the OpaqueCap protocol calls
/// for: the first time an `interfaceId` is seen its `schema` bytes are
/// loaded; subsequent sends may omit them.
pub struct SchemaCache {
    loader: SchemaLoader,
    loaded: RefCell<HashSet<u64>>,
    /// Docs read from loaded CGRs' sourceInfo: node id → (node doc,
    /// member docs keyed by member NAME). Member indices are joined to
    /// names at load time against the same CGR's nodes (methods for
    /// interfaces, fields for structs), so consumers never deal in
    /// sourceInfo's positional encoding.
    docs: RefCell<HashMap<u64, NodeDocs>>,
}

#[derive(Default, Clone)]
struct NodeDocs {
    doc: Option<String>,
    members: HashMap<String, String>,
}

impl Default for SchemaCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaCache {
    pub fn new() -> Self {
        Self {
            loader: SchemaLoader::new(),
            loaded: RefCell::new(HashSet::new()),
            docs: RefCell::new(HashMap::new()),
        }
    }

    /// Loads a serialized `CodeGeneratorRequest` (an `OpaqueCap.schema`
    /// payload, or one of `darude_wire`'s `*_SCHEMA` constants).
    pub fn load(&self, cgr_bytes: &[u8]) -> Result<()> {
        self.loader.load_serialized(cgr_bytes).map_err(ce)?;
        // Best-effort: docs missing or malformed never fail a load —
        // a schema without sourceInfo is simply undocumented.
        let _ = self.load_docs(cgr_bytes);
        Ok(())
    }

    /// Reads sourceInfo docs out of a CGR, joining member indices to
    /// member names via the CGR's own nodes.
    fn load_docs(&self, cgr_bytes: &[u8]) -> Result<()> {
        use capnp::schema_capnp::{code_generator_request, node};
        let msg = capnp::serialize::read_message(
            &mut &cgr_bytes[..],
            capnp::message::ReaderOptions::new(),
        )
        .map_err(ce)?;
        let cgr: code_generator_request::Reader = msg.get_root().map_err(ce)?;
        // node id -> member names in sourceInfo order (methods for
        // interfaces, fields for structs).
        let mut member_names: HashMap<u64, Vec<String>> = HashMap::new();
        for n in cgr.get_nodes().map_err(ce)?.iter() {
            let names: Vec<String> = match n.which().map_err(|e| anyhow!("{e}"))? {
                node::Interface(i) => i
                    .get_methods()
                    .map_err(ce)?
                    .iter()
                    .filter_map(|m| {
                        m.get_name()
                            .ok()
                            .and_then(|t| t.to_str().ok().map(String::from))
                    })
                    .collect(),
                node::Struct(s) => s
                    .get_fields()
                    .map_err(ce)?
                    .iter()
                    .filter_map(|f| {
                        f.get_name()
                            .ok()
                            .and_then(|t| t.to_str().ok().map(String::from))
                    })
                    .collect(),
                _ => continue,
            };
            member_names.insert(n.get_id(), names);
        }
        let mut docs = self.docs.borrow_mut();
        for si in cgr.get_source_info().map_err(ce)?.iter() {
            let id = si.get_id();
            let entry = docs.entry(id).or_default();
            if let Ok(t) = si.get_doc_comment() {
                if let Ok(s) = t.to_str() {
                    let s = s.trim_end();
                    if !s.is_empty() {
                        entry.doc = Some(s.to_string());
                    }
                }
            }
            if let (Ok(members), Some(names)) = (si.get_members(), member_names.get(&id)) {
                for (i, m) in members.iter().enumerate() {
                    let Some(name) = names.get(i) else { continue };
                    if let Ok(t) = m.get_doc_comment() {
                        if let Ok(s) = t.to_str() {
                            let s = s.trim_end();
                            if !s.is_empty() {
                                entry.members.insert(name.clone(), s.to_string());
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// A node's own doc comment (interface or struct), if any loaded
    /// schema carried one.
    pub fn node_doc(&self, id: u64) -> Option<String> {
        self.docs.borrow().get(&id).and_then(|d| d.doc.clone())
    }

    /// A member's doc comment — a method of an interface node, or a
    /// field of a struct node — by name.
    pub fn member_doc(&self, node_id: u64, member: &str) -> Option<String> {
        self.docs
            .borrow()
            .get(&node_id)
            .and_then(|d| d.members.get(member).cloned())
    }

    /// Records that `interface_id`'s schema is loaded (so a caller can
    /// implement the "omit schema on re-send" half of the protocol).
    pub fn mark_loaded(&self, interface_id: u64) {
        self.loaded.borrow_mut().insert(interface_id);
    }

    pub fn is_loaded(&self, interface_id: u64) -> bool {
        self.loaded.borrow().contains(&interface_id) || self.loader.get_node(interface_id).is_some()
    }

    pub fn loader(&self) -> &SchemaLoader {
        &self.loader
    }

    pub fn struct_schema(&self, id: u64) -> Option<StructSchema> {
        self.loader.get_struct(id)
    }

    /// Introspects the interface `interface_id` (which must be loaded),
    /// returning its methods in ordinal order.
    pub fn interface(&self, interface_id: u64) -> Result<InterfaceInfo> {
        let node = self
            .loader
            .get_node(interface_id)
            .ok_or_else(|| anyhow!("interface {interface_id:#x} not loaded"))?;
        let node::Interface(iface) = node.which().map_err(|e| anyhow!("node which: {e}"))? else {
            bail!("node {interface_id:#x} is not an interface");
        };
        let methods = iface.get_methods().map_err(ce)?;
        let mut out = Vec::with_capacity(methods.len() as usize);
        for i in 0..methods.len() {
            let m = methods.get(i);
            let name = m
                .get_name()
                .map_err(ce)?
                .to_str()
                .map_err(|e| anyhow!("{e}"))?
                .to_string();
            let doc = self.member_doc(interface_id, &name);
            out.push(MethodInfo {
                name,
                ordinal: i as u16,
                param_type_id: m.get_param_struct_type(),
                result_type_id: m.get_result_struct_type(),
                doc,
            });
        }
        Ok(InterfaceInfo {
            id: interface_id,
            methods: out,
            doc: self.node_doc(interface_id),
        })
    }
}

/// Dispatches a method call against a capability whose interface is only
/// known at runtime. `args` must be a `Value::Struct` matching the
/// method's parameter struct; the result is the method's result struct
/// as a `Value::Struct`.
///
/// `hook` is borrowed (the call takes its own ref), so the caller keeps
/// ownership and can dispatch again.
pub async fn dynamic_call(
    hook: &dyn ClientHook,
    interface_id: u64,
    method: &MethodInfo,
    param_schema: StructSchema,
    result_schema: StructSchema,
    args: &Value,
) -> Result<Value> {
    let client = Client::new(hook.add_ref());
    let mut request = client.new_call::<any_pointer::Owned, any_pointer::Owned>(
        interface_id,
        method.ordinal,
        None,
    );
    {
        let params = request.get();
        let mut builder = params.init_dynamic(param_schema).map_err(ce)?;
        value_to_struct(&mut builder, args)?;
    }
    let response = request.send().promise.await.map_err(ce)?;
    let reader = response.get().map_err(ce)?;
    let dyn_reader = reader.get_dynamic(result_schema).map_err(ce)?;
    struct_to_value(dyn_reader)
}

// -- Value -> dynamic struct ------------------------------------------

/// Marshals a `Value::Struct` into a dynamic struct builder, by field
/// name. Fields absent from `value` are left at their defaults.
pub fn value_to_struct(builder: &mut dynamic_struct::Builder, value: &Value) -> Result<()> {
    let Value::Struct(fields) = value else {
        bail!("expected a struct value, got {value:?}");
    };
    for (name, v) in fields {
        set_field(builder, name, v)?;
    }
    Ok(())
}

fn set_field(builder: &mut dynamic_struct::Builder, name: &str, v: &Value) -> Result<()> {
    let field = builder.get_schema().get_field_by_name(name).map_err(ce)?;
    let ty = field.get_type();
    let ty_name = match ty.which() {
        TypeVariant::Void => "Void",
        TypeVariant::Bool => "Bool",
        TypeVariant::Int8 | TypeVariant::Int16 | TypeVariant::Int32 | TypeVariant::Int64 => "Int",
        TypeVariant::UInt8 | TypeVariant::UInt16 | TypeVariant::UInt32 | TypeVariant::UInt64 => {
            "UInt"
        }
        TypeVariant::Float32 | TypeVariant::Float64 => "Float",
        TypeVariant::Text => "Text",
        TypeVariant::Data => "Data",
        TypeVariant::Enum(_) => "Enum",
        TypeVariant::Struct(_) => "Struct",
        TypeVariant::List(_) => "List",
        TypeVariant::Capability => "Capability",
        TypeVariant::AnyPointer => "AnyPointer",
    };
    tracing::info!(field = %name, ty = %ty_name, "set_field: enter");
    match ty.which() {
        TypeVariant::Void => Ok(()),
        TypeVariant::Bool => set_scalar(
            builder,
            field,
            dynamic_value::Reader::Bool(matches!(v, Value::Bool(true))),
        ),
        TypeVariant::Int8 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::Int8(num_i64(v, name)? as i8),
        ),
        TypeVariant::Int16 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::Int16(num_i64(v, name)? as i16),
        ),
        TypeVariant::Int32 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::Int32(num_i64(v, name)? as i32),
        ),
        TypeVariant::Int64 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::Int64(num_i64(v, name)?),
        ),
        TypeVariant::UInt8 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::UInt8(num_u64(v, name)? as u8),
        ),
        TypeVariant::UInt16 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::UInt16(num_u64(v, name)? as u16),
        ),
        TypeVariant::UInt32 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::UInt32(num_u64(v, name)? as u32),
        ),
        TypeVariant::UInt64 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::UInt64(num_u64(v, name)?),
        ),
        TypeVariant::Float32 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::Float32(num_f64(v, name)? as f32),
        ),
        TypeVariant::Float64 => set_scalar(
            builder,
            field,
            dynamic_value::Reader::Float64(num_f64(v, name)?),
        ),
        TypeVariant::Text => {
            let s = v
                .as_str()
                .ok_or_else(|| anyhow!("field `{name}` expects text"))?;
            set_scalar(builder, field, dynamic_value::Reader::Text(s.into()))
        }
        TypeVariant::Data => {
            let Value::Data(d) = v else {
                bail!("field `{name}` expects data")
            };
            set_scalar(builder, field, dynamic_value::Reader::Data(d.as_slice()))
        }
        TypeVariant::Enum(es) => {
            let schema: capnp::schema::EnumSchema = es.into();
            let ev = resolve_enumerant(schema, v, name)?;
            set_scalar(builder, field, dynamic_value::Enum::new(ev, schema).into())
        }
        TypeVariant::Struct(_) => {
            let dynamic_value::Builder::Struct(mut sub) =
                builder.reborrow().init(field).map_err(ce)?
            else {
                bail!("field `{name}` is not a struct builder");
            };
            value_to_struct(&mut sub, v)
        }
        TypeVariant::List(_) => {
            let Value::List(items) = v else {
                bail!("field `{name}` expects a list")
            };
            let dynamic_value::Builder::List(mut list) = builder
                .reborrow()
                .initn(field, items.len() as u32)
                .map_err(ce)?
            else {
                bail!("field `{name}` is not a list builder");
            };
            value_to_list(&mut list, items)
        }
        TypeVariant::Capability => match v {
            // A cap-typed param: bind the live cap straight into the
            // pointer field via the vendored capnp's `set_capability`.
            Value::Cap(c) => {
                tracing::info!(
                    field = %name,
                    cap_iface_id = format!("{:#x}", c.interface_id()),
                    "set_field: writing capability"
                );
                builder.set_capability(field, c.add_ref()).map_err(ce)
            }
            // No cap to bind (unresolved cap-ref, or a method with an
            // optional cap arg): leave the field unset.
            _ => {
                tracing::warn!(
                    field = %name, value = ?v,
                    "set_field: Capability field but value not a Cap; leaving unset"
                );
                Ok(())
            }
        },
        TypeVariant::AnyPointer => match v {
            // capnp Rust's introspect layer reports schema-untyped
            // `:Capability` slots as AnyPointer (the TypeVariant
            // ::Capability arm is only used for typed-interface
            // fields). Bind via the any_pointer Builder's
            // set_as_capability so a Value::Cap routed into an
            // untyped cap slot (e.g. OpaqueCap.cap, restorable.cap)
            // actually lands in the message's cap table.
            Value::Cap(c) => {
                tracing::info!(
                    field = %name,
                    cap_iface_id = format!("{:#x}", c.interface_id()),
                    "set_field: writing capability via any_pointer"
                );
                let dynamic_value::Builder::AnyPointer(mut ap) =
                    builder.reborrow().init(field).map_err(ce)?
                else {
                    bail!("field `{name}`: init did not return an AnyPointer builder");
                };
                ap.set_as_capability(c.add_ref());
                Ok(())
            }
            // No schema to interpret a non-cap AnyPointer param;
            // leave unset so a method with an optional any-pointer
            // arg still dispatches.
            _ => Ok(()),
        },
    }
}

fn set_scalar(
    builder: &mut dynamic_struct::Builder,
    field: capnp::schema::Field,
    reader: dynamic_value::Reader,
) -> Result<()> {
    builder.set(field, reader).map_err(ce)
}

fn value_to_list(list: &mut dynamic_list::Builder, items: &[Value]) -> Result<()> {
    let et = list.element_type();
    for (i, v) in items.iter().enumerate() {
        let i = i as u32;
        match et.which() {
            TypeVariant::Bool => list
                .set(
                    i,
                    dynamic_value::Reader::Bool(matches!(v, Value::Bool(true))),
                )
                .map_err(ce)?,
            TypeVariant::Int8 => list
                .set(i, dynamic_value::Reader::Int8(num_i64(v, "list")? as i8))
                .map_err(ce)?,
            TypeVariant::Int16 => list
                .set(i, dynamic_value::Reader::Int16(num_i64(v, "list")? as i16))
                .map_err(ce)?,
            TypeVariant::Int32 => list
                .set(i, dynamic_value::Reader::Int32(num_i64(v, "list")? as i32))
                .map_err(ce)?,
            TypeVariant::Int64 => list
                .set(i, dynamic_value::Reader::Int64(num_i64(v, "list")?))
                .map_err(ce)?,
            TypeVariant::UInt8 => list
                .set(i, dynamic_value::Reader::UInt8(num_u64(v, "list")? as u8))
                .map_err(ce)?,
            TypeVariant::UInt16 => list
                .set(i, dynamic_value::Reader::UInt16(num_u64(v, "list")? as u16))
                .map_err(ce)?,
            TypeVariant::UInt32 => list
                .set(i, dynamic_value::Reader::UInt32(num_u64(v, "list")? as u32))
                .map_err(ce)?,
            TypeVariant::UInt64 => list
                .set(i, dynamic_value::Reader::UInt64(num_u64(v, "list")?))
                .map_err(ce)?,
            TypeVariant::Float32 => list
                .set(
                    i,
                    dynamic_value::Reader::Float32(num_f64(v, "list")? as f32),
                )
                .map_err(ce)?,
            TypeVariant::Float64 => list
                .set(i, dynamic_value::Reader::Float64(num_f64(v, "list")?))
                .map_err(ce)?,
            TypeVariant::Text => {
                let s = v
                    .as_str()
                    .ok_or_else(|| anyhow!("list element expects text"))?;
                list.set(i, dynamic_value::Reader::Text(s.into()))
                    .map_err(ce)?
            }
            TypeVariant::Struct(_) => {
                let dynamic_value::Builder::Struct(mut sub) = list.reborrow().get(i).map_err(ce)?
                else {
                    bail!("list element is not a struct");
                };
                value_to_struct(&mut sub, v)?;
            }
            TypeVariant::List(_) => {
                let Value::List(sub_items) = v else {
                    bail!("nested list expects a list")
                };
                let dynamic_value::Builder::List(mut sub) = list
                    .reborrow()
                    .init(i, sub_items.len() as u32)
                    .map_err(ce)?
                else {
                    bail!("nested list builder");
                };
                value_to_list(&mut sub, sub_items)?;
            }
            _ => bail!("unsupported list element type"),
        }
    }
    Ok(())
}

fn resolve_enumerant(schema: capnp::schema::EnumSchema, v: &Value, name: &str) -> Result<u16> {
    let want = match v {
        Value::Enum(s) | Value::Text(s) => s.clone(),
        Value::UInt(n) => return Ok(*n as u16),
        Value::Int(n) => return Ok(*n as u16),
        _ => bail!("field `{name}` expects an enum name"),
    };
    let enumerants = schema.get_enumerants().map_err(ce)?;
    for i in 0..enumerants.len() {
        let e = enumerants.get(i);
        if e.get_proto()
            .get_name()
            .map_err(ce)?
            .to_str()
            .map_err(|e| anyhow!("{e}"))?
            == want
        {
            return Ok(e.get_ordinal());
        }
    }
    bail!("no enumerant `{want}` in enum for field `{name}`")
}

fn num_i64(v: &Value, name: &str) -> Result<i64> {
    v.as_i64()
        .ok_or_else(|| anyhow!("field `{name}` expects a signed integer, got {v:?}"))
}
fn num_u64(v: &Value, name: &str) -> Result<u64> {
    v.as_u64()
        .ok_or_else(|| anyhow!("field `{name}` expects an unsigned integer, got {v:?}"))
}
fn num_f64(v: &Value, name: &str) -> Result<f64> {
    v.as_f64()
        .ok_or_else(|| anyhow!("field `{name}` expects a number, got {v:?}"))
}

// -- dynamic struct -> Value ------------------------------------------

/// Reads a dynamic struct into a `Value::Struct`, including set
/// non-union fields and the active union arm (if any).
pub fn struct_to_value(reader: dynamic_struct::Reader) -> Result<Value> {
    let schema = reader.get_schema();
    let mut out: Vec<(String, Value)> = Vec::new();

    let non_union = schema.get_non_union_fields().map_err(ce)?;
    for i in 0..non_union.len() {
        let field = non_union.get(i);
        if reader.has(field).map_err(ce)? {
            out.push((field_name(field)?, field_to_value(reader, field)?));
        }
    }
    if let Some(active) = reader.which().map_err(ce)? {
        if reader.has(active).map_err(ce)? {
            out.push((field_name(active)?, field_to_value(reader, active)?));
        }
    }
    Ok(Value::Struct(out))
}

/// Reads one field of a struct into a `Value`. Capability fields can't
/// go through the dynamic-value path (it yields an opaque placeholder),
/// so they're pulled out as a live `Value::Cap` carrying the hook and
/// the field's declared interface id.
///
/// Untyped `:Capability` slots (notably `OpaqueCap.cap`) come through
/// the introspect layer as `TypeVariant::AnyPointer`, so we also try
/// reading those as caps — on success this is the symmetric read-side
/// counterpart to `set_field`'s AnyPointer→Capability write path.
fn field_to_value(reader: dynamic_struct::Reader, field: capnp::schema::Field) -> Result<Value> {
    match field.get_type().which() {
        TypeVariant::Capability => {
            let hook = reader.get_capability(field).map_err(ce)?;
            Ok(Value::Cap(CapRef::new(
                hook,
                field_interface_id(field).unwrap_or(0),
            )))
        }
        TypeVariant::AnyPointer => {
            // The field MIGHT be a `:Capability` slot (untyped cap) or
            // a true `:AnyPointer`. Try the cap read; if the pointer
            // doesn't hold a capability we surface Null.
            match reader.get_capability(field) {
                Ok(hook) => Ok(Value::Cap(CapRef::new(hook, 0))),
                Err(_) => Ok(Value::Null),
            }
        }
        _ => reader_to_value(reader.get(field).map_err(ce)?),
    }
}

/// The interface id a capability field declares in the schema, or `None`
/// for a bare `Capability` (e.g. `OpaqueCap.cap`).
fn field_interface_id(field: capnp::schema::Field) -> Option<u64> {
    use capnp::schema_capnp::{field as f, type_};
    let f::Slot(slot) = field.get_proto().which().ok()? else {
        return None;
    };
    if let type_::Interface(i) = slot.get_type().ok()?.which().ok()? {
        let id = i.get_type_id();
        if id != 0 {
            return Some(id);
        }
    }
    None
}

fn field_name(field: capnp::schema::Field) -> Result<String> {
    Ok(field
        .get_proto()
        .get_name()
        .map_err(ce)?
        .to_str()
        .map_err(|e| anyhow!("{e}"))?
        .to_string())
}

fn reader_to_value(v: dynamic_value::Reader) -> Result<Value> {
    Ok(match v {
        dynamic_value::Reader::Void => Value::Null,
        dynamic_value::Reader::Bool(b) => Value::Bool(b),
        dynamic_value::Reader::Int8(x) => Value::Int(x as i64),
        dynamic_value::Reader::Int16(x) => Value::Int(x as i64),
        dynamic_value::Reader::Int32(x) => Value::Int(x as i64),
        dynamic_value::Reader::Int64(x) => Value::Int(x),
        dynamic_value::Reader::UInt8(x) => Value::UInt(x as u64),
        dynamic_value::Reader::UInt16(x) => Value::UInt(x as u64),
        dynamic_value::Reader::UInt32(x) => Value::UInt(x as u64),
        dynamic_value::Reader::UInt64(x) => Value::UInt(x),
        dynamic_value::Reader::Float32(x) => Value::Float(x as f64),
        dynamic_value::Reader::Float64(x) => Value::Float(x),
        dynamic_value::Reader::Text(t) => {
            Value::Text(t.to_str().map_err(|e| anyhow!("{e}"))?.to_string())
        }
        dynamic_value::Reader::Data(d) => Value::Data(d.to_vec()),
        dynamic_value::Reader::Enum(e) => match e.get_enumerant().map_err(ce)? {
            Some(en) => Value::Enum(field_name_enum(en)?),
            None => Value::UInt(e.get_value() as u64),
        },
        dynamic_value::Reader::Struct(s) => struct_to_value(s)?,
        dynamic_value::Reader::List(l) => {
            let mut items = Vec::with_capacity(l.len() as usize);
            for el in l.iter() {
                items.push(reader_to_value(el.map_err(ce)?)?);
            }
            Value::List(items)
        }
        // capnp-rust 0.20 has no dynamic capability extraction, and
        // AnyPointer has no schema to interpret it — surface as Null.
        dynamic_value::Reader::AnyPointer(_) | dynamic_value::Reader::Capability(_) => Value::Null,
    })
}

fn field_name_enum(en: capnp::schema::Enumerant) -> Result<String> {
    Ok(en
        .get_proto()
        .get_name()
        .map_err(ce)?
        .to_str()
        .map_err(|e| anyhow!("{e}"))?
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use capnp::capability::{FromClientHook, Promise};
    use darude_wire::web_fetch_v1_capnp::web_fetch_v1;

    #[test]
    fn docs_ride_the_injected_schema_bytes() {
        // wire's build step injects source-extracted docs into each
        // CGR's sourceInfo; loading those bytes must surface them on
        // InterfaceInfo/MethodInfo — the whole join is by name, no
        // positional coupling.
        let (id, bytes) = darude_wire::compiled_schema_for_interface("self.v1").unwrap();
        let cache = SchemaCache::new();
        cache.load(bytes).unwrap();
        let iface = cache.interface(id).unwrap();
        let held = iface.method("held").expect("held");
        assert!(
            held.doc
                .as_deref()
                .is_some_and(|d| d.contains("authority surface")),
            "held doc: {:?}",
            held.doc
        );
        // Struct docs and field docs come through node_doc/member_doc
        // on the struct's own node id: Self.Entry's block sits above
        // `struct Entry`, and its `cap` field has its own adjacent
        // block in self-v1.capnp.
        let entry_id = cache
            .loader()
            .get_node(id)
            .and_then(|n| n.get_nested_nodes().ok())
            .and_then(|nested| {
                nested.iter().find_map(|nn| {
                    (nn.get_name().ok()?.to_str().ok()? == "Entry").then(|| nn.get_id())
                })
            })
            .expect("Entry nested under Self");
        assert!(
            cache
                .node_doc(entry_id)
                .is_some_and(|d| d.contains("One held cap")),
            "Entry doc: {:?}",
            cache.node_doc(entry_id)
        );
        assert!(
            cache
                .member_doc(entry_id, "cap")
                .is_some_and(|d| d.contains("LIVE cap")),
            "Entry.cap doc: {:?}",
            cache.member_doc(entry_id, "cap")
        );
    }

    // A real web.fetch.v1 server; the dynamic call must route to it by
    // interface id + method ordinal and marshal params/results purely
    // from the loaded schema.
    struct StubWeb;
    impl web_fetch_v1::Server for StubWeb {
        fn fetch(
            &mut self,
            params: web_fetch_v1::FetchParams,
            mut results: web_fetch_v1::FetchResults,
        ) -> Promise<(), capnp::Error> {
            let req = capnp_rpc::pry!(capnp_rpc::pry!(params.get()).get_req());
            let url = capnp_rpc::pry!(req.get_url())
                .to_str()
                .unwrap_or("")
                .to_string();
            let max = req.get_max_bytes();
            let mut resp = results.get().init_resp();
            resp.set_status(200);
            resp.set_content_type("text/plain");
            resp.set_body(format!("fetched {url} (max={max})"));
            resp.set_error("");
            resp.set_truncated(false);
            Promise::ok(())
        }
    }

    #[test]
    fn dynamic_dispatch_against_real_server() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            // Load web.fetch.v1's schema purely from the wire constant —
            // no compile-time interface binding from here on.
            let (iface_id, schema_bytes) =
                darude_wire::schema_for_interface("web.fetch.v1").expect("known interface");
            let cache = SchemaCache::new();
            cache.load(schema_bytes).unwrap();

            let info = cache.interface(iface_id).unwrap();
            let fetch = info.method("fetch").expect("fetch method");
            let param_schema = cache.struct_schema(fetch.param_type_id).unwrap();
            let result_schema = cache.struct_schema(fetch.result_type_id).unwrap();

            // The cap, as an opaque client hook.
            let client: web_fetch_v1::Client = capnp_rpc::new_client(StubWeb);
            let hook = client.into_client_hook();

            // Params struct is `fetch$Params { req: FetchRequest }`.
            let args = Value::struct_of([(
                "req".to_string(),
                Value::struct_of([
                    ("url".to_string(), Value::Text("http://example.com".into())),
                    ("maxBytes".to_string(), Value::UInt(1024)),
                ]),
            )]);

            let result = dynamic_call(&*hook, iface_id, fetch, param_schema, result_schema, &args)
                .await
                .unwrap();

            // Results struct is `fetch$Results { resp: FetchResponse }`.
            let resp = result.get("resp").expect("resp field");
            assert_eq!(resp.get("status"), Some(&Value::UInt(200)));
            assert_eq!(
                resp.get("body").and_then(Value::as_str),
                Some("fetched http://example.com (max=1024)")
            );
            assert_eq!(resp.get("truncated"), Some(&Value::Bool(false)));
        });
    }
}
