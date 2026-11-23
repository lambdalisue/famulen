//! Build the API from its protobuf definition. `buf` builds the descriptor
//! set with the comments kept, so the secretary's tools can be described
//! from them; the Rust code is generated from the same set.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../proto");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let set = out.join("famulen.binpb");
    println!("cargo:rerun-if-changed={}", root.display());
    let status = Command::new("buf")
        .arg("build")
        .arg(&root)
        .arg("--as-file-descriptor-set")
        .arg("-o")
        .arg(&set)
        .status()
        .expect("buf is needed to build the API; run inside `nix develop`");
    assert!(status.success(), "buf build failed");
    let files: Vec<String> = std::fs::read_dir(root.join("famulen/v1"))
        .expect("proto/famulen/v1")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".proto"))
        .map(|name| format!("famulen/v1/{name}"))
        .collect();
    connectrpc_build::Config::new()
        .descriptor_set(&set)
        .files(&files)
        .include_file("_connectrpc.rs")
        .compile()
        .expect("generating the API");
}
