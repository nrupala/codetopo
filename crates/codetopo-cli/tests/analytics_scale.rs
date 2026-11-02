// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The analytics scale gate (RA5 of the Rust analytics engine).
//! `#[ignore]`d so CI stays fast; run it deliberately, in release
//! mode:
//!
//! ```sh
//! cargo test --release -p codetopo-cli --test analytics_scale -- --ignored --nocapture
//! ```
//!
//! Same 10,000-file fixture as `scale_gate.rs`. Every analytics
//! surface runs over the indexed corpus TWICE and must produce
//! byte-identical answers (SPEC §7: determinism at corpus scale),
//! under generous tripwire budgets — `docs/scale-gate.md` records
//! the measured numbers these tripwires guard. Two answers are
//! asserted as DESIGNED outcomes of this fixture, not failures:
//! eigenvector centrality reports an absence (power iteration does
//! not converge on this graph within the method's 100 iterations),
//! and the force layout declines (40,001 nodes condense to 30,031
//! components, above its 3,000 ceiling — the layered layout is the
//! full-graph drawing).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use codetopo_core::RankMetric;

const FILES: usize = 10_000;

/// Tripwires (seconds, release mode, CI-class machine). Measured
/// 2026-11-02 on this class of machine — pagerank 0.5, katz 0.5,
/// closeness 13, betweenness 33, communities 0.9, topo-sort 0.5,
/// render layered 0.7 — each tripwire is set well above its
/// measurement to catch pathological regressions, not to claim
/// performance.
const PAGERANK_BUDGET: u64 = 60;
const KATZ_BUDGET: u64 = 60;
const EIGENVECTOR_BUDGET: u64 = 60;
const CLOSENESS_BUDGET: u64 = 180;
const BETWEENNESS_BUDGET: u64 = 300;
const COMMUNITIES_BUDGET: u64 = 120;
const TOPO_BUDGET: u64 = 60;
const RENDER_BUDGET: u64 = 120;

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
    let dir =
        std::env::temp_dir().join(format!("codetopo-analytics-scale-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

/// Run `f` twice; both runs must finish within `budget` seconds and
/// produce byte-identical serializations. Returns the first output.
fn twice(name: &str, budget: u64, f: impl Fn() -> serde_json::Value) -> serde_json::Value {
    let started = Instant::now();
    let first = f();
    let elapsed = started.elapsed();
    println!("{name}: {:?} (tripwire {budget}s)", elapsed);
    assert!(
        elapsed.as_secs() < budget,
        "{name} blew the {budget}s tripwire: {elapsed:?}"
    );
    let second = f();
    assert_eq!(
        serde_json::to_vec(&first).expect("serializes"),
        serde_json::to_vec(&second).expect("serializes"),
        "{name} must be run-twice byte-identical at corpus scale"
    );
    first
}

#[test]
#[ignore = "analytics scale gate: run deliberately in release mode (see docs/scale-gate.md)"]
fn analytics_over_the_ten_thousand_file_corpus() {
    let root = tempdir("fixture");
    write_fixture(&root);
    let db_dir = tempdir("db");
    let db = db_dir.join("scale.db");
    let report = codetopo_cli::index_repo(&root, &db, "scale-fixture", None, None)
        .expect("indexing 10k files must succeed");
    assert_eq!(report.files_indexed, FILES);
    assert_eq!(report.files_failed, 0);

    let store = codetopo_store::Store::open(&db).expect("open store");
    let graph = store.load_graph().expect("load graph");
    let proj = codetopo_core::AnalyticsProjection::build(&graph);
    println!(
        "corpus: {} graph nodes, {} projection nodes, {} projection edges",
        graph.node_count(),
        proj.node_count(),
        proj.directed_edges().len()
    );
    assert_eq!(proj.node_count(), 40_001, "the fixture's projection size");

    // The iterative measures: present on this corpus (except
    // eigenvector, whose absence is the designed answer here).
    let pagerank = twice("rank pagerank", PAGERANK_BUDGET, || {
        codetopo_cli::rank_value(&graph, RankMetric::PageRank, usize::MAX)
    });
    assert_eq!(pagerank["total"], 40_001);
    let _katz = twice("rank katz", KATZ_BUDGET, || {
        codetopo_cli::rank_value(&graph, RankMetric::Katz, usize::MAX)
    });
    let eigenvector = twice("rank eigenvector", EIGENVECTOR_BUDGET, || {
        codetopo_cli::rank_value(&graph, RankMetric::Eigenvector, usize::MAX)
    });
    assert_eq!(
        eigenvector["measured"], false,
        "eigenvector on this corpus is a stated absence, never a partial vector: {eigenvector}"
    );

    // The BFS/Brandes measures.
    let _closeness = twice("rank closeness", CLOSENESS_BUDGET, || {
        codetopo_cli::rank_value(&graph, RankMetric::Closeness, usize::MAX)
    });
    let _betweenness = twice("rank betweenness", BETWEENNESS_BUDGET, || {
        codetopo_cli::rank_value(&graph, RankMetric::Betweenness, usize::MAX)
    });

    // Communities and the order.
    let communities = twice("communities", COMMUNITIES_BUDGET, || {
        codetopo_cli::communities_value(&graph, usize::MAX)
    });
    assert!(communities["modularity"].as_f64().expect("Q present") > 0.0);
    let topo = twice("topo-sort", TOPO_BUDGET, || {
        codetopo_cli::topo_sort_value(&graph)
    });
    assert_eq!(topo["cyclic_groups"], 1, "the fixture's one giant cycle");

    // Layouts: layered in full; force declines by design here.
    let _layered = twice("render layered", RENDER_BUDGET, || {
        codetopo_cli::render_value(&graph, "layered").expect("layered renders")
    });
    let force = codetopo_cli::render_value(&graph, "force").expect("force answers");
    assert_eq!(
        force["rendered"], false,
        "the corpus is above the force ceilings — a stated decline: {force}"
    );
    assert!(
        force["reason"].as_str().expect("reason").contains("3000"),
        "the decline names the ceiling"
    );

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&db_dir);
}
