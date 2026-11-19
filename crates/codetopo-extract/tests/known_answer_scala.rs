// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-scala`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-scala")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["Shapes.scala", "Engine.scala", "Main.scala"] {
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
fn scala_extraction_known_answers() {
    let files = extract_fixture();

    // Shapes.scala — the trait, its abstract def, the class.
    let shapes = &files["Shapes.scala"];
    assert_eq!(shapes.module_id, "quarry::Shapes");
    assert_eq!(
        sorted_ids(shapes),
        vec![
            "quarry::Shapes::Circle",
            "quarry::Shapes::Circle::area",
            "quarry::Shapes::Shape",
            "quarry::Shapes::Shape::area",
        ]
    );
    let shape_ty = sym(shapes, "quarry::Shapes::Shape");
    assert_eq!(shape_ty.kind, NodeKind::Interface);
    assert_eq!(shape_ty.line, 3);
    let shape_area = sym(shapes, "quarry::Shapes::Shape::area");
    assert_eq!(shape_area.kind, NodeKind::Method);
    assert_eq!(shape_area.line, 4);
    assert_eq!(shape_area.signature.as_deref(), Some("def area: Double"));
    assert_eq!(shape_area.decisions, None);
    assert!(shape_area.defines.is_empty());
    assert_eq!(shape_area.refs, sv(&["Double"]));
    let circle = sym(shapes, "quarry::Shapes::Circle");
    assert_eq!(circle.kind, NodeKind::Class);
    assert_eq!(circle.line, 7);
    assert_eq!(
        circle.signature.as_deref(),
        Some("class Circle(val radius: Double) extends Shape")
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = shapes
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Inherits, 7)]);
    let circle_area = sym(shapes, "quarry::Shapes::Circle::area");
    assert_eq!(circle_area.kind, NodeKind::Method);
    assert_eq!(circle_area.line, 8);
    assert!(circle_area.defines.is_empty());
    assert_eq!(circle_area.refs, sv(&["Double", "radius"]));
    assert_eq!(circle_area.decisions, Some(0));
    assert!(shapes.imports.is_empty() && shapes.calls.is_empty());

    // Engine.scala — the import, the render methods.
    let engine = &files["Engine.scala"];
    assert_eq!(engine.module_id, "quarry::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::Engine::Engine",
            "quarry::Engine::Engine::render",
            "quarry::Engine::Engine::renderCount",
        ]
    );
    let engine_ty = sym(engine, "quarry::Engine::Engine");
    assert_eq!(engine_ty.kind, NodeKind::Class);
    assert_eq!(engine_ty.line, 5);
    let imports: Vec<(&str, Vec<String>, u32)> = engine
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("quarry::shapes::Circle", vec!["Circle".to_string()], 3)]
    );
    let render = sym(engine, "quarry::Engine::Engine::render");
    assert_eq!(render.kind, NodeKind::Method);
    assert_eq!(render.line, 8);
    assert_eq!(render.defines, sv(&["wheel"]));
    assert_eq!(render.refs, sv(&["Circle", "Double", "renders"]));
    assert!(render.writes.is_empty());
    assert_eq!(render.decisions, Some(0));
    let count = sym(engine, "quarry::Engine::Engine::renderCount");
    assert_eq!(count.kind, NodeKind::Method);
    assert_eq!(count.line, 13);
    assert!(count.defines.is_empty());
    assert_eq!(count.refs, sv(&["Int", "renders"]));
    assert_eq!(count.decisions, Some(0));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![("quarry::Engine::Engine::render", "wheel.area", 10)]
    );
    assert!(engine.heritage.is_empty());

    // Main.scala — the object entry point, two imports, four calls.
    let main = &files["Main.scala"];
    assert_eq!(main.module_id, "quarry::Main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::Main::Main", "quarry::Main::Main::main"]
    );
    let main_obj = sym(main, "quarry::Main::Main");
    assert_eq!(main_obj.kind, NodeKind::Class);
    assert_eq!(main_obj.line, 6);
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
    let main_fn = sym(main, "quarry::Main::Main::main");
    assert_eq!(main_fn.kind, NodeKind::Method);
    assert_eq!(main_fn.line, 7);
    assert_eq!(main_fn.defines, sv(&["args", "engine", "wheel"]));
    assert_eq!(main_fn.writes, sv(&["engine", "wheel"]));
    assert_eq!(main_fn.refs, sv(&["Array", "String", "Unit", "println"]));
    assert_eq!(main_fn.decisions, Some(0));
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
            ("quarry::Main::Main::main", "println", 10),
            ("quarry::Main::Main::main", "engine.render", 10),
        ]
    );
    assert!(main.heritage.is_empty());

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

#[test]
fn scala_deep_chain_does_not_overflow() {
    // A 50,000-segment string-concatenation chain in Scala
    // idiom (a `def` whose body is `"a" + "a" + …`). The
    // walkers are heap-stack iterative, and the grammar's
    // parser survives the full 50,000 in-process (verified
    // 2026-11-19) — no bisect reduction was needed. (The
    // standalone tsdump tool's recursive s-expression printer
    // overflows on this input; that is the dumper, not the
    // parser or this extractor.)
    let mut src = String::from("package quarry\nobject Deep {\n  def chain: String = ");
    for _ in 0..50_000 {
        src.push_str("\"a\" + ");
    }
    src.push_str("\"z\"\n}\n");
    let fs = codetopo_extract::extract_file("quarry", Path::new("Deep.scala"), &src)
        .expect("deep chain extracts");
    assert!(fs.symbols.iter().any(|s| s.label == "chain"));
}
