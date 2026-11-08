// schema-docs — make the schema bytes carry their docs.
//
// capnp's own docComment channel (Node.SourceInfo in the CGR) attaches
// comments that FOLLOW a declaration; this repo's schemas write comment
// blocks BEFORE each declaration, which capnp attributes to the
// preceding member (an off-by-one) or — with the house style's blank
// line above each block — drops entirely. Verified empirically; see
// docs/capscript.md's editor section.
//
// So the schema SOURCE is the doc authority. `extract` reads the
// comment block directly above each interface, method, struct, and
// field declaration (strict adjacency: a blank or other non-comment
// line detaches); `inject` rebuilds a compiled CGR's sourceInfo from
// those rows — node docs by display_name, member docs joined BY NAME
// against the node's own method/field lists (never by source order, so
// ordinal renumbering can't misattribute). After injection the CGR is
// self-describing: every consumer of the bytes (`SchemaCache`, the LLM
// tool lowering, capscript's editor) reads one standard channel, and
// the docs travel wherever the schema does.

use capnp::message::ReaderOptions;
use capnp::schema_capnp::{code_generator_request, node};
use capnp::serialize;
use std::collections::HashMap;

/// One extracted doc: (scope key, member name, text). The scope key
/// matches the CGR node display_name ("<file>:<Path.To.Decl>"); member
/// "" means the doc is the declaration's own.
pub type DocRow = (String, String, String);

/// Extract doc rows from one schema source. `file` must be the path
/// capnp will use in display_names — i.e. the path relative to the
/// compile's src-prefix (for the platform's schemas/ dir, the bare
/// file name).
pub fn extract(file: &str, src: &str, rows: &mut Vec<DocRow>) {
    // (scope name, brace depth at which it opened)
    let mut stack: Vec<(String, usize)> = Vec::new();
    let mut depth = 0usize;
    let mut block: Vec<String> = Vec::new();
    for raw in src.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix('#') {
            block.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
            continue;
        }
        if let Some(name) = scope_decl_name(line) {
            let path: Vec<&str> = stack
                .iter()
                .map(|(n, _)| n.as_str())
                .chain([name.as_str()])
                .collect();
            if !block.is_empty() {
                rows.push((
                    format!("{file}:{}", path.join(".")),
                    String::new(),
                    block.join("\n"),
                ));
            }
            stack.push((name, depth));
        } else if !stack.is_empty() {
            if let Some(member) = member_decl_name(line) {
                let path: Vec<&str> = stack.iter().map(|(n, _)| n.as_str()).collect();
                if !block.is_empty() {
                    rows.push((
                        format!("{file}:{}", path.join(".")),
                        member,
                        block.join("\n"),
                    ));
                }
            }
        }
        // Naive brace tracking (schema sources don't put braces in
        // string literals); pop scopes whose block closed. An anonymous
        // `union {` adds depth but no scope, so its fields attribute to
        // the enclosing struct.
        depth += line.matches('{').count();
        for _ in 0..line.matches('}').count() {
            depth = depth.saturating_sub(1);
            while stack.last().is_some_and(|(_, d)| *d >= depth) {
                stack.pop();
            }
        }
        block.clear();
    }
}

/// `interface X` / `struct X` / `enum X` → Some(X).
fn scope_decl_name(line: &str) -> Option<String> {
    for kw in ["interface ", "struct ", "enum "] {
        if let Some(rest) = line.strip_prefix(kw) {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// A member declaration: `name @N (` (a method) or `name @N :` (a
/// field) → Some(name). Enumerants (`name @N;`) and everything else →
/// None. (Enumerant docs can join the same mechanism when something
/// wants them; nothing consumes them yet.)
fn member_decl_name(line: &str) -> Option<String> {
    let name: String = line
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() || !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let rest = line[name.len()..].trim_start();
    let rest = rest.strip_prefix('@')?;
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    let after = rest[digits..].trim_start();
    (after.starts_with('(') || after.starts_with(':')).then_some(name)
}

/// Rebuild `cgr_bytes`' sourceInfo from `rows`: one entry per node,
/// docs joined by display_name and member NAME (methods for
/// interfaces, fields for structs). Nodes without rows get empty
/// docs — existing (mis-attributed or absent) sourceInfo is replaced
/// wholesale, so the output's doc channel is exactly the extracted
/// truth.
pub fn inject(cgr_bytes: &[u8], rows: &[DocRow]) -> capnp::Result<Vec<u8>> {
    let by_key: HashMap<(&str, &str), &str> = rows
        .iter()
        .map(|(k, m, d)| ((k.as_str(), m.as_str()), d.as_str()))
        .collect();

    let msg = serialize::read_message(&mut &cgr_bytes[..], ReaderOptions::new())?;
    let cgr: code_generator_request::Reader = msg.get_root()?;
    let nodes = cgr.get_nodes()?;

    let mut out = capnp::message::Builder::new_default();
    out.set_root(cgr)?;
    let root: code_generator_request::Builder = out.get_root()?;
    let mut si = root.init_source_info(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        let mut e = si.reborrow().get(i as u32);
        e.set_id(n.get_id());
        let display = n.get_display_name()?.to_str()?;
        if let Some(doc) = by_key.get(&(display, "")) {
            e.set_doc_comment(doc);
        }
        match n.which()? {
            node::Interface(ifc) => {
                let methods = ifc.get_methods()?;
                let mut ms = e.init_members(methods.len());
                for (j, m) in methods.iter().enumerate() {
                    let name = m.get_name()?.to_str()?;
                    if let Some(doc) = by_key.get(&(display, name)) {
                        ms.reborrow().get(j as u32).set_doc_comment(doc);
                    }
                }
            }
            node::Struct(st) => {
                let fields = st.get_fields()?;
                let mut ms = e.init_members(fields.len());
                for (j, f) in fields.iter().enumerate() {
                    let name = f.get_name()?.to_str()?;
                    if let Some(doc) = by_key.get(&(display, name)) {
                        ms.reborrow().get(j as u32).set_doc_comment(doc);
                    }
                }
            }
            _ => {}
        }
    }

    let mut bytes = Vec::new();
    serialize::write_message(&mut bytes, &out)?;
    Ok(bytes)
}

/// Convenience for build scripts: extract from `sources` and inject
/// into `cgr_bytes` in one step. `sources` should cover every file
/// whose nodes may appear in the CGR (imports included) so imported
/// nodes get their docs too.
pub fn extract_and_inject(cgr_bytes: &[u8], sources: &[(&str, &str)]) -> capnp::Result<Vec<u8>> {
    let mut rows = Vec::new();
    for (file, src) in sources {
        extract(file, src, &mut rows);
    }
    inject(cgr_bytes, &rows)
}

/// Soft lint for the summary convention (docs/grain-authoring.md): a
/// doc block's FIRST paragraph becomes the LLM tool / param
/// description, so it should be a 1–2 sentence caller-facing summary.
/// Returns one message per member whose first paragraph exceeds
/// `max_chars` when reflowed; build scripts print them as
/// cargo:warning lines — a nudge, never a failure.
pub fn lint_first_paragraphs(rows: &[DocRow], max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    for (key, member, doc) in rows {
        if member.is_empty() {
            continue; // interface/struct docs don't feed tool defs
        }
        let para: String = doc
            .split("\n\n")
            .next()
            .unwrap_or(doc)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // Count CHARS, not bytes — the schemas are em-dash-rich UTF-8
        // and the budget is about prompt length, not encoding.
        let n = para.chars().count();
        if n > max_chars {
            out.push(format!(
                "{key}.{member}: first doc paragraph is {n} chars (aim ≤{max_chars}) — it \
                 becomes the LLM tool description; move detail to a later paragraph"
            ));
        }
    }
    out
}

/// The summary-convention lint threshold shared by the build scripts.
pub const LINT_MAX_FIRST_PARAGRAPH: usize = 250;

/// The bridge-crate shape: capnpc already wrote the CGR to
/// `schema_bin` (`raw_code_generator_request_path`); rewrite it in
/// place with docs extracted from `capnp_src`. The extraction key is
/// the source's file name, matching the display_name capnpc produces
/// for a bare relative `.file(...)`. Nodes imported from OTHER files
/// keep empty docs here — their docs arrive when that file's own
/// bytes are loaded, since a SchemaCache merges docs across loads.
pub fn inject_into_file(
    schema_bin: &std::path::Path,
    capnp_src: &std::path::Path,
) -> capnp::Result<()> {
    let bytes = std::fs::read(schema_bin)
        .map_err(|e| capnp::Error::failed(format!("read {}: {e}", schema_bin.display())))?;
    let src = std::fs::read_to_string(capnp_src)
        .map_err(|e| capnp::Error::failed(format!("read {}: {e}", capnp_src.display())))?;
    let file = capnp_src
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or_else(|| capnp::Error::failed("capnp source has no file name".into()))?;
    let mut rows = Vec::new();
    extract(file, &src, &mut rows);
    for msg in lint_first_paragraphs(&rows, LINT_MAX_FIRST_PARAGRAPH) {
        println!("cargo:warning=schema-docs: {msg}");
    }
    let injected = inject(&bytes, &rows)?;
    std::fs::write(schema_bin, injected)
        .map_err(|e| capnp::Error::failed(format!("write {}: {e}", schema_bin.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = r#"@0xeeaabbccddee0012;

# Adjacent to the interface — its doc.
interface Api {
  # Doc for ping.
  ping @0 () -> ();

  pong @1 () -> ();

  # Detached by the blank line below — attaches to nothing.

  gone @2 () -> ();

  # Nested struct doc.
  struct Item {
    # Doc for name.
    name @0 :Text;
    union {
      # Doc for a union arm (a field of Item).
      left @1 :Text;
      right @2 :Text;
    }
  }
}
"#;

    #[test]
    fn extract_walks_scopes_and_members() {
        let mut rows = Vec::new();
        extract("t.capnp", SCHEMA, &mut rows);
        let get = |k: &str, m: &str| {
            rows.iter()
                .find(|(rk, rm, _)| rk == k && rm == m)
                .map(|(_, _, d)| d.as_str())
        };
        assert_eq!(
            get("t.capnp:Api", ""),
            Some("Adjacent to the interface — its doc.")
        );
        assert_eq!(get("t.capnp:Api", "ping"), Some("Doc for ping."));
        assert_eq!(get("t.capnp:Api", "pong"), None);
        assert_eq!(get("t.capnp:Api", "gone"), None, "blank line detaches");
        assert_eq!(get("t.capnp:Api.Item", ""), Some("Nested struct doc."));
        assert_eq!(get("t.capnp:Api.Item", "name"), Some("Doc for name."));
        assert_eq!(
            get("t.capnp:Api.Item", "left"),
            Some("Doc for a union arm (a field of Item)."),
            "union arms attribute to the enclosing struct"
        );
    }

    #[test]
    fn member_matcher_distinguishes_methods_fields_and_noise() {
        assert_eq!(member_decl_name("ping @0 () -> ();"), Some("ping".into()));
        assert_eq!(member_decl_name("name @0 :Text;"), Some("name".into()));
        assert_eq!(member_decl_name("enumerant @0;"), None);
        assert_eq!(
            member_decl_name("annotation doc @0xabc (field) :Text;"),
            None
        );
        assert_eq!(member_decl_name("} "), None);
    }

    /// Full round trip: compile the fixture with the real capnp
    /// binary, inject, and read the docs back off the rebuilt CGR the
    /// way a runtime consumer would.
    #[test]
    fn inject_round_trips_through_a_real_cgr() {
        let dir = std::env::temp_dir().join(format!("schema-docs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.capnp");
        std::fs::write(&path, SCHEMA).unwrap();
        let out = std::process::Command::new("capnp")
            .args(["compile", "-o-"])
            .arg(format!("--src-prefix={}", dir.display()))
            .arg(&path)
            .output()
            .expect("capnp binary (build prerequisite)");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );

        let injected = extract_and_inject(&out.stdout, &[("t.capnp", SCHEMA)]).unwrap();

        let msg = serialize::read_message(&mut &injected[..], ReaderOptions::new()).unwrap();
        let cgr: code_generator_request::Reader = msg.get_root().unwrap();
        // Find the Api node id by display_name, then its sourceInfo.
        let nodes = cgr.get_nodes().unwrap();
        let api = nodes
            .iter()
            .find(|n| n.get_display_name().unwrap().to_str().unwrap() == "t.capnp:Api")
            .expect("Api node");
        let methods = match api.which().unwrap() {
            node::Interface(i) => i.get_methods().unwrap(),
            _ => panic!("Api is an interface"),
        };
        let si = cgr
            .get_source_info()
            .unwrap()
            .iter()
            .find(|s| s.get_id() == api.get_id())
            .expect("Api sourceInfo");
        assert_eq!(
            si.get_doc_comment().unwrap().to_str().unwrap(),
            "Adjacent to the interface — its doc."
        );
        // Member docs joined by name, whatever the order.
        let members = si.get_members().unwrap();
        assert_eq!(members.len(), methods.len());
        for (j, m) in methods.iter().enumerate() {
            let name = m.get_name().unwrap().to_str().unwrap();
            let doc = members
                .get(j as u32)
                .get_doc_comment()
                .unwrap()
                .to_str()
                .unwrap();
            match name {
                "ping" => assert_eq!(doc, "Doc for ping."),
                _ => assert_eq!(doc, "", "{name} has no doc"),
            }
        }
        // Struct field docs too.
        let item = nodes
            .iter()
            .find(|n| n.get_display_name().unwrap().to_str().unwrap() == "t.capnp:Api.Item")
            .expect("Item node");
        let si = cgr
            .get_source_info()
            .unwrap()
            .iter()
            .find(|s| s.get_id() == item.get_id())
            .expect("Item sourceInfo");
        assert_eq!(
            si.get_doc_comment().unwrap().to_str().unwrap(),
            "Nested struct doc."
        );
        let fields = match item.which().unwrap() {
            node::Struct(s) => s.get_fields().unwrap(),
            _ => panic!("Item is a struct"),
        };
        let members = si.get_members().unwrap();
        let mut saw_left = false;
        for (j, f) in fields.iter().enumerate() {
            if f.get_name().unwrap().to_str().unwrap() == "left" {
                saw_left = true;
                assert_eq!(
                    members
                        .get(j as u32)
                        .get_doc_comment()
                        .unwrap()
                        .to_str()
                        .unwrap(),
                    "Doc for a union arm (a field of Item)."
                );
            }
        }
        assert!(saw_left, "union arm present in fields");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
