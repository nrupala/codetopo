// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-lua`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-lua")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["shapes.lua", "core/engine.lua", "main.lua"] {
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
fn lua_extraction_known_answers() {
    let files = extract_fixture();

    // shapes.lua — two table classes, three methods, one heritage.
    let shapes = &files["shapes.lua"];
    assert_eq!(shapes.module_id, "quarry::shapes");
    assert_eq!(
        sorted_ids(shapes),
        vec![
            "quarry::shapes::Circle",
            "quarry::shapes::Circle::area",
            "quarry::shapes::Shape",
            "quarry::shapes::Shape::area",
            "quarry::shapes::Shape::new",
        ]
    );
    let shape_ty = sym(shapes, "quarry::shapes::Shape");
    assert_eq!(shape_ty.kind, NodeKind::Class);
    assert_eq!(shape_ty.line, 1);
    assert_eq!(shape_ty.signature.as_deref(), Some("local Shape"));
    let new = sym(shapes, "quarry::shapes::Shape::new");
    assert_eq!(new.kind, NodeKind::Method);
    assert_eq!(new.line, 4);
    assert_eq!(
        new.signature.as_deref(),
        Some("function Shape.new(radius)")
    );
    assert_eq!(new.defines, sv(&["radius", "self"]));
    assert_eq!(new.refs, sv(&["Shape", "setmetatable"]));
    assert_eq!(new.writes, sv(&["radius", "self"]));
    assert_eq!(new.decisions, Some(0));
    let s_area = sym(shapes, "quarry::shapes::Shape::area");
    assert_eq!(s_area.kind, NodeKind::Method);
    assert_eq!(s_area.line, 10);
    assert_eq!(s_area.refs, sv(&["self"]));
    assert_eq!(s_area.decisions, Some(0));
    let circle = sym(shapes, "quarry::shapes::Circle");
    assert_eq!(circle.kind, NodeKind::Class);
    assert_eq!(circle.line, 14);
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = shapes
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Inherits, 15)]);
    let c_area = sym(shapes, "quarry::shapes::Circle::area");
    assert_eq!(c_area.kind, NodeKind::Method);
    assert_eq!(c_area.line, 17);
    assert_eq!(c_area.refs, sv(&["Shape", "self"]));
    assert_eq!(c_area.decisions, Some(1));
    assert!(shapes.imports.is_empty());
    let calls: Vec<(&str, &str, u32)> = shapes
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::shapes::Shape::new", "setmetatable", 5),
            ("quarry::shapes", "setmetatable", 15),
            ("quarry::shapes::Circle::area", "Shape.area", 19),
        ]
    );

    // core/engine.lua — one require, one class, two methods.
    let engine = &files["core/engine.lua"];
    assert_eq!(engine.module_id, "quarry::core::engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::core::engine::Engine",
            "quarry::core::engine::Engine::new",
            "quarry::core::engine::Engine::render",
            "quarry::core::engine::Shape",
            "quarry::core::engine::shapes",
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::shapes", vec!["shapes".to_string()], 1)]
    );
    let shapes_var = sym(engine, "quarry::core::engine::shapes");
    assert_eq!(shapes_var.kind, NodeKind::Variable);
    assert_eq!(shapes_var.refs, sv(&["require"]));
    let shape_var = sym(engine, "quarry::core::engine::Shape");
    assert_eq!(shape_var.kind, NodeKind::Variable);
    assert_eq!(shape_var.refs, sv(&["shapes"]));
    let e_new = sym(engine, "quarry::core::engine::Engine::new");
    assert_eq!(e_new.line, 8);
    assert_eq!(e_new.defines, sv(&["self"]));
    assert_eq!(e_new.refs, sv(&["Engine", "setmetatable"]));
    assert_eq!(e_new.writes, sv(&["self"]));
    let render = sym(engine, "quarry::core::engine::Engine::render");
    assert_eq!(render.kind, NodeKind::Method);
    assert_eq!(render.line, 13);
    assert_eq!(render.defines, sv(&["wheel"]));
    assert_eq!(render.refs, sv(&["Engine"]));
    assert_eq!(render.writes, sv(&["renders"]));
    assert_eq!(render.decisions, Some(0));
    assert!(engine.heritage.is_empty());
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::core::engine::Engine::new", "setmetatable", 9),
            ("quarry::core::engine::Engine::render", "wheel:area", 15),
        ]
    );

    // main.lua — two requires, four variables, one local function.
    let main = &files["main.lua"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(
        sorted_ids(main),
        vec![
            "quarry::main::bump",
            "quarry::main::engine",
            "quarry::main::shapes",
            "quarry::main::sharedCount",
            "quarry::main::state",
            "quarry::main::wheel",
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("quarry::core::engine", vec!["engine".to_string()], 1),
            ("quarry::shapes", vec!["shapes".to_string()], 2),
        ]
    );
    let bump = sym(main, "quarry::main::bump");
    assert_eq!(bump.kind, NodeKind::Function);
    assert_eq!(bump.line, 11);
    assert_eq!(bump.defines, sv(&["sharedCount"]));
    assert_eq!(bump.refs, sv(&[]));
    assert_eq!(bump.writes, sv(&["sharedCount"]));
    let state = sym(main, "quarry::main::state");
    assert_eq!(state.refs, sv(&["engine"]));
    let calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::main", "engine.new", 7),
            ("quarry::main", "print", 8),
            ("quarry::main", "state:render", 8),
            ("quarry::main", "print", 9),
            ("quarry::main", "print", 16),
            ("quarry::main", "bump", 16),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (16, 3, 11, 1)
    );
}

#[test]
fn lua_deep_concat_chain_does_not_overflow() {
    // A generated file whose value is one expression nested tens of
    // thousands of levels deep — a string blob written as
    // `"a" .. "a" .. …` — must extract without overflowing: the
    // walker and its scans run on explicit heap stacks (the E-1
    // lesson). 50,000 segments verified against
    // tree-sitter-lua 0.5.0 (its parser survives the full chain,
    // unlike the PHP grammar's at this size).
    let chain = vec!["\"a\""; 50_000].join(" .. ");
    let src = format!("local s = {chain}\n");
    let fs = codetopo_extract::extract_file("quarry", Path::new("deep.lua"), &src)
        .expect("deep chain extracts");
    assert_eq!(fs.symbols.len(), 1);
    assert_eq!(fs.symbols[0].label, "s");
}
