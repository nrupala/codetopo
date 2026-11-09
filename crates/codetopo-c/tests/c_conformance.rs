// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! C conformance runner: compile `tests/conformance.c` with cc
//! against the built cdylib and run it — the F1–F5 known-answer
//! battery exercised through the actual C ABI, not through Rust.
//!
//! Also pins the header: every function declared in
//! `include/codetopo.h` must be an exported symbol of the built
//! cdylib, and the cdylib must not export `codetopo_*` symbols the
//! header does not declare.
//!
//! Where the cdylib comes from: `cargo build` emits it into
//! `target/debug/`, but `cargo test` does NOT (it builds the rlib
//! the test harness links, not the cdylib artifact) — on a clean
//! checkout the file simply isn't there. The resolver below first
//! looks in the usual places, and if the cdylib is absent builds it
//! with a nested `cargo build` into a SCRATCH target dir (the outer
//! cargo holds the workspace target dir's lock while tests run, so
//! a nested build into the same dir would deadlock).
//!
//! Unix only: the Windows CI job builds the agent binaries, not this
//! crate's test suite (the dedicated diaspora-cabi workflow runs this
//! on Linux).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

fn cdylib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libcodetopo_c.dylib"
    } else {
        "libcodetopo_c.so"
    }
}

/// The built cdylib for this crate, resolved once per test process:
/// found in the workspace target dir if a `cargo build` already
/// produced it, otherwise built into a scratch target dir.
fn cdylib_path() -> PathBuf {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(resolve_cdylib).clone()
}

fn resolve_cdylib() -> PathBuf {
    let name = cdylib_name();
    let root = repo_root();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(td) = std::env::var_os("CARGO_TARGET_DIR") {
        candidates.push(PathBuf::from(td).join("debug").join(name));
    }
    candidates.push(root.join("target").join("debug").join(name));
    candidates.push(root.join("target").join("debug").join("deps").join(name));
    // A hashed copy in deps/ (libcodetopo_c-<hash>.so) also serves.
    if let Ok(entries) = std::fs::read_dir(root.join("target").join("debug").join("deps")) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("libcodetopo_c") && n.ends_with(name.rsplit('.').next().unwrap_or("so"))
                        && !n.ends_with(".rlib")
                })
                .unwrap_or(false)
            {
                candidates.push(p);
            }
        }
    }
    if let Some(found) = candidates.iter().find(|p| p.is_file()) {
        return found.clone();
    }

    // Build it, into a scratch target dir (see the module docs for
    // why not the workspace's own). Deps come from the warm registry
    // cache; offline mode keeps the nested build from reaching for
    // the network the outer build may not have needed.
    let scratch = std::env::temp_dir().join(format!("codetopo-c-cdylib-{}", std::process::id()));
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let status = Command::new(&cargo)
        .args(["build", "-p", "codetopo-c", "--target-dir"])
        .arg(&scratch)
        .current_dir(&root)
        .env("CARGO_NET_OFFLINE", "true")
        .status()
        .expect("nested cargo build runs");
    assert!(status.success(), "nested cargo build of the cdylib failed");
    let built = scratch.join("debug").join(name);
    assert!(
        built.is_file(),
        "nested cargo build did not produce {}",
        built.display()
    );
    built
}

/// Function names declared in the header (same extraction rule as
/// the unit test in src/lib.rs).
fn header_declared() -> Vec<String> {
    let header = std::fs::read_to_string(manifest_dir().join("include/codetopo.h"))
        .expect("header reads");
    let mut names: Vec<String> = Vec::new();
    for line in header.lines() {
        let line = line.trim();
        if !line.ends_with(';') || line.starts_with('*') || line.starts_with("/*") {
            continue;
        }
        // Token scan (see the unit test in src/lib.rs): keep every
        // `codetopo_*` token immediately followed by `(`.
        let mut rest = line;
        while let Some(start) = rest.find("codetopo_") {
            let tail = &rest[start..];
            let end = tail
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(tail.len());
            if tail[end..].starts_with('(') {
                names.push(tail[..end].to_string());
            }
            rest = &tail[end..];
        }
    }
    names.sort();
    names.dedup();
    names
}

#[test]
fn header_matches_cdylib_exports() {
    let lib = cdylib_path();
    let output = Command::new("nm")
        .args(["-D", "--defined-only"])
        .arg(&lib)
        .output()
        .expect("nm runs");
    assert!(output.status.success(), "nm failed on {}", lib.display());
    let text = String::from_utf8_lossy(&output.stdout);
    let mut exported: Vec<String> = text
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .filter(|name| name.starts_with("codetopo_"))
        .map(|name| name.trim_start_matches('_').to_string())
        .collect();
    exported.sort();
    exported.dedup();
    assert_eq!(
        exported,
        header_declared(),
        "cdylib codetopo_* exports drifted from include/codetopo.h"
    );
}

#[test]
fn c_conformance_battery() {
    let lib = cdylib_path();
    let lib_dir = lib.parent().expect("cdylib has a directory");

    let work = tempfile::tempdir().expect("temp dir");
    let exe = work.path().join("conformance");
    let src = manifest_dir().join("tests/conformance.c");

    // Compile with cc against the cdylib + header, rpath to the
    // cdylib's directory so the binary runs without LD_LIBRARY_PATH.
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let status = Command::new(&cc)
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-I")
        .arg(manifest_dir().join("include"))
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .arg("-L")
        .arg(lib_dir)
        .arg("-lcodetopo_c")
        .arg("-lm")
        .arg(format!("-Wl,-rpath,{}", lib_dir.display()))
        .status()
        .expect("cc runs");
    assert!(status.success(), "conformance.c failed to compile/link against the cdylib");

    let run_dir = work.path().join("work");
    let output = Command::new(&exe)
        .arg(repo_root())
        .arg(&run_dir)
        .output()
        .expect("conformance binary runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    println!("{stdout}{stderr}");
    assert!(
        output.status.success(),
        "C conformance battery failed (see output above)"
    );
    assert!(
        stdout.contains("CONFORMANCE PASS"),
        "conformance did not report PASS: {stdout}"
    );
}
