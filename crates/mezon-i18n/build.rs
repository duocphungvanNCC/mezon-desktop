use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn emit(fn_name: &str, json_path: &Path, out: &mut String) {
    let json = fs::read_to_string(json_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", json_path.display()));
    let map: BTreeMap<String, String> = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("parse {}: {e}", json_path.display()));
    out.push_str(&format!(
        "fn {fn_name}(key: &str) -> Option<&'static str> {{\n    Some(match key {{\n"
    ));
    for (k, v) in &map {
        out.push_str(&format!("        {k:?} => {v:?},\n"));
    }
    out.push_str("        _ => return None,\n    })\n}\n");
    println!("cargo:rerun-if-changed={}", json_path.display());
}

fn main() {
    let assets = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets/i18n");
    let mut out = String::new();
    emit("lookup_en", &assets.join("en.json"), &mut out);
    emit("lookup_vi", &assets.join("vi.json"), &mut out);
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("i18n_gen.rs");
    fs::write(&dest, out).unwrap();
}
