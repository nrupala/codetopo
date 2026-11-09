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
//! Unix only: the Windows CI job builds the agent binaries, not this
//! crate's test suite (the dedicated diaspora-cabi workflow runs this
//! on Linux).

#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// The built cdylib for this crate, in the workspace target dir.
fn cdylib_path() -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target"));
    let file = if cfg!(target_os = "macos") {
        "libcodetopo_c.dylib"
    } else {
        "libcodetopo_c.so"
    };
    target_dir.join("debug").join(file)
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
    assert!(
        lib.is_file(),
        "cdylib not found at {} — cargo should have built it with the crate",
        lib.display()
    );
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
    assert!(lib.is_file(), "cdylib not found at {}", lib.display());
    let lib_dir = lib.parent().expect("cdylib has a directory");

    let work = tempfile::tempdir().expect("temp dir");
    let exe = work.path().join("conformance");
    let src = manifest_dir().join("tests/conformance.c");

    // Compile with cc against the cdylib + header, rpath to the
    // target dir so the binary runs without LD_LIBRARY_PATH.
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
