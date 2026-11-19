// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-cpp`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-cpp")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in [
        "shapes/Shape.hpp",
        "shapes/Circle.hpp",
        "shapes/Circle.cpp",
        "core/Engine.hpp",
        "core/Engine.cpp",
    ] {
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
fn cpp_extraction_known_answers() {
    let files = extract_fixture();

    // shapes/Shape.hpp — the interface-shaped class.
    let shape = &files["shapes/Shape.hpp"];
    assert_eq!(shape.module_id, "quarry::shapes::Shape");
    assert_eq!(
        sorted_ids(shape),
        vec![
            "quarry::shapes::Shape::quarry::Shape",
            "quarry::shapes::Shape::quarry::Shape::area",
        ]
    );
    let shape_sym = sym(shape, "quarry::shapes::Shape::quarry::Shape");
    assert_eq!(shape_sym.kind, NodeKind::Class);
    assert_eq!(shape_sym.line, 5);
    let area = sym(shape, "quarry::shapes::Shape::quarry::Shape::area");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 7);
    assert_eq!(area.decisions, None);
    assert_eq!(area.refs, sv(&["double"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // shapes/Circle.hpp — heritage, declarations, one include.
    let circle_h = &files["shapes/Circle.hpp"];
    assert_eq!(circle_h.module_id, "quarry::shapes::Circle");
    assert_eq!(
        sorted_ids(circle_h),
        vec![
            "quarry::shapes::Circle::quarry::Circle",
            "quarry::shapes::Circle::quarry::Circle::Circle",
            "quarry::shapes::Circle::quarry::Circle::area",
        ]
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = circle_h
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Inherits, 7)]);
    let imports: Vec<(&str, u32)> = circle_h
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::shapes::Shape", 3)]);
    let ctor_decl = sym(circle_h, "quarry::shapes::Circle::quarry::Circle::Circle");
    assert_eq!(ctor_decl.kind, NodeKind::Method);
    assert_eq!(ctor_decl.line, 9);
    assert_eq!(ctor_decl.defines, sv(&["radius"]));
    assert_eq!(ctor_decl.refs, sv(&["double"]));

    // shapes/Circle.cpp — the out-of-line definitions compose
    // the same ids as the declarations.
    let circle_c = &files["shapes/Circle.cpp"];
    assert_eq!(circle_c.module_id, "quarry::shapes::Circle");
    assert_eq!(
        sorted_ids(circle_c),
        vec![
            "quarry::shapes::Circle::quarry::Circle::Circle",
            "quarry::shapes::Circle::quarry::Circle::area",
        ]
    );
    let imports: Vec<(&str, u32)> = circle_c
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::shapes::Circle", 1)]);
    let ctor = sym(circle_c, "quarry::shapes::Circle::quarry::Circle::Circle");
    assert_eq!(ctor.line, 5);
    assert_eq!(ctor.defines, sv(&["radius"]));
    assert_eq!(ctor.writes, sv(&["radius_"]));
    assert_eq!(ctor.refs, sv(&["double"]));
    let c_area = sym(circle_c, "quarry::shapes::Circle::quarry::Circle::area");
    assert_eq!(c_area.line, 7);
    assert_eq!(c_area.refs, sv(&["double", "radius_"]));
    assert_eq!(c_area.decisions, Some(0));

    // core/Engine.hpp — declarations and a .. include.
    let engine_h = &files["core/Engine.hpp"];
    assert_eq!(engine_h.module_id, "quarry::core::Engine");
    assert_eq!(
        sorted_ids(engine_h),
        vec![
            "quarry::core::Engine::quarry::Engine",
            "quarry::core::Engine::quarry::Engine::render",
            "quarry::core::Engine::quarry::Engine::renderCount",
        ]
    );
    let imports: Vec<(&str, u32)> = engine_h
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::shapes::Circle", 3)]);
    let render_decl = sym(engine_h, "quarry::core::Engine::quarry::Engine::render");
    assert_eq!(render_decl.line, 9);
    assert_eq!(render_decl.defines, sv(&["wheel"]));
    assert_eq!(render_decl.refs, sv(&["Circle", "double"]));

    // core/Engine.cpp — definitions, the fixture's one call.
    let engine_c = &files["core/Engine.cpp"];
    assert_eq!(engine_c.module_id, "quarry::core::Engine");
    assert_eq!(
        sorted_ids(engine_c),
        vec![
            "quarry::core::Engine::quarry::Engine::render",
            "quarry::core::Engine::quarry::Engine::renderCount",
        ]
    );
    let imports: Vec<(&str, u32)> = engine_c
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::core::Engine", 1), ("external::cstddef", 3)]
    );
    let calls: Vec<(&str, &str, u32)> = engine_c
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::core::Engine::quarry::Engine::render", "wheel.area", 9)]
    );
    let render = sym(engine_c, "quarry::core::Engine::quarry::Engine::render");
    assert_eq!(render.line, 7);
    assert_eq!(render.defines, sv(&["wheel"]));
    assert_eq!(render.refs, sv(&["Circle", "double", "renders_"]));
    assert!(render.writes.is_empty());
    let count = sym(engine_c, "quarry::core::Engine::quarry::Engine::renderCount");
    assert_eq!(count.line, 12);
    assert_eq!(count.refs, sv(&["int", "renders_"]));

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (12, 5, 1, 1)
    );
}
