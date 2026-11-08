fn main() {
    println!("cargo:rerun-if-changed=stufftracker-v1.capnp");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let schema = out.join("stufftracker-v1.schema.bin");
    capnpc::CompilerCommand::new()
        .file("stufftracker-v1.capnp")
        .raw_code_generator_request_path(&schema)
        .run()
        .expect("compile stufftracker.v1 schema (requires capnp)");
    darude_schema_docs::inject_into_file(&schema, std::path::Path::new("stufftracker-v1.capnp"))
        .expect("inject API documentation");
}
