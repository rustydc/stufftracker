//! Runtime schema loading — the Rust analogue of C++ `capnp::SchemaLoader`.
//!
//! The generated-code path bakes a `RawStructSchema` (encoded node +
//! member-index tables) and a `field_types: fn(u16) -> Type` function
//! pointer into static memory for every struct. That works for schemas
//! known at compile time, but a process that receives a `schema.capnp`
//! `CodeGeneratorRequest` (or bare `Node`s) over the wire — e.g. the
//! `OpaqueCap` introspection protocol — has no such statics.
//!
//! `SchemaLoader` builds the same `RawStructSchema` / `RawEnumSchema`
//! structures at runtime by copying each node into a leaked single-
//! segment message (giving the `&'static [Word]` the rest of the
//! introspection machinery expects) and computing the member tables
//! from the node. The one thing a bare `fn` pointer can't do is close
//! over runtime data, so the per-struct `field_types` resolver is held
//! in a process-global table keyed by the struct's leaked
//! `&'static RawStructSchema` pointer; `schema::Field::get_type`
//! consults that table before falling back to the static function
//! pointer. Loaded schemas are intentionally permanent (leaked), so the
//! resolver table never needs to evict.
//!
//! Once loaded, a `schema::StructSchema` / `EnumSchema` produced here is
//! indistinguishable from a generated one: `dynamic_struct`,
//! `dynamic_list`, and `dynamic_value` all drive off `Field::get_type`
//! and the member tables, so they work unchanged. Pair this with
//! `any_pointer::Builder::init_dynamic` / `Reader::get_dynamic` and the
//! low-level `capability::Client::new_call` to construct and dispatch
//! calls against an interface whose definition only arrived at runtime.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::introspect::{
    RawBrandedStructSchema, RawEnumSchema, RawStructSchema, Type, TypeVariant,
};
use crate::schema::{EnumSchema, StructSchema};
use crate::schema_capnp::{code_generator_request, field, node, type_};
use crate::{message, Result, Word};

// -- Process-global runtime field-type resolver table ------------------
//
// `schema::Field::get_type` calls `runtime_field_type` before invoking
// the static `field_types` function pointer. For generated schemas the
// lookup misses (their generic pointer was never registered) and the
// fast-path atomic keeps the cost to a single relaxed load until the
// first runtime schema is loaded.

type Resolver = Arc<dyn Fn(u16) -> Type + Send + Sync>;

static PRESENT: AtomicBool = AtomicBool::new(false);
static REGISTRY: OnceLock<RwLock<HashMap<usize, Resolver>>> = OnceLock::new();

fn registry() -> &'static RwLock<HashMap<usize, Resolver>> {
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Looks up the runtime field-type resolver for a struct, if one was
/// registered by a `SchemaLoader`. Called from `schema::Field::get_type`.
pub(crate) fn runtime_field_type(generic: &'static RawStructSchema, index: u16) -> Option<Type> {
    if !PRESENT.load(Ordering::Relaxed) {
        return None;
    }
    let key = generic as *const RawStructSchema as usize;
    let resolver = {
        let reg = registry().read().ok()?;
        reg.get(&key)?.clone()
    };
    Some(resolver(index))
}

fn register_resolver(generic: &'static RawStructSchema, resolver: Resolver) {
    let key = generic as *const RawStructSchema as usize;
    registry()
        .write()
        .expect("schema_loader registry poisoned")
        .insert(key, resolver);
    PRESENT.store(true, Ordering::Relaxed);
}

// Dummy function pointers stored in the runtime `RawStructSchema` /
// `RawBrandedStructSchema`. They are never invoked: `Field::get_type`
// routes runtime structs through `runtime_field_type`, and the runtime
// schemas we build carry no annotations.
fn dummy_field_types(_: u16) -> Type {
    TypeVariant::Void.into()
}
fn dummy_annotation_types(_: Option<u16>, _: u32) -> Type {
    TypeVariant::Void.into()
}

// -- Leaking helpers ---------------------------------------------------

fn leak_struct(s: RawStructSchema) -> &'static RawStructSchema {
    Box::leak(Box::new(s))
}

/// Copies `node` into its own single-segment canonical message and leaks
/// it, yielding the `&'static [Word]` that `StructSchema`/`EnumSchema`
/// read their proto from. Canonicalization guarantees a single segment
/// whose first word is the root pointer — exactly the layout
/// `PointerReader::get_root_unchecked` expects.
fn encode_node(node: node::Reader) -> Result<&'static [Word]> {
    let mut msg = message::Builder::new_default();
    msg.set_root(node)?;
    let words = msg.into_reader().canonicalize()?;
    Ok(Vec::leak(words))
}

/// Reconstructs a node reader from a leaked encoded node. Mirrors what
/// `StructSchema::new` does internally; used by the field-type resolver
/// so it can capture a plain `&'static [Word]` (which is `Send + Sync`)
/// rather than a non-`Send` reader.
fn read_node(encoded: &'static [Word]) -> node::Reader<'static> {
    crate::any_pointer::Reader::new(unsafe {
        crate::private::layout::PointerReader::get_root_unchecked(encoded.as_ptr() as *const u8)
    })
    .get_as()
    .expect("leaked encoded node is a valid Node")
}

// -- Loaded entries ----------------------------------------------------

#[derive(Clone, Copy)]
struct StructEntry {
    branded: RawBrandedStructSchema,
}

struct Inner {
    // All nodes by id, including interfaces/consts — used for method and
    // generic node lookups.
    nodes: RwLock<HashMap<u64, &'static [Word]>>,
    structs: RwLock<HashMap<u64, StructEntry>>,
    enums: RwLock<HashMap<u64, RawEnumSchema>>,
}

impl Inner {
    fn struct_schema(&self, id: u64) -> Option<StructSchema> {
        self.structs
            .read()
            .ok()?
            .get(&id)
            .map(|e| StructSchema::from(e.branded))
    }

    fn enum_schema(&self, id: u64) -> Option<EnumSchema> {
        self.enums.read().ok()?.get(&id).map(|&r| EnumSchema::from(r))
    }

    // Resolve a node-id reference to an introspect::Type. Unknown ids
    // degrade to AnyPointer rather than panicking — a well-formed CGR
    // carries the transitive closure, so this only bites on truncated
    // input, where AnyPointer is the safe inert choice.
    fn type_of_node(&self, id: u64) -> Type {
        if let Some(e) = self.structs.read().ok().and_then(|m| m.get(&id).copied()) {
            return TypeVariant::Struct(e.branded).into();
        }
        if let Some(r) = self.enums.read().ok().and_then(|m| m.get(&id).copied()) {
            return TypeVariant::Enum(r).into();
        }
        TypeVariant::AnyPointer.into()
    }

    fn type_of(&self, t: type_::Reader) -> Type {
        match t.which() {
            Ok(type_::Void(())) => TypeVariant::Void.into(),
            Ok(type_::Bool(())) => TypeVariant::Bool.into(),
            Ok(type_::Int8(())) => TypeVariant::Int8.into(),
            Ok(type_::Int16(())) => TypeVariant::Int16.into(),
            Ok(type_::Int32(())) => TypeVariant::Int32.into(),
            Ok(type_::Int64(())) => TypeVariant::Int64.into(),
            Ok(type_::Uint8(())) => TypeVariant::UInt8.into(),
            Ok(type_::Uint16(())) => TypeVariant::UInt16.into(),
            Ok(type_::Uint32(())) => TypeVariant::UInt32.into(),
            Ok(type_::Uint64(())) => TypeVariant::UInt64.into(),
            Ok(type_::Float32(())) => TypeVariant::Float32.into(),
            Ok(type_::Float64(())) => TypeVariant::Float64.into(),
            Ok(type_::Text(())) => TypeVariant::Text.into(),
            Ok(type_::Data(())) => TypeVariant::Data.into(),
            Ok(type_::List(l)) => match l.get_element_type() {
                Ok(el) => Type::list_of(self.type_of(el)),
                Err(_) => TypeVariant::AnyPointer.into(),
            },
            Ok(type_::Enum(e)) => self.type_of_node(e.get_type_id()),
            Ok(type_::Struct(s)) => self.type_of_node(s.get_type_id()),
            Ok(type_::Interface(_)) => TypeVariant::Capability.into(),
            Ok(type_::AnyPointer(_)) => TypeVariant::AnyPointer.into(),
            Err(_) => TypeVariant::AnyPointer.into(),
        }
    }
}

/// Resolves a struct field's `introspect::Type` from the raw node.
/// `Void` is returned for malformed input (rather than panicking inside
/// `Field::get_type`, which has no error channel).
fn resolve_field_type(inner: &Inner, node: node::Reader, index: u16) -> Type {
    let Ok(node::Struct(st)) = node.which() else {
        return TypeVariant::Void.into();
    };
    let Ok(fields) = st.get_fields() else {
        return TypeVariant::Void.into();
    };
    if u32::from(index) >= fields.len() {
        return TypeVariant::Void.into();
    }
    let f = fields.get(u32::from(index));
    match f.which() {
        Ok(field::Slot(slot)) => match slot.get_type() {
            Ok(t) => inner.type_of(t),
            Err(_) => TypeVariant::Void.into(),
        },
        Ok(field::Group(g)) => inner.type_of_node(g.get_type_id()),
        Err(_) => TypeVariant::Void.into(),
    }
}

// -- Public API --------------------------------------------------------

/// Loads Cap'n Proto schema nodes at runtime and hands back
/// `schema::StructSchema` / `EnumSchema` usable with the dynamic API.
///
/// Cloning a `SchemaLoader` yields another handle onto the same loaded
/// set (it is internally `Arc`-shared).
#[derive(Clone)]
pub struct SchemaLoader {
    inner: Arc<Inner>,
}

impl Default for SchemaLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaLoader {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                nodes: RwLock::new(HashMap::new()),
                structs: RwLock::new(HashMap::new()),
                enums: RwLock::new(HashMap::new()),
            }),
        }
    }

    /// Loads every node in a serialized `CodeGeneratorRequest` (the
    /// `OpaqueCap.schema` payload), as produced by `capnp compile -o-`
    /// — a standard framed capnp stream message.
    pub fn load_serialized(&self, bytes: &[u8]) -> Result<()> {
        let reader = crate::serialize::read_message(bytes, Default::default())?;
        let cgr: code_generator_request::Reader = reader.get_root()?;
        self.load_code_generator_request(cgr)
    }

    /// Loads every node in an already-parsed `CodeGeneratorRequest`.
    pub fn load_code_generator_request(
        &self,
        cgr: code_generator_request::Reader,
    ) -> Result<()> {
        for n in cgr.get_nodes()? {
            self.load_node(n)?;
        }
        Ok(())
    }

    /// Loads a single node. Idempotent per id (re-loading the same id is
    /// a no-op, matching C++ `SchemaLoader::loadOnce`).
    pub fn load_node(&self, node: node::Reader) -> Result<()> {
        let id = node.get_id();
        if self.inner.nodes.read().unwrap().contains_key(&id) {
            return Ok(());
        }
        let encoded = encode_node(node)?;
        self.inner.nodes.write().unwrap().insert(id, encoded);

        match node.which() {
            Ok(node::Struct(st)) => {
                let raw = leak_struct(RawStructSchema {
                    encoded_node: encoded,
                    nonunion_members: build_nonunion_members(st)?,
                    members_by_discriminant: build_members_by_discriminant(st)?,
                    members_by_name: build_members_by_name(st)?,
                });
                let branded = RawBrandedStructSchema {
                    generic: raw,
                    field_types: dummy_field_types,
                    annotation_types: dummy_annotation_types,
                };
                self.inner
                    .structs
                    .write()
                    .unwrap()
                    .insert(id, StructEntry { branded });

                // Register the runtime field-type resolver. Capturing the
                // Arc<Inner> + the &'static encoded node (both Send+Sync)
                // keeps the closure Send+Sync; the node table is fully
                // populated by the time any resolver runs (lazily, from
                // Field::get_type).
                let inner = self.inner.clone();
                let resolver: Resolver = Arc::new(move |index| {
                    resolve_field_type(&inner, read_node(encoded), index)
                });
                register_resolver(raw, resolver);
            }
            Ok(node::Enum(_)) => {
                let raw = RawEnumSchema {
                    encoded_node: encoded,
                    annotation_types: dummy_annotation_types,
                };
                self.inner.enums.write().unwrap().insert(id, raw);
            }
            // Interfaces, consts, annotations, files: recorded in `nodes`
            // (so `get_node` / method lookup works) but carry no struct
            // or enum schema of their own.
            _ => {}
        }
        Ok(())
    }

    /// The loaded struct schema for `id`, if any.
    pub fn get_struct(&self, id: u64) -> Option<StructSchema> {
        self.inner.struct_schema(id)
    }

    /// The loaded enum schema for `id`, if any.
    pub fn get_enum(&self, id: u64) -> Option<EnumSchema> {
        self.inner.enum_schema(id)
    }

    /// The raw node proto for `id`, if loaded. Use this to read interface
    /// methods (`node.which()? -> Interface; iface.get_methods()`), which
    /// have no dedicated wrapper type in this crate.
    pub fn get_node(&self, id: u64) -> Option<node::Reader<'static>> {
        let encoded = *self.inner.nodes.read().ok()?.get(&id)?;
        Some(read_node(encoded))
    }

    /// The `introspect::Type` naming node `id` (struct, enum, or — for an
    /// interface — `Capability`). `None` if not loaded.
    pub fn get_type(&self, id: u64) -> Option<Type> {
        if let Some(e) = self.inner.structs.read().unwrap().get(&id).copied() {
            return Some(TypeVariant::Struct(e.branded).into());
        }
        if let Some(r) = self.inner.enums.read().unwrap().get(&id).copied() {
            return Some(TypeVariant::Enum(r).into());
        }
        match self.get_node(id)?.which() {
            Ok(node::Interface(_)) => Some(TypeVariant::Capability.into()),
            _ => None,
        }
    }
}

// -- Member-table construction -----------------------------------------

fn fields_of<'a>(
    st: node::struct_::Reader<'a>,
) -> Result<crate::struct_list::Reader<'a, field::Owned>> {
    st.get_fields()
}

fn build_nonunion_members(st: node::struct_::Reader) -> Result<&'static [u16]> {
    let fields = fields_of(st)?;
    let mut out: Vec<u16> = Vec::new();
    for i in 0..fields.len() {
        if fields.get(i).get_discriminant_value() == field::NO_DISCRIMINANT {
            out.push(i as u16);
        }
    }
    Ok(Vec::leak(out))
}

fn build_members_by_discriminant(st: node::struct_::Reader) -> Result<&'static [u16]> {
    let fields = fields_of(st)?;
    let count = st.get_discriminant_count() as usize;
    let mut out: Vec<u16> = alloc::vec![0u16; count];
    for i in 0..fields.len() {
        let d = fields.get(i).get_discriminant_value();
        if d != field::NO_DISCRIMINANT && (d as usize) < count {
            out[d as usize] = i as u16;
        }
    }
    Ok(Vec::leak(out))
}

fn build_members_by_name(st: node::struct_::Reader) -> Result<&'static [u16]> {
    let fields = fields_of(st)?;
    let mut pairs: Vec<(String, u16)> = Vec::new();
    for i in 0..fields.len() {
        let name = fields.get(i).get_name()?.to_str()?.to_string();
        pairs.push((name, i as u16));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Vec::leak(pairs.into_iter().map(|(_, i)| i).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dynamic_value;

    const FIXTURE: &[u8] = include_bytes!("../tests/schemas/loader_test.cgr.bin");

    // The fixture is a CodeGeneratorRequest; node ids are content-derived,
    // so we look them up by the trailing component of their display name
    // ("...loader_test.capnp:Sample") rather than hard-coding.
    fn id_by_name(loader_bytes: &[u8], suffix: &str) -> u64 {
        let reader = crate::serialize::read_message(loader_bytes, Default::default()).unwrap();
        let cgr: code_generator_request::Reader = reader.get_root().unwrap();
        for n in cgr.get_nodes().unwrap() {
            let dn = n.get_display_name().unwrap().to_str().unwrap();
            if dn.ends_with(suffix) {
                return n.get_id();
            }
        }
        panic!("no node ending in {suffix}");
    }

    #[test]
    fn loads_and_round_trips_a_struct() {
        let loader = SchemaLoader::new();
        loader.load_serialized(FIXTURE).unwrap();

        let sample_id = id_by_name(FIXTURE, ":Sample");
        let schema = loader.get_struct(sample_id).expect("Sample loaded");

        // Field lookup by name + type resolution come from the runtime
        // resolver (the static field_types fn is a never-called dummy).
        let name_field = schema.get_field_by_name("name").unwrap();
        assert!(matches!(name_field.get_type().which(), TypeVariant::Text));
        let inner_field = schema.get_field_by_name("inner").unwrap();
        assert!(matches!(inner_field.get_type().which(), TypeVariant::Struct(_)));
        let tags_field = schema.get_field_by_name("tags").unwrap();
        assert!(matches!(tags_field.get_type().which(), TypeVariant::List(_)));
        let color_field = schema.get_field_by_name("color").unwrap();
        assert!(matches!(color_field.get_type().which(), TypeVariant::Enum(_)));

        // Build a message dynamically, then read it back dynamically.
        let mut msg = message::Builder::new_default();
        {
            let root = msg.init_root::<crate::any_pointer::Builder>();
            let mut b = root.init_dynamic(schema).unwrap();
            b.set_named("flag", dynamic_value::Reader::Bool(true)).unwrap();
            b.set_named("i32v", dynamic_value::Reader::Int32(-7)).unwrap();
            b.set_named("u64v", dynamic_value::Reader::UInt64(42)).unwrap();
            b.set_named("name", dynamic_value::Reader::Text("hello".into()))
                .unwrap();
            // Nested struct.
            let dynamic_value::Builder::Struct(mut inner) =
                b.reborrow().init_named("inner").unwrap()
            else {
                panic!("inner is a struct");
            };
            inner
                .set_named("label", dynamic_value::Reader::Text("lbl".into()))
                .unwrap();
            inner.set_named("count", dynamic_value::Reader::UInt32(3)).unwrap();
            // List of text.
            let dynamic_value::Builder::List(mut tags) =
                b.reborrow().initn_named("tags", 2).unwrap()
            else {
                panic!("tags is a list");
            };
            tags.set(0, dynamic_value::Reader::Text("a".into())).unwrap();
            tags.set(1, dynamic_value::Reader::Text("b".into())).unwrap();
            // Union arm.
            b.set_named("asText", dynamic_value::Reader::Text("u".into()))
                .unwrap();
        }

        let root = msg
            .get_root_as_reader::<crate::any_pointer::Reader>()
            .unwrap();
        let r = root.get_dynamic(schema).unwrap();
        assert!(matches!(
            r.get_named("flag").unwrap(),
            dynamic_value::Reader::Bool(true)
        ));
        assert!(matches!(
            r.get_named("i32v").unwrap(),
            dynamic_value::Reader::Int32(-7)
        ));
        let dynamic_value::Reader::Text(name) = r.get_named("name").unwrap() else {
            panic!("name is text");
        };
        assert_eq!(name.to_str().unwrap(), "hello");

        let dynamic_value::Reader::Struct(inner) = r.get_named("inner").unwrap() else {
            panic!("inner is struct");
        };
        let dynamic_value::Reader::Text(lbl) = inner.get_named("label").unwrap() else {
            panic!("label is text");
        };
        assert_eq!(lbl.to_str().unwrap(), "lbl");

        let dynamic_value::Reader::List(tags) = r.get_named("tags").unwrap() else {
            panic!("tags is list");
        };
        assert_eq!(tags.len(), 2);

        // Active union field reads back as the arm we set.
        let active = r.which().unwrap().expect("union has an active field");
        assert_eq!(active.get_proto().get_name().unwrap().to_str().unwrap(), "asText");
    }

    #[test]
    fn exposes_interface_methods() {
        let loader = SchemaLoader::new();
        loader.load_serialized(FIXTURE).unwrap();
        let echo_id = id_by_name(FIXTURE, ":Echo");
        let node = loader.get_node(echo_id).expect("Echo loaded");
        let node::Interface(iface) = node.which().unwrap() else {
            panic!("Echo is an interface");
        };
        let methods = iface.get_methods().unwrap();
        assert_eq!(methods.len(), 1);
        let ping = methods.get(0);
        assert_eq!(ping.get_name().unwrap().to_str().unwrap(), "ping");
        // Param/result struct types resolve to loaded Sample schemas.
        assert!(loader.get_struct(ping.get_param_struct_type()).is_some());
        assert!(loader.get_struct(ping.get_result_struct_type()).is_some());
        assert!(matches!(
            loader.get_type(echo_id),
            Some(t) if matches!(t.which(), TypeVariant::Capability)
        ));
    }
}
