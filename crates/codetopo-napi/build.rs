// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Build glue for the napi surface.
//!
//! Two jobs: napi's own setup, and provenance — `CORE_VERSION` in the JS
//! surface must be the version of the `codetopo-core` crate actually bound,
//! read here from that crate's own manifest rather than restated as a
//! literal that could drift from it.

use std::path::PathBuf;

fn main() {
    napi_build::setup();

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let core_manifest = manifest_dir.join("../codetopo-core/Cargo.toml");
    let text = std::fs::read_to_string(&core_manifest).unwrap_or_else(|e| {
        panic!(
            "read the bound codetopo-core manifest at {}: {e}",
            core_manifest.display()
        )
    });
    let mut in_package = false;
    let mut version: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && trimmed.starts_with("version") {
            if let Some(v) = trimmed
                .split('=')
                .nth(1)
                .map(|s| s.trim().trim_matches('"').to_string())
            {
                version = Some(v);
                break;
            }
        }
    }
    let version = version.expect("codetopo-core's Cargo.toml carries a [package] version");
    println!("cargo:rustc-env=CODETOPO_CORE_VERSION={version}");
    println!(
        "cargo:rerun-if-changed={}",
        core_manifest.display()
    );
}
