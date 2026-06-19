use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if env::var("PROTOC").is_err()
        && let Ok(path) = protoc_bin_vendored::protoc_bin_path()
    {
        unsafe { env::set_var("PROTOC", path) };
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    // Canonical schema lives in the `mezon-protocol` git submodule at the repo root (same approach
    // as mezon-android). Compile straight from it — no vendored copies, no drift. Include dir =
    // submodule root so `import "api/api.proto"` resolves.
    let proto_root = manifest_dir.join("../../mezon-protocol").canonicalize()?;
    let api_proto = proto_root.join("api/api.proto");
    let realtime_proto = proto_root.join("rtapi/realtime.proto");

    prost_build::Config::new()
        .out_dir(&out_dir)
        .compile_protos(&[api_proto.clone(), realtime_proto.clone()], &[proto_root])?;

    println!("cargo:rerun-if-changed={}", api_proto.display());
    println!("cargo:rerun-if-changed={}", realtime_proto.display());

    Ok(())
}
