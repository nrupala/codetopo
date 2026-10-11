// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! `codetopo render` end to end: index a fixture, render it twice,
//! and require byte-identical spec JSON and SVG — the determinism
//! contract of docs/render.md, enforced at the shipped binary.

use std::path::PathBuf;
use std::process::Command;

use codetopo_cli::index_repo;

fn fixture(tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("codetopo-render-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    std::fs::write(
        repo.join("a.rs"),
        "pub fn root() { helper(); }\nfn helper() { root(); }\n",
    )
    .expect("fixture file");
    let db = dir.join("graph.db");
    index_repo(&repo, &db, "fixture", None, None).expect("index");
    (dir, db)
}

fn render(db: &PathBuf, stem: &str) -> (Vec<u8>, Vec<u8>) {
    let dir = db.parent().expect("db dir");
    let json_path = dir.join(format!("{stem}.json"));
    let svg_path = dir.join(format!("{stem}.svg"));
    let out = Command::new(env!("CARGO_BIN_EXE_codetopo"))
        .args(["render", "--db"])
        .arg(db)
        .arg("--out")
        .arg(&json_path)
        .arg("--svg")
        .arg(&svg_path)
        .output()
        .expect("run render");
    assert!(out.status.success(), "render failed: {}", String::from_utf8_lossy(&out.stderr));
    (
        std::fs::read(&json_path).expect("spec json"),
        std::fs::read(&svg_path).expect("svg"),
    )
}

#[test]
fn render_is_byte_deterministic_and_well_formed() {
    let (dir, db) = fixture("det");
    let (json1, svg1) = render(&db, "one");
    let (json2, svg2) = render(&db, "two");
    assert_eq!(json1, json2, "same graph → same spec bytes");
    assert_eq!(svg1, svg2, "same graph → same SVG bytes");

    let spec: serde_json::Value = serde_json::from_slice(&json1).expect("spec parses");
    assert_eq!(spec["layout_version"], 1);
    let ids: Vec<&str> = spec["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with("::root")), "root placed: {ids:?}");
    assert!(ids.iter().any(|id| id.ends_with("::helper")), "helper placed: {ids:?}");
    // root ↔ helper form a cycle: the layout must share their layer.
    let y_of = |suffix: &str| {
        spec["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["id"].as_str().unwrap().ends_with(suffix))
            .unwrap()["y"]
            .clone()
    };
    assert_eq!(y_of("::root"), y_of("::helper"), "cycle members share a layer");

    let svg = String::from_utf8(svg1).expect("svg is utf-8");
    assert!(svg.starts_with("<?xml version=\"1.0\""));
    assert!(svg.contains("<svg"), "svg root present");
    assert!(svg.trim_end().ends_with("</svg>"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn render_defaults_to_stdout_json() {
    let (dir, db) = fixture("stdout");
    let out = Command::new(env!("CARGO_BIN_EXE_codetopo"))
        .args(["render", "--db"])
        .arg(&db)
        .output()
        .expect("run render");
    assert!(out.status.success());
    let spec: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is the spec");
    assert_eq!(spec["layout_version"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}
