// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-jl`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-jl")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["Shapes.jl", "Core/Engine.jl", "main.jl"] {
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
fn jl_extraction_known_answers() {
    let files = extract_fixture();

    // Shapes.jl — module, abstract type, struct, short-form fn.
    let shapes = &files["Shapes.jl"];
    assert_eq!(shapes.module_id, "quarry::Shapes");
    assert_eq!(
        sorted_ids(shapes),
        vec![
            "quarry::Shapes::Shapes",
            "quarry::Shapes::Shapes::Circle",
            "quarry::Shapes::Shapes::Shape",
            "quarry::Shapes::Shapes::area",
        ]
    );
    let shape_mod = sym(shapes, "quarry::Shapes::Shapes");
    assert_eq!(shape_mod.kind, NodeKind::Class);
    assert_eq!(shape_mod.line, 1);
    assert_eq!(shape_mod.signature.as_deref(), Some("module Shapes"));
    let shape_ty = sym(shapes, "quarry::Shapes::Shapes::Shape");
    assert_eq!(shape_ty.kind, NodeKind::Interface);
    assert_eq!(shape_ty.line, 3);
    let circle = sym(shapes, "quarry::Shapes::Shapes::Circle");
    assert_eq!(circle.kind, NodeKind::Class);
    assert_eq!(circle.line, 5);
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = shapes
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Inherits, 5)]);
    let area = sym(shapes, "quarry::Shapes::Shapes::area");
    assert_eq!(area.kind, NodeKind::Function);
    assert_eq!(area.line, 9);
    assert_eq!(area.defines, sv(&["c"]));
    assert_eq!(area.refs, sv(&["Circle"]));
    assert_eq!(area.decisions, Some(0));

    // Core/Engine.jl — module, mutable struct, long-form fn.
    let engine = &files["Core/Engine.jl"];
    assert_eq!(engine.module_id, "quarry::Core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::Core::Engine::Engine",
            "quarry::Core::Engine::Engine::EngineState",
            "quarry::Core::Engine::Engine::render",
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::Shapes", vec!["Circle".to_string()], 3)]
    );
    let render = sym(engine, "quarry::Core::Engine::Engine::render");
    assert_eq!(render.line, 9);
    assert_eq!(render.defines, sv(&["state", "wheel"]));
    assert_eq!(render.refs, sv(&["Circle", "EngineState", "area"]));
    assert!(render.writes.is_empty());
    assert_eq!(render.decisions, Some(0));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::Core::Engine::Engine::render", "area", 11)]
    );

    // main.jl — two selected imports, two variables, four calls.
    let main = &files["main.jl"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::main::state", "quarry::main::wheel"]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("quarry::Shapes", vec!["Circle".to_string()], 1),
            ("quarry::Core", vec!["Engine".to_string()], 2),
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
            ("quarry::main", "Engine.EngineState", 4),
            ("quarry::main", "Circle", 5),
            ("quarry::main", "println", 6),
            ("quarry::main", "Engine.render", 6),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (9, 3, 5, 1)
    );
}
