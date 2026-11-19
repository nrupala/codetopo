// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-kt`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-kt")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["shapes/Shape.kt", "shapes/Circle.kt", "core/Engine.kt", "Main.kt"] {
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

fn sv(names: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn kt_extraction_known_answers() {
    let files = extract_fixture();

    // shapes/Shape.kt — the interface and its stub method.
    let shape = &files["shapes/Shape.kt"];
    assert_eq!(shape.module_id, "quarry::shapes::Shape");
    assert_eq!(
        sorted_ids(shape),
        vec![
            "quarry::shapes::Shape::Shape",
            "quarry::shapes::Shape::Shape::area",
        ]
    );
    let shape_sym = shape
        .symbols
        .iter()
        .find(|s| s.id == "quarry::shapes::Shape::Shape")
        .expect("Shape present");
    assert_eq!(shape_sym.kind, NodeKind::Interface);
    assert_eq!(shape_sym.line, 3);
    let area = shape
        .symbols
        .iter()
        .find(|s| s.id == "quarry::shapes::Shape::Shape::area")
        .expect("area present");
    assert_eq!(area.kind, NodeKind::Method);
    assert_eq!(area.line, 4);
    assert_eq!(area.refs, sv(&["Double"]));
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // shapes/Circle.kt — the implementor.
    let circle = &files["shapes/Circle.kt"];
    assert_eq!(circle.module_id, "quarry::shapes::Circle");
    assert_eq!(
        sorted_ids(circle),
        vec![
            "quarry::shapes::Circle::Circle",
            "quarry::shapes::Circle::Circle::area",
        ]
    );
    let circle_imports: Vec<(&str, Vec<String>)> = circle
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
        .collect();
    assert_eq!(circle_imports, vec![("external::kotlin", sv(&["PI"]))]);
    let heritage: Vec<(&str, &str, u32)> = circle
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", 5)]);
    let carea = circle
        .symbols
        .iter()
        .find(|s| s.id == "quarry::shapes::Circle::Circle::area")
        .expect("Circle::area present");
    assert_eq!(carea.refs, sv(&["Double", "PI", "radius"]));
    assert!(carea.defines.is_empty() && carea.writes.is_empty());

    // core/Engine.kt — variables, the class, the builder function.
    let engine = &files["core/Engine.kt"];
    assert_eq!(engine.module_id, "quarry::core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::core::Engine::DEFAULT_SIZE",
            "quarry::core::Engine::Engine",
            "quarry::core::Engine::Engine::render",
            "quarry::core::Engine::buildEngine",
            "quarry::core::Engine::counter",
        ]
    );
    let engine_imports: Vec<(&str, Vec<String>)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
        .collect();
    assert_eq!(
        engine_imports,
        vec![
            ("quarry::shapes::Circle", sv(&["Circle"])),
            ("quarry::shapes::Shape", sv(&["Shape"])),
        ]
    );
    let engine_calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        engine_calls,
        vec![
            ("quarry::core::Engine::Engine::render", "shape.area", 15),
            ("quarry::core::Engine::buildEngine", "Engine", 20),
            ("quarry::core::Engine::buildEngine", "Circle", 20),
        ]
    );
    let render = engine
        .symbols
        .iter()
        .find(|s| s.id == "quarry::core::Engine::Engine::render")
        .expect("render present");
    assert_eq!(render.kind, NodeKind::Method);
    assert_eq!(render.line, 13);
    assert_eq!(render.defines, sv(&[]));
    assert_eq!(render.refs, sv(&["Double", "counter", "shape"]));
    assert_eq!(render.writes, sv(&[]));
    let class_sym = engine
        .symbols
        .iter()
        .find(|s| s.id == "quarry::core::Engine::Engine")
        .expect("Engine present");
    assert_eq!(class_sym.kind, NodeKind::Class);
    assert_eq!(class_sym.refs, sv(&["DEFAULT_SIZE", "Int"]));

    // Main.kt — the entry point.
    let main = &files["Main.kt"];
    assert_eq!(main.module_id, "quarry::Main");
    assert_eq!(sorted_ids(main), vec!["quarry::Main::main"]);
    let main_calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        main_calls,
        vec![
            ("quarry::Main::main", "Engine", 7),
            ("quarry::Main::main", "Circle", 7),
            ("quarry::Main::main", "println", 8),
            ("quarry::Main::main", "engine.render", 8),
        ]
    );
    let main_sym = &main.symbols[0];
    assert_eq!(main_sym.defines, sv(&["engine"]));
    assert_eq!(main_sym.refs, sv(&["Circle", "Engine", "println"]));
    assert_eq!(main_sym.writes, sv(&["engine"]));

    // Totals across the package: 10 symbols, 5 imports, 7 calls,
    // 1 heritage row (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (10, 5, 7, 1)
    );
}
