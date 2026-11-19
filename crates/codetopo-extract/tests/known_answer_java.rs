// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-java`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-java")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["shapes/Shape.java", "shapes/Circle.java", "core/Engine.java", "Main.java"] {
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
fn java_extraction_known_answers() {
    let files = extract_fixture();

    // shapes/Shape.java — module id from the package declaration.
    let shape = &files["shapes/Shape.java"];
    assert_eq!(shape.module_id, "quarry::shapes::Shape");
    assert_eq!(
        sorted_ids(shape),
        vec!["quarry::shapes::Shape::Shape", "quarry::shapes::Shape::Shape::area"]
    );
    let shape_sym = sym(shape, "quarry::shapes::Shape::Shape");
    assert_eq!(shape_sym.kind, NodeKind::Interface);
    assert_eq!(shape_sym.line, 3);
    let area = sym(shape, "quarry::shapes::Shape::Shape::area");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 4);
    assert_eq!(area.decisions, None);
    assert_eq!(area.refs, sv(&["double"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // shapes/Circle.java — the implementor, its constructor, the
    // one heritage row of the fixture.
    let circle = &files["shapes/Circle.java"];
    assert_eq!(circle.module_id, "quarry::shapes::Circle");
    assert_eq!(
        sorted_ids(circle),
        vec![
            "quarry::shapes::Circle::Circle",
            "quarry::shapes::Circle::Circle::Circle",
            "quarry::shapes::Circle::Circle::area",
        ]
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = circle
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Implements, 3)]);
    let ctor = sym(circle, "quarry::shapes::Circle::Circle::Circle");
    assert_eq!(ctor.kind, NodeKind::Method);
    assert_eq!(ctor.line, 6);
    assert_eq!(ctor.defines, sv(&["radius"]));
    assert_eq!(ctor.writes, sv(&["radius"]));
    assert_eq!(ctor.refs, sv(&["double"]));
    let c_area = sym(circle, "quarry::shapes::Circle::Circle::area");
    assert_eq!(c_area.line, 10);
    assert_eq!(c_area.refs, sv(&["double", "radius"]));
    assert_eq!(c_area.decisions, Some(0));

    // core/Engine.java — one internal and one external import,
    // the cross-type call.
    let engine = &files["core/Engine.java"];
    assert_eq!(engine.module_id, "quarry::core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::core::Engine::Engine",
            "quarry::core::Engine::Engine::render",
            "quarry::core::Engine::Engine::renderCount",
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("external::java", vec!["List".to_string()], 3),
            ("quarry::shapes::Circle", vec!["Circle".to_string()], 5),
        ]
    );
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::core::Engine::Engine::render", "area", 12)]
    );
    let render = sym(engine, "quarry::core::Engine::Engine::render");
    assert_eq!(render.line, 10);
    assert_eq!(render.defines, sv(&["wheel"]));
    assert_eq!(render.refs, sv(&["Circle", "area", "double", "renders"]));
    assert!(render.writes.is_empty());
    let count = sym(engine, "quarry::core::Engine::Engine::renderCount");
    assert_eq!(count.line, 15);
    assert_eq!(count.refs, sv(&["int", "renders"]));

    // Main.java — construction and invocation calls.
    let main = &files["Main.java"];
    assert_eq!(main.module_id, "quarry::Main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::Main::Main", "quarry::Main::Main::main"]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("quarry::core::Engine", vec!["Engine".to_string()], 3),
            ("quarry::shapes::Circle", vec!["Circle".to_string()], 4),
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
            ("quarry::Main::Main::main", "Engine", 8),
            ("quarry::Main::Main::main", "Circle", 9),
            ("quarry::Main::Main::main", "render", 10),
            ("quarry::Main::Main::main", "println", 11),
        ]
    );
    let main_fn = sym(main, "quarry::Main::Main::main");
    assert_eq!(main_fn.line, 7);
    assert_eq!(main_fn.defines, sv(&["args", "area", "engine", "wheel"]));
    assert_eq!(main_fn.writes, sv(&["area", "engine", "wheel"]));
    assert_eq!(
        main_fn.refs,
        sv(&["Circle", "Engine", "String", "System", "double", "println", "render", "void"])
    );
    assert_eq!(main_fn.decisions, Some(0));

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (10, 4, 5, 1)
    );
}
