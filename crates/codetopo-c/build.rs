// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Build script: derive the bound core's version from the core crate's
//! own manifest, so `codetopo_version()` reports the version of the
//! `codetopo-core` this cdylib was built against without this crate
//! duplicating the number as a literal that could drift from it.
//! (codetopo-core exposes no `VERSION` constant; its Cargo.toml
//! `[package] version` is the canonical record every workspace crate
//! already pins against.)

use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let core_manifest = manifest_dir.join("../codetopo-core/Cargo.toml");
    println!("cargo:rerun-if-changed={}", core_manifest.display());
    let text = std::fs::read_to_string(&core_manifest).expect("codetopo-core manifest reads");
    let mut in_package = false;
    let mut version: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && line.starts_with("version") {
            if let Some((_, value)) = line.split_once('=') {
                let value = value.trim().trim_matches('"').trim_matches('\'');
                if !value.is_empty() {
                    version = Some(value.to_string());
                    break;
                }
            }
        }
    }
    let version = version.expect("codetopo-core manifest declares a [package] version");
    println!("cargo:rustc-env=CODETOPO_CORE_VERSION={version}");
}
