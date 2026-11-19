// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-c`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-c")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in [
        "shapes/shape.h",
        "shapes/circle.h",
        "shapes/circle.c",
        "core/engine.h",
        "core/engine.c",
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
fn c_extraction_known_answers() {
    let files = extract_fixture();

    // shapes/shape.h — the struct and a prototype. The
    // function-pointer field is a field, not a symbol.
    let shape = &files["shapes/shape.h"];
    assert_eq!(shape.module_id, "quarry::shapes::shape");
    assert_eq!(
        sorted_ids(shape),
        vec!["quarry::shapes::shape::Shape", "quarry::shapes::shape::shape_area"]
    );
    let shape_sym = sym(shape, "quarry::shapes::shape::Shape");
    assert_eq!(shape_sym.kind, NodeKind::Class);
    assert_eq!(shape_sym.line, 4);
    assert_eq!(shape_sym.signature.as_deref(), Some("struct Shape"));
    let shape_area = sym(shape, "quarry::shapes::shape::shape_area");
    assert_eq!(shape_area.kind, NodeKind::Function);
    assert_eq!(shape_area.line, 9);
    assert_eq!(shape_area.defines, sv(&["shape"]));
    assert_eq!(shape_area.refs, sv(&["Shape", "double"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // shapes/circle.h — quoted include, struct, prototype.
    let circle_h = &files["shapes/circle.h"];
    assert_eq!(circle_h.module_id, "quarry::shapes::circle");
    assert_eq!(
        sorted_ids(circle_h),
        vec!["quarry::shapes::circle::Circle", "quarry::shapes::circle::circle_area"]
    );
    let imports: Vec<(&str, u32)> = circle_h
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::shapes::shape", 4)]);
    let proto = sym(circle_h, "quarry::shapes::circle::circle_area");
    assert_eq!(proto.line, 10);
    assert_eq!(proto.defines, sv(&["circle"]));
    assert_eq!(proto.refs, sv(&["Circle", "double"]));
    assert_eq!(proto.decisions, None);

    // shapes/circle.c — the definition's file: same module id as
    // the header, the constant, the body scan.
    let circle_c = &files["shapes/circle.c"];
    assert_eq!(circle_c.module_id, "quarry::shapes::circle");
    assert_eq!(
        sorted_ids(circle_c),
        vec!["quarry::shapes::circle::PI", "quarry::shapes::circle::circle_area"]
    );
    let imports: Vec<(&str, u32)> = circle_c
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::shapes::circle", 1), ("external::math", 3)]
    );
    let pi = sym(circle_c, "quarry::shapes::circle::PI");
    assert_eq!(pi.kind, NodeKind::Variable);
    assert_eq!(pi.line, 5);
    assert_eq!(pi.signature.as_deref(), Some("static const double PI"));
    let area = sym(circle_c, "quarry::shapes::circle::circle_area");
    assert_eq!(area.line, 7);
    assert_eq!(
        area.signature.as_deref(),
        Some("double circle_area(const struct Circle *circle)")
    );
    assert_eq!(area.decisions, Some(0));
    assert_eq!(area.defines, sv(&["circle"]));
    assert_eq!(area.refs, sv(&["Circle", "PI", "double"]));
    assert!(area.writes.is_empty());

    // core/engine.h — struct, extern variable, two prototypes,
    // a `..` include.
    let engine_h = &files["core/engine.h"];
    assert_eq!(engine_h.module_id, "quarry::core::engine");
    assert_eq!(
        sorted_ids(engine_h),
        vec![
            "quarry::core::engine::Engine",
            "quarry::core::engine::engine_count",
            "quarry::core::engine::engine_render",
            "quarry::core::engine::engine_reset",
        ]
    );
    let imports: Vec<(&str, u32)> = engine_h
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::shapes::circle", 4)]);
    let ext = sym(engine_h, "quarry::core::engine::engine_count");
    assert_eq!(ext.kind, NodeKind::Variable);
    assert_eq!(ext.line, 11);
    assert_eq!(ext.signature.as_deref(), Some("extern int engine_count"));

    // core/engine.c — macro constant, global, static helper, the
    // two definitions, the fixture's only calls.
    let engine_c = &files["core/engine.c"];
    assert_eq!(engine_c.module_id, "quarry::core::engine");
    assert_eq!(
        sorted_ids(engine_c),
        vec![
            "quarry::core::engine::MAX_RENDER_DEPTH",
            "quarry::core::engine::clamp_size",
            "quarry::core::engine::engine_count",
            "quarry::core::engine::engine_render",
            "quarry::core::engine::engine_reset",
        ]
    );
    let imports: Vec<(&str, u32)> = engine_c
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::core::engine", 1), ("external::stddef", 3)]
    );
    assert!(engine_c.imports.iter().all(|i| i.bindings.is_empty()));
    let calls: Vec<(&str, &str, u32)> = engine_c
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::core::engine::engine_render", "clamp_size", 18),
            ("quarry::core::engine::engine_render", "circle_area", 19),
        ]
    );
    let clamp = sym(engine_c, "quarry::core::engine::clamp_size");
    assert_eq!(clamp.line, 9);
    assert_eq!(clamp.decisions, Some(1));
    assert_eq!(clamp.defines, sv(&["size"]));
    assert_eq!(clamp.refs, sv(&["int"]));
    let render = sym(engine_c, "quarry::core::engine::engine_render");
    assert_eq!(render.line, 16);
    assert_eq!(render.defines, sv(&["engine"]));
    assert_eq!(
        render.refs,
        sv(&["Engine", "circle_area", "clamp_size", "double", "engine_count"])
    );
    assert_eq!(render.writes, sv(&["size"]));
    let reset = sym(engine_c, "quarry::core::engine::engine_reset");
    assert_eq!(reset.line, 22);
    assert_eq!(reset.defines, sv(&["engine_count"]));
    assert_eq!(reset.writes, sv(&["engine_count"]));
    assert_eq!(reset.refs, sv(&["void"]));
    let count = sym(engine_c, "quarry::core::engine::engine_count");
    assert_eq!(count.signature.as_deref(), Some("int engine_count"));
    let depth = sym(engine_c, "quarry::core::engine::MAX_RENDER_DEPTH");
    assert_eq!(depth.kind, NodeKind::Variable);
    assert_eq!(depth.line, 5);

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (15, 6, 2, 0)
    );
}
