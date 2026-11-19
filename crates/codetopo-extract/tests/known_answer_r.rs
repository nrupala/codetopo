// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-r`).
//!
//! Every expectation in this test is derived by hand in the
//! fixture's README.md, which shows the arithmetic. A change that
//! moves a count must move the README in the same PR — that is the
//! point of a known-answer fixture.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codetopo_core::emit::FileSymbols;
use codetopo_core::schema::NodeKind;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-r")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["lib/shapes.R", "core/engine.R", "main.R"] {
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

fn sym<'a>(fs: &'a FileSymbols, id: &str) -> &'a codetopo_core::emit::RawSymbol {
    fs.symbols
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("{id} present"))
}

fn sv(names: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn r_extraction_known_answers() {
    let files = extract_fixture();

    // lib/shapes.R — a constant and a function.
    let shapes = &files["lib/shapes.R"];
    assert_eq!(shapes.module_id, "quarry::lib::shapes");
    assert_eq!(
        sorted_ids(shapes),
        vec!["quarry::lib::shapes::PI", "quarry::lib::shapes::circle_area"]
    );
    let pi = sym(shapes, "quarry::lib::shapes::PI");
    assert_eq!(pi.kind, NodeKind::Variable);
    assert_eq!(pi.line, 1);
    let area = sym(shapes, "quarry::lib::shapes::circle_area");
    assert_eq!(area.kind, NodeKind::Function);
    assert_eq!(area.line, 3);
    assert_eq!(area.defines, sv(&["circle"]));
    assert_eq!(area.refs, sv(&["PI"]));
    assert_eq!(area.decisions, Some(0));
    assert!(shapes.imports.is_empty() && shapes.calls.is_empty() && shapes.heritage.is_empty());

    // core/engine.R — library import, a counter, two functions.
    let engine = &files["core/engine.R"];
    assert_eq!(engine.module_id, "quarry::core::engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::core::engine::clamp_size",
            "quarry::core::engine::render",
            "quarry::core::engine::render_count",
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("external::stats", vec!["stats".to_string()], 1)]
    );
    let clamp = sym(engine, "quarry::core::engine::clamp_size");
    assert_eq!(clamp.line, 5);
    assert_eq!(clamp.defines, sv(&["size"]));
    assert!(clamp.refs.is_empty());
    assert_eq!(clamp.decisions, Some(1));
    let render = sym(engine, "quarry::core::engine::render");
    assert_eq!(render.line, 13);
    assert_eq!(render.defines, sv(&["engine", "render_count"]));
    assert_eq!(render.writes, sv(&["render_count"]));
    assert_eq!(render.refs, sv(&["circle_area"]));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::core::engine::render", "circle_area", 15)]
    );

    // main.R — source import, two variables, two calls.
    let main = &files["main.R"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::main::area", "quarry::main::engine"]
    );
    let imports: Vec<(&str, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::core::engine", 1)]);
    let calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::main", "list", 3),
            ("quarry::main", "list", 3),
            ("quarry::main", "render", 4),
            ("quarry::main", "print", 5),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (7, 2, 5, 0)
    );
}
