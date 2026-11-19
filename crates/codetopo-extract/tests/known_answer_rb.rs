// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-rb`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-rb")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["lib/shape.rb", "lib/circle.rb", "core/engine.rb", "main.rb"] {
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
fn rb_extraction_known_answers() {
    let files = extract_fixture();

    // lib/shape.rb — module, class, stub method.
    let shape = &files["lib/shape.rb"];
    assert_eq!(shape.module_id, "quarry::lib::shape");
    assert_eq!(
        sorted_ids(shape),
        vec![
            "quarry::lib::shape::Quarry",
            "quarry::lib::shape::Quarry::Shape",
            "quarry::lib::shape::Quarry::Shape::area",
        ]
    );
    let area = sym(shape, "quarry::lib::shape::Quarry::Shape::area");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 3);
    assert_eq!(area.refs, sv(&["NotImplementedError", "raise"]));
    assert_eq!(area.decisions, Some(0));
    let calls: Vec<(&str, &str, u32)> = shape
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::lib::shape::Quarry::Shape::area", "raise", 4)]
    );

    // lib/circle.rb — the subclass, heritage, the import.
    let circle = &files["lib/circle.rb"];
    assert_eq!(circle.module_id, "quarry::lib::circle");
    assert_eq!(
        sorted_ids(circle),
        vec![
            "quarry::lib::circle::Quarry",
            "quarry::lib::circle::Quarry::Circle",
            "quarry::lib::circle::Quarry::Circle::area",
            "quarry::lib::circle::Quarry::Circle::initialize",
        ]
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = circle
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Inherits, 4)]);
    let imports: Vec<(&str, u32)> = circle
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::lib::shape", 1)]);
    let calls: Vec<(&str, &str, u32)> = circle
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::lib::circle::Quarry::Circle", "attr_reader", 5)]
    );
    let init = sym(circle, "quarry::lib::circle::Quarry::Circle::initialize");
    assert_eq!(init.line, 7);
    assert_eq!(init.defines, sv(&["radius"]));
    assert_eq!(init.writes, sv(&["@radius"]));
    assert!(init.refs.is_empty());
    let c_area = sym(circle, "quarry::lib::circle::Quarry::Circle::area");
    assert_eq!(c_area.line, 11);
    assert_eq!(c_area.refs, sv(&["@radius"]));

    // core/engine.rb — MAX_RENDERS is a field, not a symbol.
    let engine = &files["core/engine.rb"];
    assert_eq!(engine.module_id, "quarry::core::engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::core::engine::Quarry",
            "quarry::core::engine::Quarry::Engine",
            "quarry::core::engine::Quarry::Engine::initialize",
            "quarry::core::engine::Quarry::Engine::render",
        ]
    );
    let imports: Vec<(&str, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::lib::circle", 1)]);
    let e_init = sym(engine, "quarry::core::engine::Quarry::Engine::initialize");
    assert_eq!(e_init.writes, sv(&["@renders"]));
    assert!(e_init.defines.is_empty() && e_init.refs.is_empty());
    let render = sym(engine, "quarry::core::engine::Quarry::Engine::render");
    assert_eq!(render.line, 11);
    assert_eq!(render.defines, sv(&["wheel"]));
    assert_eq!(render.refs, sv(&["@renders"]));
    assert!(render.writes.is_empty());
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::core::engine::Quarry::Engine::render", "area", 13)]
    );

    // main.rb — top-level variables and calls in walk order.
    let main = &files["main.rb"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::main::engine", "quarry::main::wheel"]
    );
    let imports: Vec<(&str, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.line))
        .collect();
    assert_eq!(imports, vec![("quarry::core::engine", 1)]);
    let calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::main", "new", 3),
            ("quarry::main", "new", 4),
            ("quarry::main", "puts", 5),
            ("quarry::main", "render", 5),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (13, 3, 7, 1)
    );
}
