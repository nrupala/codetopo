// Capture the bound codetopo-core version from that crate's own
// manifest, so `codetopo_core_version()` reports provenance from the
// source of truth instead of restating a literal here (the same
// discipline the JavaScript napi surface's build.rs follows).
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    // surfaces/r/src/rust -> repository root is four levels up.
    let core_manifest = manifest_dir
        .join("../../../../crates/codetopo-core/Cargo.toml");
    println!("cargo:rerun-if-changed={}", core_manifest.display());
    let text = std::fs::read_to_string(&core_manifest)
        .unwrap_or_else(|e| panic!("read {}: {e}", core_manifest.display()));
    let mut in_package = false;
    let mut version = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && line.starts_with("version") {
            if let Some(v) = line
                .split_once('=')
                .map(|(_, v)| v.trim().trim_matches('"'))
            {
                if !v.is_empty() {
                    version = Some(v.to_string());
                    break;
                }
            }
        }
    }
    let version = version.expect("codetopo-core Cargo.toml carries a [package] version");
    println!("cargo:rustc-env=CODETOPO_CORE_VERSION={version}");
}
