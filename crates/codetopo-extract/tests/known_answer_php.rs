// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-php`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-php")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["Shapes/Shape.php", "Shapes/Circle.php", "Core/Engine.php", "main.php"] {
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
fn php_extraction_known_answers() {
    let files = extract_fixture();

    // Shapes/Shape.php — the interface, bracketless namespace.
    let shape = &files["Shapes/Shape.php"];
    assert_eq!(shape.module_id, "quarry::Shapes::Shape");
    assert_eq!(
        sorted_ids(shape),
        vec![
            "quarry::Shapes::Shape::Quarry::Shapes::Shape",
            "quarry::Shapes::Shape::Quarry::Shapes::Shape::area",
        ]
    );
    let shape_sym = sym(shape, "quarry::Shapes::Shape::Quarry::Shapes::Shape");
    assert_eq!(shape_sym.kind, NodeKind::Interface);
    assert_eq!(shape_sym.line, 5);
    let area = sym(shape, "quarry::Shapes::Shape::Quarry::Shapes::Shape::area");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 7);
    assert_eq!(area.decisions, None);
    assert_eq!(area.refs, sv(&["float"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // Shapes/Circle.php — implements heritage, constructor.
    let circle = &files["Shapes/Circle.php"];
    assert_eq!(circle.module_id, "quarry::Shapes::Circle");
    assert_eq!(
        sorted_ids(circle),
        vec![
            "quarry::Shapes::Circle::Quarry::Shapes::Circle",
            "quarry::Shapes::Circle::Quarry::Shapes::Circle::__construct",
            "quarry::Shapes::Circle::Quarry::Shapes::Circle::area",
        ]
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = circle
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Implements, 5)]);
    let ctor = sym(circle, "quarry::Shapes::Circle::Quarry::Shapes::Circle::__construct");
    assert_eq!(ctor.line, 9);
    assert_eq!(ctor.defines, sv(&["$radius"]));
    assert_eq!(ctor.writes, sv(&["radius"]));
    assert_eq!(ctor.refs, sv(&["float"]));
    let c_area = sym(circle, "quarry::Shapes::Circle::Quarry::Shapes::Circle::area");
    assert_eq!(c_area.line, 14);
    assert_eq!(c_area.refs, sv(&["float"]));
    assert_eq!(c_area.decisions, Some(0));

    // Core/Engine.php — the use import, the render method.
    let engine = &files["Core/Engine.php"];
    assert_eq!(engine.module_id, "quarry::Core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::Core::Engine::Quarry::Core::Engine",
            "quarry::Core::Engine::Quarry::Core::Engine::render",
            "quarry::Core::Engine::Quarry::Core::Engine::renderCount",
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::Shapes::Circle", vec!["Circle".to_string()], 5)]
    );
    let render = sym(engine, "quarry::Core::Engine::Quarry::Core::Engine::render");
    assert_eq!(render.line, 11);
    assert_eq!(render.defines, sv(&["$wheel"]));
    assert_eq!(render.refs, sv(&["Circle", "area", "float"]));
    assert!(render.writes.is_empty());
    let count = sym(engine, "quarry::Core::Engine::Quarry::Core::Engine::renderCount");
    assert_eq!(count.line, 17);
    assert_eq!(count.refs, sv(&["int"]));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::Core::Engine::Quarry::Core::Engine::render", "area", 14)]
    );

    // main.php — uses, a require, top-level variables, calls.
    let main = &files["main.php"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::main::$engine", "quarry::main::$wheel"]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("quarry::Core::Engine", vec!["Engine".to_string()], 3),
            ("quarry::Shapes::Circle", vec!["Circle".to_string()], 4),
            ("quarry::Core::Engine", vec![], 6),
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
            ("quarry::main", "Engine", 8),
            ("quarry::main", "Circle", 9),
            ("quarry::main", "render", 10),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (10, 4, 4, 1)
    );
}
