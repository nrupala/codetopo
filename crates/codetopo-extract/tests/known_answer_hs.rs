// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-hs`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-hs")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in ["Shapes.hs", "Core/Engine.hs", "Main.hs"] {
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
fn hs_extraction_known_answers() {
    let files = extract_fixture();

    // Shapes.hs — class with a default method, data/newtype/type,
    // an instance, shared top-level values.
    let shapes = &files["Shapes.hs"];
    assert_eq!(shapes.module_id, "quarry::Shapes");
    assert_eq!(
        sorted_ids(shapes),
        vec![
            "quarry::Shapes::Circle",
            "quarry::Shapes::Circle::area",
            "quarry::Shapes::Circle::describe",
            "quarry::Shapes::Radius",
            "quarry::Shapes::RadiusAlias",
            "quarry::Shapes::Shape",
            "quarry::Shapes::Shape::area",
            "quarry::Shapes::Shape::describe",
            "quarry::Shapes::defaultRadius",
            "quarry::Shapes::scaleFactor",
            "quarry::Shapes::unitCircle",
        ]
    );
    let class = sym(shapes, "quarry::Shapes::Shape");
    assert_eq!(class.kind, NodeKind::Interface);
    assert_eq!(class.line, 5);
    assert_eq!(class.signature.as_deref(), Some("class Shape a"));
    let class_area = sym(shapes, "quarry::Shapes::Shape::area");
    assert_eq!(class_area.kind, NodeKind::Method);
    assert_eq!(class_area.line, 6);
    assert_eq!(class_area.signature.as_deref(), Some("area :: a -> Double"));
    assert_eq!(class_area.decisions, None);
    assert_eq!(class_area.refs, sv(&["Double"]));
    let class_describe = sym(shapes, "quarry::Shapes::Shape::describe");
    assert_eq!(class_describe.kind, NodeKind::Method);
    assert_eq!(class_describe.line, 7);
    assert_eq!(class_describe.defines, sv(&["s"]));
    assert_eq!(class_describe.refs, sv(&["String"]));
    assert_eq!(class_describe.decisions, Some(0));
    let circle = sym(shapes, "quarry::Shapes::Circle");
    assert_eq!(circle.kind, NodeKind::Class);
    assert_eq!(circle.line, 10);
    assert_eq!(circle.signature.as_deref(), Some("data Circle"));
    let radius = sym(shapes, "quarry::Shapes::Radius");
    assert_eq!(radius.kind, NodeKind::Class);
    assert_eq!(radius.line, 13);
    let alias = sym(shapes, "quarry::Shapes::RadiusAlias");
    assert_eq!(alias.kind, NodeKind::Class);
    assert_eq!(alias.line, 15);
    let inst_area = sym(shapes, "quarry::Shapes::Circle::area");
    assert_eq!(inst_area.kind, NodeKind::Method);
    assert_eq!(inst_area.line, 18);
    assert_eq!(inst_area.defines, sv(&["r"]));
    assert_eq!(inst_area.refs, sv(&["Circle"]));
    assert_eq!(inst_area.decisions, Some(0));
    let inst_describe = sym(shapes, "quarry::Shapes::Circle::describe");
    assert_eq!(inst_describe.line, 19);
    assert_eq!(inst_describe.defines, sv(&["c"]));
    assert!(inst_describe.refs.is_empty());
    let unit = sym(shapes, "quarry::Shapes::unitCircle");
    assert_eq!(unit.kind, NodeKind::Variable);
    assert_eq!(unit.line, 21);
    assert_eq!(unit.signature.as_deref(), Some("unitCircle :: Circle"));
    assert_eq!(unit.refs, sv(&["Circle"]));
    let factor = sym(shapes, "quarry::Shapes::scaleFactor");
    assert_eq!(factor.line, 24);
    assert_eq!(factor.refs, sv(&["Double"]));
    let def_radius = sym(shapes, "quarry::Shapes::defaultRadius");
    assert_eq!(def_radius.line, 27);
    assert_eq!(def_radius.refs, sv(&["Radius"]));
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = shapes
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(
        heritage,
        vec![
            ("Circle", "Show", EdgeKind::Implements, 11),
            ("Circle", "Eq", EdgeKind::Implements, 11),
            ("Circle", "Shape", EdgeKind::Implements, 17),
        ]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = shapes
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![("external::Data", vec!["fromMaybe".to_string()], 3)]
    );
    let calls: Vec<(&str, &str, u32)> = shapes
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::Shapes", "Circle", 22),
            ("quarry::Shapes", "Radius", 28),
        ]
    );

    // Core/Engine.hs — cross-module calls, a let/if function, a
    // case function, a qualified call.
    let engine = &files["Core/Engine.hs"];
    assert_eq!(engine.module_id, "quarry::Core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::Core::Engine::EngineState",
            "quarry::Core::Engine::describeAll",
            "quarry::Core::Engine::fallbackArea",
            "quarry::Core::Engine::render",
            "quarry::Core::Engine::renderCount",
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
            ("external::Data", vec!["fromMaybe".to_string()], 3),
            (
                "quarry::Shapes",
                vec![
                    "Circle".to_string(),
                    "Shape".to_string(),
                    "area".to_string(),
                    "describe".to_string(),
                    "scaleFactor".to_string(),
                    "unitCircle".to_string()
                ],
                4
            ),
            ("quarry::Shapes", vec!["Shapes".to_string()], 5),
        ]
    );
    let render = sym(engine, "quarry::Core::Engine::render");
    assert_eq!(render.kind, NodeKind::Function);
    assert_eq!(render.line, 9);
    assert_eq!(
        render.signature.as_deref(),
        Some("render :: EngineState -> Circle -> Double")
    );
    assert_eq!(render.defines, sv(&["scaled", "state", "wheel"]));
    assert_eq!(
        render.refs,
        sv(&[
            "Circle",
            "Double",
            "EngineState",
            "area",
            "renders",
            "scaleFactor",
            "unitCircle"
        ])
    );
    assert!(render.writes.is_empty());
    assert_eq!(render.decisions, Some(2));
    let count = sym(engine, "quarry::Core::Engine::renderCount");
    assert_eq!(count.line, 16);
    assert_eq!(count.defines, sv(&["override", "state"]));
    assert_eq!(
        count.refs,
        sv(&["EngineState", "Int", "Maybe", "fromMaybe", "renders"])
    );
    assert_eq!(count.decisions, Some(0));
    let describe_all = sym(engine, "quarry::Core::Engine::describeAll");
    assert_eq!(describe_all.line, 19);
    assert_eq!(describe_all.defines, sv(&["c", "rest", "shapes"]));
    assert_eq!(
        describe_all.refs,
        sv(&["Circle", "String", "describe", "describeAll"])
    );
    assert_eq!(describe_all.decisions, Some(1));
    let fallback = sym(engine, "quarry::Core::Engine::fallbackArea");
    assert_eq!(fallback.kind, NodeKind::Variable);
    assert_eq!(fallback.line, 24);
    assert_eq!(fallback.refs, sv(&["Double", "area", "unitCircle"]));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::Core::Engine::render", "area", 11),
            ("quarry::Core::Engine::render", "renders", 12),
            ("quarry::Core::Engine::render", "area", 14),
            ("quarry::Core::Engine::renderCount", "fromMaybe", 17),
            ("quarry::Core::Engine::renderCount", "renders", 17),
            ("quarry::Core::Engine::describeAll", "describe", 22),
            ("quarry::Core::Engine::describeAll", "describeAll", 22),
            ("quarry::Core::Engine", "Shapes.area", 25),
        ]
    );
    assert!(engine.heritage.is_empty());

    // Main.hs — imports, one variable (the IO action), module calls.
    let main = &files["Main.hs"];
    assert_eq!(main.module_id, "quarry::Main");
    assert_eq!(sorted_ids(main), vec!["quarry::Main::main"]);
    let main_sym = sym(main, "quarry::Main::main");
    assert_eq!(main_sym.kind, NodeKind::Variable);
    assert_eq!(main_sym.line, 6);
    assert_eq!(main_sym.signature.as_deref(), Some("main :: IO ()"));
    assert_eq!(main_sym.defines, sv(&["state", "wheel"]));
    assert_eq!(
        main_sym.refs,
        sv(&[
            "EngineState",
            "IO",
            "Nothing",
            "describeAll",
            "print",
            "render",
            "renderCount",
            "unitCircle"
        ])
    );
    assert_eq!(main_sym.decisions, None);
    let imports: Vec<(&str, Vec<String>, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            (
                "quarry::Core::Engine",
                vec![
                    "EngineState".to_string(),
                    "describeAll".to_string(),
                    "render".to_string(),
                    "renderCount".to_string()
                ],
                3
            ),
            (
                "quarry::Shapes",
                vec!["Circle".to_string(), "unitCircle".to_string()],
                4
            ),
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
            ("quarry::Main", "EngineState", 9),
            ("quarry::Main", "print", 10),
            ("quarry::Main", "render", 10),
            ("quarry::Main", "print", 11),
            ("quarry::Main", "renderCount", 11),
            ("quarry::Main", "print", 12),
            ("quarry::Main", "describeAll", 12),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (17, 6, 17, 3)
    );
}

