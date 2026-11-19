// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-sh`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-sh")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["lib/util.sh", "lib/shapes.sh", "core/engine.sh", "main.sh"] {
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
fn sh_extraction_known_answers() {
    let files = extract_fixture();

    // lib/util.sh — a variable and a function.
    let util = &files["lib/util.sh"];
    assert_eq!(util.module_id, "quarry::lib::util");
    assert_eq!(
        sorted_ids(util),
        vec!["quarry::lib::util::UTIL_VERSION", "quarry::lib::util::log_info"]
    );
    let version = sym(util, "quarry::lib::util::UTIL_VERSION");
    assert_eq!(version.kind, NodeKind::Variable);
    assert_eq!(version.line, 4);
    let log = sym(util, "quarry::lib::util::log_info");
    assert_eq!(log.kind, NodeKind::Function);
    assert_eq!(log.line, 6);
    assert_eq!(log.refs, sv(&["echo"]));
    assert!(log.defines.is_empty() && log.writes.is_empty());
    let calls: Vec<(&str, &str, u32)> = util
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(calls, vec![("quarry::lib::util::log_info", "echo", 7)]);

    // lib/shapes.sh — a pipeline of two calls, a local.
    let shapes = &files["lib/shapes.sh"];
    assert_eq!(shapes.module_id, "quarry::lib::shapes");
    assert_eq!(sorted_ids(shapes), vec!["quarry::lib::shapes::shape_area"]);
    let area = sym(shapes, "quarry::lib::shapes::shape_area");
    assert_eq!(area.line, 4);
    assert_eq!(area.defines, sv(&["radius"]));
    assert_eq!(area.writes, sv(&["radius"]));
    assert_eq!(area.refs, sv(&["bc", "echo"]));
    let calls: Vec<(&str, &str, u32)> = shapes
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::lib::shapes::shape_area", "echo", 6),
            ("quarry::lib::shapes::shape_area", "bc", 6),
        ]
    );

    // core/engine.sh — two source imports, two variables, two
    // functions.
    let engine = &files["core/engine.sh"];
    assert_eq!(engine.module_id, "quarry::core::engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::core::engine::MAX_RENDERS",
            "quarry::core::engine::clamp_size",
            "quarry::core::engine::render",
            "quarry::core::engine::render_count",
        ]
    );
    let imports: Vec<(&str, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::lib::util", 4), ("quarry::lib::shapes", 5)]
    );
    assert!(engine.imports.iter().all(|i| i.bindings.is_empty()));
    let clamp = sym(engine, "quarry::core::engine::clamp_size");
    assert_eq!(clamp.line, 10);
    assert_eq!(clamp.decisions, Some(1));
    assert_eq!(clamp.refs, sv(&["echo"]));
    let render = sym(engine, "quarry::core::engine::render");
    assert_eq!(render.line, 18);
    assert_eq!(render.defines, sv(&["render_count"]));
    assert_eq!(render.writes, sv(&["render_count"]));
    assert_eq!(render.refs, sv(&["log_info", "shape_area"]));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::core::engine::clamp_size", "echo", 12),
            ("quarry::core::engine::clamp_size", "echo", 14),
            ("quarry::core::engine::render", "log_info", 20),
            ("quarry::core::engine::render", "shape_area", 21),
        ]
    );

    // main.sh — one import, one function, a top-level call.
    let main = &files["main.sh"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(sorted_ids(main), vec!["quarry::main::main"]);
    let imports: Vec<(&str, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::core::engine", 4)]);
    let main_fn = sym(main, "quarry::main::main");
    assert_eq!(main_fn.line, 6);
    assert_eq!(main_fn.defines, sv(&["size"]));
    assert_eq!(main_fn.writes, sv(&["size"]));
    assert_eq!(main_fn.refs, sv(&["clamp_size", "render"]));
    let calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::main::main", "clamp_size", 8),
            ("quarry::main::main", "render", 9),
            ("quarry::main", "main", 12),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (8, 3, 10, 0)
    );
}
