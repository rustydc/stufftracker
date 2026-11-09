fn main() {
    println!("cargo:rerun-if-changed=stufftracker-v1.capnp");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let schema = out.join("stufftracker-v1.schema.bin");
    capnpc::CompilerCommand::new()
        .file("stufftracker-v1.capnp")
        // Darude's shared blob.capnp; its generated type lives in darude_wire,
        // the one the runtime and agent loop use.
        .import_path("../../vendor/darude/schemas")
        .crate_provides("darude_wire", [0x88958d009b0f5516u64])
        .raw_code_generator_request_path(&schema)
        .run()
        .expect("compile stufftracker.v1 schema (requires capnp)");
    darude_schema_docs::inject_into_file(&schema, std::path::Path::new("stufftracker-v1.capnp"))
        .expect("inject API documentation");
    // Client types for the bridges StuffTracker reads through optional slots.
    println!("cargo:rerun-if-changed=schemas");
    capnpc::CompilerCommand::new()
        .src_prefix("schemas")
        .file("schemas/homeassistant-v1.capnp")
        .file("schemas/outline-v1.capnp")
        .import_path("../../vendor/darude/schemas")
        .crate_provides("darude_wire", [0x88958d009b0f5516u64])
        .run()
        .expect("compile bridge client schemas");
}
