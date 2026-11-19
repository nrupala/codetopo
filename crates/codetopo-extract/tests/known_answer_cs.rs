// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-cs`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-cs")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["Shapes/Shape.cs", "Shapes/Circle.cs", "Core/Engine.cs", "Program.cs"] {
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
fn cs_extraction_known_answers() {
    let files = extract_fixture();

    // Shapes/Shape.cs — the interface, namespaced.
    let shape = &files["Shapes/Shape.cs"];
    assert_eq!(shape.module_id, "quarry::Shapes::Shape");
    assert_eq!(
        sorted_ids(shape),
        vec![
            "quarry::Shapes::Shape::Quarry::Shapes::Shape",
            "quarry::Shapes::Shape::Quarry::Shapes::Shape::Area",
        ]
    );
    let shape_sym = sym(shape, "quarry::Shapes::Shape::Quarry::Shapes::Shape");
    assert_eq!(shape_sym.kind, NodeKind::Interface);
    assert_eq!(shape_sym.line, 3);
    let area = sym(shape, "quarry::Shapes::Shape::Quarry::Shapes::Shape::Area");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 5);
    assert_eq!(area.decisions, None);
    assert_eq!(area.refs, sv(&["double"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // Shapes/Circle.cs — heritage, constructor, method.
    let circle = &files["Shapes/Circle.cs"];
    assert_eq!(circle.module_id, "quarry::Shapes::Circle");
    assert_eq!(
        sorted_ids(circle),
        vec![
            "quarry::Shapes::Circle::Quarry::Shapes::Circle",
            "quarry::Shapes::Circle::Quarry::Shapes::Circle::Area",
            "quarry::Shapes::Circle::Quarry::Shapes::Circle::Circle",
        ]
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = circle
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Inherits, 5)]);
    let imports: Vec<(&str, u32)> = circle
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("external::System", 1)]);
    assert!(circle.imports.iter().all(|i| i.bindings.is_empty()));
    let ctor = sym(circle, "quarry::Shapes::Circle::Quarry::Shapes::Circle::Circle");
    assert_eq!(ctor.kind, NodeKind::Method);
    assert_eq!(ctor.line, 9);
    assert_eq!(ctor.defines, sv(&["Radius", "radius"]));
    assert_eq!(ctor.writes, sv(&["Radius"]));
    assert_eq!(ctor.refs, sv(&["double"]));
    let c_area = sym(circle, "quarry::Shapes::Circle::Quarry::Shapes::Circle::Area");
    assert_eq!(c_area.line, 14);
    assert_eq!(c_area.refs, sv(&["Math", "Radius", "double"]));
    assert_eq!(c_area.decisions, Some(0));

    // Core/Engine.cs — two usings, the render method.
    let engine = &files["Core/Engine.cs"];
    assert_eq!(engine.module_id, "quarry::Core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::Core::Engine::Quarry::Core::Engine",
            "quarry::Core::Engine::Quarry::Core::Engine::Render",
            "quarry::Core::Engine::Quarry::Core::Engine::RenderCount",
        ]
    );
    let imports: Vec<(&str, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("external::System", 1), ("quarry::Shapes", 2)]
    );
    let render = sym(engine, "quarry::Core::Engine::Quarry::Core::Engine::Render");
    assert_eq!(render.line, 10);
    assert_eq!(render.defines, sv(&["wheel"]));
    assert_eq!(render.refs, sv(&["Area", "Circle", "_renders", "double"]));
    assert!(render.writes.is_empty());
    let count = sym(engine, "quarry::Core::Engine::Quarry::Core::Engine::RenderCount");
    assert_eq!(count.line, 16);
    assert_eq!(count.refs, sv(&["_renders", "int"]));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::Core::Engine::Quarry::Core::Engine::Render", "wheel.Area", 13)]
    );

    // Program.cs — top-level statements.
    let prog = &files["Program.cs"];
    assert_eq!(prog.module_id, "quarry::Program");
    assert_eq!(
        sorted_ids(prog),
        vec!["quarry::Program::engine", "quarry::Program::wheel"]
    );
    let imports: Vec<(&str, u32)> = prog
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("external::System", 1),
            ("quarry::Core", 2),
            ("quarry::Shapes", 3),
        ]
    );
    let calls: Vec<(&str, &str, u32)> = prog
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::Program", "Engine", 5),
            ("quarry::Program", "Circle", 6),
            ("quarry::Program", "Console.WriteLine", 7),
            ("quarry::Program", "engine.Render", 7),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (10, 6, 5, 1)
    );
}
