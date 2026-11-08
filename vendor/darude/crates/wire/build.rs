// capnpc runs at build time to turn ../../schemas/*.capnp into Rust
// modules under OUT_DIR. Each schema file produces:
//   1. A `<name>_capnp.rs` Rust module that lib.rs `include!`s,
//      giving the typed Reader/Builder/Client/Server APIs.
//   2. A `<name>.schema.bin` containing the serialized
//      capnp::schema::CodeGeneratorRequest for that file. Senders
//      drop this into OpaqueCap.schema so the receiver can parse
//      it with a runtime SchemaLoader (see the OpaqueCap protocol
//      in introspectable.capnp).
//
// Adding a new schema = adding it to this list.

use std::process::Command;

fn main() {
    let schemas_dir = std::path::Path::new("../../schemas");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let files = [
        "accept-cap.capnp",
        "blob.capnp",
        "byte-writer.capnp",
        "agent-notes-v1.capnp",
        "cap-v1.capnp",
        "cap-echo-v1.capnp",
        "change-listener.capnp",
        "code-exec-v1.capnp",
        "delegate-v1.capnp",
        "fs-v1.capnp",
        "grain-v2.capnp",
        "llm-chat-v1.capnp",
        "introspectable.capnp",
        "observer-v1.capnp",
        "persistent.capnp",
        "principal.capnp",
        "restorable.capnp",
        "saver.capnp",
        "schedule-v1.capnp",
        "self-notify-v1.capnp",
        "self-v1.capnp",
        "session.capnp",
        "shelf.capnp",
        "powerbox-request.capnp",
        "spawn.capnp",
        "subagent.capnp",
        "text-writer.capnp",
        "trigger-v1.capnp",
        "user-notify-v1.capnp",
        "web-fetch-v1.capnp",
        "web-search-v1.capnp",
        "workspace-v1.capnp",
    ];

    // 1. Generate Rust modules via capnpc (as before).
    let mut cmd = capnpc::CompilerCommand::new();
    cmd.src_prefix(schemas_dir);
    for f in files {
        let p = schemas_dir.join(f);
        println!("cargo:rerun-if-changed={}", p.display());
        cmd.file(p);
    }
    cmd.run().expect("capnpc failed");

    // 2. Also drop the CodeGeneratorRequest bytes for each file into
    // OUT_DIR. `capnp compile -o-` writes the binary
    // CodeGeneratorRequest to stdout — we capture it per-file so a
    // sender can ship just the relevant schema (not every interface
    // in the workspace) when packaging a cap as an OpaqueCap. Before
    // writing, each CGR's sourceInfo is REBUILT from the schema
    // sources (darude-schema-docs): capnp's native docComment rule
    // can't represent this repo's comment-before-declaration style,
    // so the source text is the doc authority and the emitted bytes
    // are made self-describing — docs travel wherever the schema
    // does. Rows cover every schema file, so imported nodes (blob in
    // llm-chat's CGR, …) get their docs too.
    let mut doc_rows = Vec::new();
    for f in files {
        let src = std::fs::read_to_string(schemas_dir.join(f)).expect("read schema source");
        darude_schema_docs::extract(f, &src, &mut doc_rows);
    }
    // Summary-convention nudge (docs/grain-authoring.md): the first
    // paragraph is the LLM-facing description, so it should stay a
    // 1–2 sentence summary. Warn, never fail.
    for msg in darude_schema_docs::lint_first_paragraphs(
        &doc_rows,
        darude_schema_docs::LINT_MAX_FIRST_PARAGRAPH,
    ) {
        println!("cargo:warning=schema-docs: {msg}");
    }
    for f in files {
        let p = schemas_dir.join(f);
        let stem = std::path::Path::new(f)
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("schema filename has a stem");
        let dest = std::path::Path::new(&out_dir).join(format!("{stem}.schema.bin"));
        let output = Command::new("capnp")
            .args(["compile", "-o-"])
            .arg(format!("--src-prefix={}", schemas_dir.display()))
            .arg(&p)
            .output()
            .expect("invoke capnp compile -o-");
        if !output.status.success() {
            panic!(
                "capnp compile -o- on {}: {}",
                p.display(),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let injected =
            darude_schema_docs::inject(&output.stdout, &doc_rows).expect("inject schema docs");
        std::fs::write(&dest, injected).expect("write schema bytes");
    }
}
