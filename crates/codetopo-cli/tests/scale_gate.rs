// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! The 10,000-file scale gate. `#[ignore]`d so CI stays fast; run it
//! deliberately, in release mode:
//!
//! ```sh
//! cargo test --release -p codetopo-cli --test scale_gate -- --ignored --nocapture
//! ```
//!
//! The fixture mirrors `scripts/gen_scale_fixture.py`: 10,000 files,
//! one call chain spanning all of them (depth 10,000 — the recursion
//! stress for cycle detection), a back edge every 997 files (large
//! real cycles), and an intra-file helper call per file. The gate
//! asserts the whole pipeline — index, persist, reload, and the
//! long-pole queries — completes, and prints the timings
//! `docs/scale-gate.md` records. Time ceilings are generous tripwires
//! against pathological regressions, not performance claims.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

const FILES: usize = 10_000;
/// Generous tripwires (release mode, any CI-class machine).
const INDEX_BUDGET_SECS: u64 = 300;
const QUERY_BUDGET_SECS: u64 = 60;

fn write_fixture(root: &Path) {
    for i in 0..FILES {
        let dir = root.join(format!("dir{:04}", i / 100));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let mut text = String::new();
        if i % 5 < 3 {
            if i + 1 < FILES {
                let _ = writeln!(
                    text,
                    "use crate::dir{:04}::file{:05}::step{:05};",
                    (i + 1) / 100,
                    i + 1,
                    i + 1
                );
            }
            let _ = writeln!(text, "pub fn step{i:05}() {{");
            if i + 1 < FILES {
                let _ = writeln!(text, "    step{:05}();", i + 1);
            }
            if i >= 997 && i % 997 == 0 {
                let _ = writeln!(text, "    step{:05}();", i - 997);
            }
            let _ = writeln!(text, "    help{i:05}();");
            let _ = writeln!(text, "}}");
            let _ = writeln!(text, "pub fn help{i:05}() {{}}");
            std::fs::write(dir.join(format!("file{i:05}.rs")), text).expect("write rs");
        } else {
            if i + 1 < FILES {
                let _ = writeln!(
                    text,
                    "import {{ step{:05} }} from '../dir{:04}/file{:05}';",
                    i + 1,
                    (i + 1) / 100,
                    i + 1
                );
            }
            let _ = writeln!(text, "export function step{i:05}(): void {{");
            if i + 1 < FILES {
                let _ = writeln!(text, "  step{:05}();", i + 1);
            }
            if i >= 997 && i % 997 == 0 {
                let _ = writeln!(text, "  step{:05}();", i - 997);
            }
            let _ = writeln!(text, "  help{i:05}();");
            let _ = writeln!(text, "}}");
            let _ = writeln!(text, "export function help{i:05}(): void {{}}");
            std::fs::write(dir.join(format!("file{i:05}.ts")), text).expect("write ts");
        }
    }
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("codetopo-scale-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

#[test]
#[ignore = "scale gate: run deliberately in release mode (see docs/scale-gate.md)"]
fn ten_thousand_file_gate() {
    let root = tempdir("fixture");
    let write_started = Instant::now();
    write_fixture(&root);
    println!("fixture: {FILES} files written in {:?}", write_started.elapsed());

    // Indexing runs the full pipeline, including cycle detection
    // (Tarjan SCC) over the 10,000-deep call chain during build.
    let db_dir = tempdir("db");
    let db = db_dir.join("scale.db");
    let index_started = Instant::now();
    let report = codetopo_cli::index_repo(&root, &db, "scale-fixture", None, None)
        .expect("indexing 10k files must succeed");
    let index_elapsed = index_started.elapsed();
    println!(
        "index: {:?} — {} files indexed, {} failed, {} nodes, {} edges, {} diagnostics",
        index_elapsed,
        report.files_indexed,
        report.files_failed,
        report.nodes,
        report.edges,
        report.diagnostics
    );
    assert_eq!(report.files_indexed, FILES, "every fixture file indexed");
    assert_eq!(report.files_failed, 0);
    assert!(
        index_elapsed.as_secs() < INDEX_BUDGET_SECS,
        "indexing blew the {INDEX_BUDGET_SECS}s tripwire: {index_elapsed:?}"
    );

    let store = codetopo_store::Store::open(&db).expect("open store");
    let load_started = Instant::now();
    let graph = store.load_graph().expect("load graph");
    println!("load_graph: {:?}", load_started.elapsed());
    assert_eq!(graph.node_count(), report.nodes);
    assert_eq!(graph.edge_count(), report.edges);

    let diagnostics = store.load_diagnostics().expect("load diagnostics");
    assert!(
        diagnostics
            .iter()
            .any(|d| matches!(d, codetopo_core::Diagnostic::Cycle { .. })),
        "the fixture's back edges must surface cycle diagnostics"
    );

    // Long-pole queries, addressed by discovered ids (never hardcoded):
    // descendants of the chain head reach nearly every step function;
    // blast radius of the chain tail reaches back just as far.
    let head_hits = graph.search_symbols("step00000", 5);
    let head = head_hits
        .iter()
        .find(|n| n.label == "step00000")
        .expect("chain head discoverable")
        .id
        .clone();
    let desc_started = Instant::now();
    let descendants = graph.descendants(&head);
    let desc_elapsed = desc_started.elapsed();
    println!(
        "descendants(head): {:?} — {} nodes",
        desc_elapsed,
        descendants.len()
    );
    assert!(
        descendants.len() > 9_000,
        "the chain must be reachable end to end, got {}",
        descendants.len()
    );

    let tail_hits = graph.search_symbols("step09999", 5);
    let tail = tail_hits
        .iter()
        .find(|n| n.label == "step09999")
        .expect("chain tail discoverable")
        .id
        .clone();
    let blast_started = Instant::now();
    let blast = graph.blast_radius(&tail);
    let blast_elapsed = blast_started.elapsed();
    println!(
        "blast_radius(tail): {:?} — {} nodes",
        blast_elapsed,
        blast.len()
    );
    assert!(
        blast.len() > 9_000,
        "the tail's dependents must span the chain, got {}",
        blast.len()
    );

    let path_started = Instant::now();
    let path = graph.path(&head, &tail).expect("a path along the chain exists");
    let path_elapsed = path_started.elapsed();
    println!(
        "path(head -> tail): {:?} — {} steps",
        path_elapsed,
        path.len()
    );
    assert!(path.len() > 9_000, "path follows the chain, got {}", path.len());

    for (name, elapsed) in [
        ("descendants", desc_elapsed),
        ("blast_radius", blast_elapsed),
        ("path", path_elapsed),
    ] {
        assert!(
            elapsed.as_secs() < QUERY_BUDGET_SECS,
            "{name} blew the {QUERY_BUDGET_SECS}s tripwire: {elapsed:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&db_dir);
}
