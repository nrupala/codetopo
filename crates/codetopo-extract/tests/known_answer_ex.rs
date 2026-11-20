// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The known-answer fixture package (`tests/fixture-ex`).
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-ex")
}

fn extract_fixture() -> BTreeMap<String, FileSymbols> {
    let dir = fixture_dir();
    let mut out = BTreeMap::new();
    for name in [
        "Shapes/Shape.ex",
        "Shapes/Circle.ex",
        "Shapes/Drawable.ex",
        "Core/Engine.ex",
        "main.exs",
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
fn ex_extraction_known_answers() {
    let files = extract_fixture();

    // Shapes/Shape.ex — the behaviour module; the @callback
    // declares the contract and is neither symbol nor call.
    let shape = &files["Shapes/Shape.ex"];
    assert_eq!(shape.module_id, "quarry::Shapes::Shape");
    assert_eq!(
        sorted_ids(shape),
        vec!["quarry::Shapes::Shape::Quarry::Shapes::Shape"]
    );
    let shape_mod = sym(shape, "quarry::Shapes::Shape::Quarry::Shapes::Shape");
    assert_eq!(shape_mod.kind, NodeKind::Class);
    assert_eq!(shape_mod.line, 1);
    assert_eq!(
        shape_mod.signature.as_deref(),
        Some("defmodule Quarry.Shapes.Shape")
    );
    assert!(shape.imports.is_empty() && shape.calls.is_empty() && shape.heritage.is_empty());

    // Shapes/Circle.ex — defmodule, @behaviour, defstruct,
    // the @pi attribute, def / defp.
    let circle = &files["Shapes/Circle.ex"];
    assert_eq!(circle.module_id, "quarry::Shapes::Circle");
    assert_eq!(
        sorted_ids(circle),
        vec![
            "quarry::Shapes::Circle::Quarry::Shapes::Circle",
            "quarry::Shapes::Circle::Quarry::Shapes::Circle::area",
            "quarry::Shapes::Circle::Quarry::Shapes::Circle::new",
            "quarry::Shapes::Circle::Quarry::Shapes::Circle::scale",
        ]
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = circle
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(heritage, vec![("Circle", "Shape", EdgeKind::Implements, 2)]);
    let new = sym(circle, "quarry::Shapes::Circle::Quarry::Shapes::Circle::new");
    assert_eq!(new.kind, NodeKind::Method);
    assert_eq!(new.line, 8);
    assert_eq!(new.signature.as_deref(), Some("def new(radius)"));
    assert_eq!(new.defines, sv(&["radius"]));
    assert!(new.refs.is_empty() && new.writes.is_empty());
    assert_eq!(new.decisions, Some(0));
    let area = sym(circle, "quarry::Shapes::Circle::Quarry::Shapes::Circle::area");
    assert_eq!(area.line, 12);
    assert_eq!(
        area.signature.as_deref(),
        Some("def area(%__MODULE__{radius: radius})")
    );
    assert_eq!(area.defines, sv(&["radius"]));
    assert_eq!(area.refs, sv(&["@pi"]));
    assert_eq!(area.decisions, Some(0));
    let scale = sym(circle, "quarry::Shapes::Circle::Quarry::Shapes::Circle::scale");
    assert_eq!(scale.line, 16);
    assert_eq!(scale.defines, sv(&["factor", "radius"]));
    assert!(circle.imports.is_empty() && circle.calls.is_empty());

    // Shapes/Drawable.ex — the protocol and its Circle
    // implementation.
    let drawable = &files["Shapes/Drawable.ex"];
    assert_eq!(drawable.module_id, "quarry::Shapes::Drawable");
    assert_eq!(
        sorted_ids(drawable),
        vec![
            "quarry::Shapes::Drawable::Quarry::Shapes::Drawable",
            "quarry::Shapes::Drawable::Quarry::Shapes::Drawable::Quarry::Shapes::Circle",
            "quarry::Shapes::Drawable::Quarry::Shapes::Drawable::Quarry::Shapes::Circle::draw",
            "quarry::Shapes::Drawable::Quarry::Shapes::Drawable::draw",
        ]
    );
    let proto = sym(drawable, "quarry::Shapes::Drawable::Quarry::Shapes::Drawable");
    assert_eq!(proto.kind, NodeKind::Interface);
    assert_eq!(proto.line, 1);
    let pdraw = sym(drawable, "quarry::Shapes::Drawable::Quarry::Shapes::Drawable::draw");
    assert_eq!(pdraw.kind, NodeKind::Method);
    assert_eq!(pdraw.line, 2);
    assert_eq!(pdraw.signature.as_deref(), Some("def draw(shape)"));
    assert_eq!(pdraw.defines, sv(&["shape"]));
    assert_eq!(pdraw.decisions, None);
    let impl_mod = sym(
        drawable,
        "quarry::Shapes::Drawable::Quarry::Shapes::Drawable::Quarry::Shapes::Circle",
    );
    assert_eq!(impl_mod.kind, NodeKind::Class);
    assert_eq!(impl_mod.line, 5);
    assert_eq!(
        impl_mod.signature.as_deref(),
        Some("defimpl Quarry.Shapes.Drawable, for: Quarry.Shapes.Circle")
    );
    let heritage: Vec<(&str, &str, EdgeKind, u32)> = drawable
        .heritage
        .iter()
        .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind, h.line))
        .collect();
    assert_eq!(
        heritage,
        vec![("Circle", "Drawable", EdgeKind::Implements, 5)]
    );
    let idraw = sym(
        drawable,
        "quarry::Shapes::Drawable::Quarry::Shapes::Drawable::Quarry::Shapes::Circle::draw",
    );
    assert_eq!(idraw.line, 6);
    assert_eq!(idraw.defines, sv(&["circle"]));
    assert_eq!(idraw.refs, sv(&["IO"]));
    assert_eq!(idraw.decisions, Some(0));
    let calls: Vec<(&str, &str, u32)> = drawable
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![(
            "quarry::Shapes::Drawable::Quarry::Shapes::Drawable::Quarry::Shapes::Circle::draw",
            "inspect",
            7
        )]
    );

    // Core/Engine.ex — use / alias / import / require, the
    // render decisions.
    let engine = &files["Core/Engine.ex"];
    assert_eq!(engine.module_id, "quarry::Core::Engine");
    assert_eq!(
        sorted_ids(engine),
        vec![
            "quarry::Core::Engine::Quarry::Core::Engine",
            "quarry::Core::Engine::Quarry::Core::Engine::new",
            "quarry::Core::Engine::Quarry::Core::Engine::render",
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
            ("external::GenServer", vec![], 2),
            ("quarry::Shapes::Circle", vec!["Circle".to_string()], 4),
            ("quarry::Shapes::Drawable", vec!["Drawable".to_string()], 5),
            ("quarry::Shapes::Circle", vec!["area".to_string()], 6),
            ("external::Logger", vec!["Logger".to_string()], 7),
        ]
    );
    let enew = sym(engine, "quarry::Core::Engine::Quarry::Core::Engine::new");
    assert_eq!(enew.line, 11);
    assert_eq!(enew.signature.as_deref(), Some("def new"));
    assert!(enew.defines.is_empty() && enew.refs.is_empty());
    assert_eq!(enew.decisions, Some(0));
    let render = sym(engine, "quarry::Core::Engine::Quarry::Core::Engine::render");
    assert_eq!(render.line, 15);
    assert_eq!(render.defines, sv(&["engine", "wheel"]));
    assert_eq!(render.refs, sv(&["Drawable", "Logger", "area"]));
    assert!(render.writes.is_empty());
    assert_eq!(render.decisions, Some(2));
    let calls: Vec<(&str, &str, u32)> = engine
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::Core::Engine::Quarry::Core::Engine::render", "info", 17),
            ("quarry::Core::Engine::Quarry::Core::Engine::render", "area", 20),
            ("quarry::Core::Engine::Quarry::Core::Engine::render", "draw", 20),
        ]
    );

    // main.exs — two aliases, two variables, four calls.
    let main = &files["main.exs"];
    assert_eq!(main.module_id, "quarry::main");
    assert_eq!(
        sorted_ids(main),
        vec!["quarry::main::engine", "quarry::main::wheel"]
    );
    let imports: Vec<(&str, Vec<String>, u32)> = main
        .imports
        .iter()
        .map(|i| (i.to_module_id.as_str(), i.bindings.clone(), i.line))
        .collect();
    assert_eq!(
        imports,
        vec![
            ("quarry::Core::Engine", vec!["Engine".to_string()], 1),
            ("quarry::Shapes::Circle", vec!["Circle".to_string()], 2),
        ]
    );
    let eng_var = sym(main, "quarry::main::engine");
    assert_eq!(eng_var.kind, NodeKind::Variable);
    assert_eq!(eng_var.refs, sv(&["Engine"]));
    let wheel_var = sym(main, "quarry::main::wheel");
    assert_eq!(wheel_var.refs, sv(&["Circle"]));
    let calls: Vec<(&str, &str, u32)> = main
        .calls
        .iter()
        .map(|c| (c.from_id.as_str(), c.to_name.as_str(), c.line))
        .collect();
    assert_eq!(
        calls,
        vec![
            ("quarry::main", "new", 4),
            ("quarry::main", "new", 5),
            ("quarry::main", "puts", 6),
            ("quarry::main", "render", 6),
        ]
    );

    // Totals across the fixture (arithmetic in the fixture README).
    let total_symbols: usize = files.values().map(|f| f.symbols.len()).sum();
    let total_imports: usize = files.values().map(|f| f.imports.len()).sum();
    let total_calls: usize = files.values().map(|f| f.calls.len()).sum();
    let total_heritage: usize = files.values().map(|f| f.heritage.len()).sum();
    assert_eq!(
        (total_symbols, total_imports, total_calls, total_heritage),
        (14, 7, 8, 2)
    );
}
