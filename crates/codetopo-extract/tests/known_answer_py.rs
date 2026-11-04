// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! F2 — the known-answer fixture package (`tests/fixture-py`).
//!
//! Every expectation in this test is derived by hand in the
//! fixture's README.md, which shows the arithmetic. A change that
//! moves a count must move the README in the same PR — that is the
//! point of a known-answer fixture.
//!
//! The pinned surprise, recorded per spec §3.2: the `@property`
//! getter and its `@size.setter` in `core.py` emit the SAME symbol
//! id (`quarry::core::Engine::size`) — no renaming is invented at
//! extraction. The builder's documented duplicate rule then applies:
//! the first occurrence (the getter, line 20) is kept, the duplicate
//! is ignored wholesale, and a `MultiDefinition` diagnostic names
//! both locations. This test pins that whole chain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codetopo_core::diag::Diagnostic;
use codetopo_core::emit::FileSymbols;
use codetopo_core::graph::GraphBuilder;
use codetopo_core::schema::{EdgeKind, NodeKind};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-py")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["__init__.py", "core.py", "shapes.py"] {
        let src = std::fs::read_to_string(dir.join(name)).expect("fixture file readable");
        let fs = codetopo_extract::extract_file("quarry", Path::new(name), &src)
            .expect("fixture extracts");
        out.insert(name.to_string(), fs);
    }
    out
}

fn sorted_ids(fs: &FileSymbols) -> Vec<&str> {
    let mut ids: Vec<&str> = fs.symbols.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    ids
}

#[test]
fn f2_extraction_known_answers() {
    let files = extract_fixture();

    // __init__.py — the package module itself (id == package name).
    let init = &files["__init__.py"];
    assert_eq!(init.module_id, "quarry");
    assert_eq!(sorted_ids(init), vec!["quarry::VERSION", "quarry::__all__"]);
    let init_imports: Vec<(&str, Vec<String>)> = init
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
        .collect();
    assert_eq!(
        init_imports,
        vec![
            ("quarry::core", vec!["Engine".to_string()]),
            ("quarry::shapes", vec!["Shape".to_string()]),
        ]
    );

    // core.py — 8 symbol rows, 7 unique ids: `Engine::size` appears
    // TWICE (property getter line 20, setter line 24), by design.
    let core = &files["core.py"];
    assert_eq!(core.module_id, "quarry::core");
    assert_eq!(
        sorted_ids(core),
        vec![
            "quarry::core::DEFAULT_SIZE",
            "quarry::core::Drawable",
            "quarry::core::Drawable::draw",
            "quarry::core::Engine",
            "quarry::core::Engine::__init__",
            "quarry::core::Engine::render",
            "quarry::core::Engine::size",
            "quarry::core::Engine::size",
        ]
    );
    let size_lines: Vec<u32> = core
        .symbols
        .iter()
        .filter(|s| s.id == "quarry::core::Engine::size")
        .map(|s| s.line)
        .collect();
    assert_eq!(size_lines, vec![20, 24], "getter first, setter second");
    // Drawable is an interface (Protocol base, D2).
    let drawable = core
        .symbols
        .iter()
        .find(|s| s.id == "quarry::core::Drawable")
        .expect("Drawable present");
    assert_eq!(drawable.kind, NodeKind::Interface);
    // core's calls: exactly render's `shape.draw()`.
    let core_calls: Vec<(&str, &str)> = core
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str()))
        .collect();
    assert_eq!(
        core_calls,
        vec![("quarry::core::Engine::render", "shape.draw")]
    );

    // shapes.py — base/derived pair + dataclass; Point's annotated
    // class-level fields are NOT symbols.
    let shapes = &files["shapes.py"];
    assert_eq!(shapes.module_id, "quarry::shapes");
    assert_eq!(
        sorted_ids(shapes),
        vec![
            "quarry::shapes::Circle",
            "quarry::shapes::Circle::__init__",
            "quarry::shapes::Circle::area",
            "quarry::shapes::Point",
            "quarry::shapes::Shape",
            "quarry::shapes::Shape::area",
        ]
    );

    // Totals across the package: 16 symbol rows, 7 imports,
    // 1 call, 2 heritage entries.
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (16, 7, 1, 2)
    );
}

#[test]
fn f2_graph_known_answers_and_the_property_collision() {
    let files = extract_fixture();
    let mut builder = GraphBuilder::new();
    for fs in files.values() {
        builder.add_file(fs.clone());
    }
    let (graph, diags) = builder.build();

    // Diagnostics: exactly two, both understood.
    assert_eq!(diags.len(), 2, "{diags:?}");
    let multi: Vec<&Diagnostic> = diags
        .iter()
        .filter(|d| matches!(d, Diagnostic::MultiDefinition { .. }))
        .collect();
    assert_eq!(multi.len(), 1);
    match multi[0] {
        Diagnostic::MultiDefinition { symbol, locations } => {
            assert_eq!(symbol, "quarry::core::Engine::size");
            let lines: Vec<u32> = locations.iter().map(|l| l.line).collect();
            assert_eq!(lines, vec![20, 24]);
        }
        _ => unreachable!(),
    }
    assert!(diags.iter().any(|d| matches!(
        d,
        Diagnostic::UnresolvedCall { from, target, .. }
            if from == "quarry::core::Engine::render" && target == "shape.draw"
    )));

    // The builder kept the FIRST `size` — the getter at line 20 with
    // the getter's signature; the setter (line 24) is ignored
    // wholesale (graph.rs's documented duplicate rule).
    let size_node = graph
        .nodes()
        .find(|n| n.id == "quarry::core::Engine::size")
        .expect("size node kept");
    assert_eq!(size_node.loc.line, 20);
    assert_eq!(size_node.label, "size");

    // Node counts by kind — arithmetic in the fixture README.
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for n in graph.nodes() {
        *counts.entry(format!("{:?}", n.kind)).or_default() += 1;
    }
    let want: BTreeMap<String, usize> = [
        ("Package".to_string(), 1),
        ("Module".to_string(), 2),
        ("File".to_string(), 3),
        ("Variable".to_string(), 3),
        ("Interface".to_string(), 1),
        ("Class".to_string(), 4),
        ("Method".to_string(), 7),
        ("External".to_string(), 5),
    ]
    .into_iter()
    .collect();
    assert_eq!(counts, want);
    assert_eq!(graph.nodes().count(), 26);

    // Edge counts by kind — arithmetic in the fixture README.
    let mut ecounts: BTreeMap<String, usize> = BTreeMap::new();
    for e in graph.edges() {
        *ecounts.entry(format!("{:?}", e.kind)).or_default() += 1;
    }
    let ewant: BTreeMap<String, usize> = [
        ("Contains".to_string(), 20),
        ("Imports".to_string(), 7),
        ("Calls".to_string(), 1),
        ("Reads".to_string(), 2),
        ("Defines".to_string(), 1),
        ("Writes".to_string(), 1),
        ("Inherits".to_string(), 1),
    ]
    .into_iter()
    .collect();
    assert_eq!(ecounts, ewant);
    assert_eq!(graph.edges().count(), 33);

    // The named edges, and one named absence: Circle inherits Shape;
    // Drawable's Protocol heritage is SILENT (the name "Protocol"
    // resolves to no symbol, and unresolvable heritage emits no
    // edge — the interface kind is the carrier, per D2).
    let has_edge = |from: &str, to: &str, kind: EdgeKind| {
        graph
            .edges()
            .any(|e| e.from == from && e.to == to && e.kind == kind)
    };
    assert!(has_edge(
        "quarry::shapes::Circle",
        "quarry::shapes::Shape",
        EdgeKind::Inherits
    ));
    assert!(!graph
        .edges()
        .any(|e| e.from == "quarry::core::Drawable" && e.kind == EdgeKind::Inherits));
    assert!(has_edge(
        "quarry::core::Engine::render",
        "external::shape.draw",
        EdgeKind::Calls
    ));
    assert!(has_edge(
        "quarry::core::Engine::__init__",
        "quarry::core::Engine::size",
        EdgeKind::Writes
    ));
    assert!(has_edge(
        "quarry::core::Engine::__init__",
        "quarry::core::DEFAULT_SIZE",
        EdgeKind::Reads
    ));
    assert!(has_edge(
        "quarry::shapes::Circle::area",
        "external::math",
        EdgeKind::Reads
    ));
}
