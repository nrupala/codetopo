// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-swift`).
//!
//! Every expectation in this test is derived by hand in the
//! fixture's README.md, which shows the arithmetic. A change that
//! moves a count must move the README in the same PR — that is the
//! point of a known-answer fixture.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use codetopo_core::emit::FileSymbols;
use codetopo_core::schema::{EdgeKind, NodeKind};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-swift")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["Shapes/Shape.swift", "Shapes/Circle.swift", "Core/Engine.swift", "main.swift"] {
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
fn swift_extraction_known_answers() {
    let files = extract_fixture();

    // Shapes/Shape.swift — the protocol.
    let shape = &files["Shapes/Shape.swift"];
    assert_eq!(shape.module_id, "quarry::Shapes::Shape");
    assert_eq!(
        sorted_ids(shape),
        vec!["quarry::Shapes::Shape::Shape", "quarry::Shapes::Shape::Shape::area"]
    );
    let shape_sym = sym(shape, "quarry::Shapes::Shape::Shape");
    assert_eq!(shape_sym.kind, NodeKind::Interface);
    assert_eq!(shape_sym.line, 1);
    let area = sym(shape, "quarry::Shapes::Shape::Shape::area");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 2);
    assert_eq!(area.decisions, None);
    assert_eq!(area.refs, sv(&["Double"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // Shapes/Circle.swift — the conforming struct.
    let circle = &files["Shapes/Circle.swift"];
    assert_eq!(circle.module_id, "quarry::Shapes::Circle");
    assert_eq!(
        sorted_ids(circle),
        vec!["quarry::Shapes::Circle::Circle", "quarry::Shapes::Circle::Circle::area"]
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = circle
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Inherits, 1)]);
    let c_area = sym(circle, "quarry::Shapes::Circle::Circle::area");
    assert_eq!(c_area.line, 4);
    assert_eq!(c_area.refs, sv(&["Double", "radius"]));
    assert_eq!(c_area.decisions, Some(0));

    // Core/Engine.swift — the class, the import, the call.
    let engine = &files["Core/Engine.swift"];
    assert_eq!(engine.module_id, "quarry::Core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec!["quarry::Core::Engine::Engine", "quarry::Core::Engine::Engine::render"]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("external::Foundation", vec!["Foundation".to_string()], 1)]
    );
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::Core::Engine::Engine::render", "wheel.area", 8)]
    );
    let render = sym(engine, "quarry::Core::Engine::Engine::render");
    assert_eq!(render.line, 6);
    assert_eq!(render.defines, sv(&["wheel"]));
    assert_eq!(render.refs, sv(&["Circle", "Double", "renders"]));
    assert!(render.writes.is_empty());

    // main.swift — top-level variables and calls.
    let main = &files["main.swift"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::main::area", "quarry::main::engine", "quarry::main::wheel"]
    );
    let calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::main", "Engine", 1),
            ("quarry::main", "Circle", 2),
            ("quarry::main", "engine.render", 3),
            ("quarry::main", "print", 4),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (9, 1, 5, 1)
    );
}
