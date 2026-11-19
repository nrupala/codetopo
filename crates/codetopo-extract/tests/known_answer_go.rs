// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-go`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-go")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["shapes/shape.go", "shapes/circle.go", "core/engine.go", "main.go"] {
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
fn go_extraction_known_answers() {
    let files = extract_fixture();

    // shapes/shape.go — module id is package + directory.
    let shape = &files["shapes/shape.go"];
    assert_eq!(shape.module_id, "quarry::shapes");
    assert_eq!(
        sorted_ids(shape),
        vec!["quarry::shapes::Shape", "quarry::shapes::Shape::Area"]
    );
    let shape_sym = sym(shape, "quarry::shapes::Shape");
    assert_eq!(shape_sym.kind, NodeKind::Interface);
    assert_eq!(shape_sym.line, 3);
    let area = sym(shape, "quarry::shapes::Shape::Area");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 4);
    assert_eq!(area.decisions, None);
    assert_eq!(area.refs, sv(&["float64"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // shapes/circle.go — struct, constructor function, method on
    // the receiver.
    let circle = &files["shapes/circle.go"];
    assert_eq!(circle.module_id, "quarry::shapes");
    assert_eq!(
        sorted_ids(circle),
        vec![
            "quarry::shapes::Circle",
            "quarry::shapes::Circle::Area",
            "quarry::shapes::NewCircle",
        ]
    );
    let circle_sym = sym(circle, "quarry::shapes::Circle");
    assert_eq!(circle_sym.kind, NodeKind::Class);
    assert_eq!(circle_sym.line, 3);
    let new_circle = sym(circle, "quarry::shapes::NewCircle");
    assert_eq!(new_circle.kind, NodeKind::Function);
    assert_eq!(new_circle.line, 7);
    assert_eq!(new_circle.defines, sv(&["radius"]));
    assert_eq!(new_circle.refs, sv(&["Circle", "float64"]));
    let c_area = sym(circle, "quarry::shapes::Circle::Area");
    assert_eq!(c_area.kind, NodeKind::Method);
    assert_eq!(c_area.line, 11);
    assert_eq!(c_area.defines, sv(&["c"]));
    assert_eq!(c_area.refs, sv(&["Circle", "float64"]));
    assert_eq!(c_area.decisions, Some(0));

    // core/engine.go — const, var, struct, method; one import.
    let engine = &files["core/engine.go"];
    assert_eq!(engine.module_id, "quarry::core");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::core::Count",
            "quarry::core::Engine",
            "quarry::core::Engine::Render",
            "quarry::core::MaxRenders",
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::shapes", vec!["shapes".to_string()], 3)]
    );
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(calls, vec![("quarry::core::Engine::Render", "Area", 17)]);
    let render = sym(engine, "quarry::core::Engine::Render");
    assert_eq!(render.line, 14);
    assert_eq!(render.defines, sv(&["e"]));
    assert_eq!(render.refs, sv(&["Engine", "float64"]));
    assert!(render.writes.is_empty());
    let max = sym(engine, "quarry::core::MaxRenders");
    assert_eq!(max.kind, NodeKind::Variable);
    assert_eq!(max.line, 5);

    // main.go — the root package: module id is the package alone.
    let main = &files["main.go"];
    assert_eq!(main.module_id, "quarry");
    assert_eq!(sorted_ids(main), vec!["quarry::main"]);
    let imports: Vec<(&str, Vec<String>, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("external::fmt", vec!["fmt".to_string()], 4),
            ("quarry::core", vec!["core".to_string()], 6),
            ("quarry::shapes", vec!["shapes".to_string()], 7),
        ]
    );
    let calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::main", "NewCircle", 11),
            ("quarry::main", "Render", 12),
            ("quarry::main", "Println", 13),
        ]
    );
    let main_fn = sym(main, "quarry::main");
    assert_eq!(main_fn.line, 10);
    assert_eq!(main_fn.defines, sv(&["area", "engine"]));
    assert_eq!(main_fn.writes, sv(&["area", "engine"]));
    assert_eq!(main_fn.refs, sv(&["Engine", "fmt", "shapes"]));
    assert_eq!(main_fn.decisions, Some(0));

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (10, 4, 4, 0)
    );
}
