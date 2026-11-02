// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Python extractor (`.py`, `.pyi`) — the third extractor, beside Rust
//! and TypeScript/JavaScript.
//!
//! Node kinds below were verified against
//! `tree-sitter-python-0.25.0/src/node-types.json` in the crates.io
//! registry cache (the pinned crate, never guessed): `module`,
//! `function_definition`, `class_definition`, `decorated_definition`,
//! `import_statement`, `import_from_statement` (fields `module_name`,
//! `name`), `call` (fields `function`, `arguments`), `assignment`
//! (fields `left`, `right`, `type`), `parameters`, `aliased_import`
//! (fields `name`, `alias`), `attribute` (fields `object`,
//! `attribute`), `block`. One delta against the spec pass's research
//! copy of the grammar: `tuple_expression` exists there but not in the
//! pinned 0.25.0; nothing in the mapping depends on it.
//!
//! The mapping itself is specified in
//! `~/workspace/codetopo-work/python-extractor/SPEC.md` §3 and lands in
//! stages: this module currently carries the module-id rule and the
//! parse gate; the symbol walk follows in the same PR series.

use std::path::Path;

use tree_sitter::Parser;

use codetopo_core::emit::FileSymbols;

use crate::{display_path, ExtractError};

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    let module_id = module_id_for(package, rel_path);
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let _tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;

    // WP0 skeleton: the parse gate and the module id are in place; the
    // symbol/import/call/heritage walk lands in the following WPs.
    Ok(FileSymbols {
        package: package.to_string(),
        file: display_path(rel_path),
        module_id,
        symbols: Vec::new(),
        imports: Vec::new(),
        calls: Vec::new(),
        heritage: Vec::new(),
    })
}

/// `__init__.py` → `{pkg}`; `core/__init__.py` → `{pkg}::core`;
/// `core/graph.py` → `{pkg}::core::graph`; `main.py` → `{pkg}::main`.
/// The `__init__` collapse mirrors the TypeScript `index` rule: a
/// package's `__init__` module doubles as the package module
/// (CODE_SCHEMA §3.1). `.pyi` stubs follow the same rule.
fn module_id_for(pkg: &str, rel_path: &Path) -> String {
    let mut s = display_path(rel_path);
    for ext in [".pyi", ".py"] {
        if let Some(stripped) = s.strip_suffix(ext) {
            s = stripped.to_string();
            break;
        }
    }
    if s == "__init__" {
        s.clear();
    } else if let Some(stripped) = s.strip_suffix("/__init__") {
        s = stripped.to_string();
    }
    if s.is_empty() {
        pkg.to_string()
    } else {
        format!("{}::{}", pkg, s.replace('/', "::"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn py_module_id_rules() {
        let cases = [
            ("__init__.py", "mypkg"),
            ("core/__init__.py", "mypkg::core"),
            ("core/graph.py", "mypkg::core::graph"),
            ("main.py", "mypkg::main"),
            ("core/graph.pyi", "mypkg::core::graph"),
            ("core/__init__.pyi", "mypkg::core"),
            ("a/b/c.py", "mypkg::a::b::c"),
        ];
        for (rel, want) in cases {
            assert_eq!(module_id_for("mypkg", Path::new(rel)), want, "{rel}");
        }
    }

    #[test]
    fn py_skeleton_extracts_module_only() {
        let fs = extract("mypkg", Path::new("core/graph.py"), "import os\n").unwrap();
        assert_eq!(fs.module_id, "mypkg::core::graph");
        assert!(fs.symbols.is_empty());
    }
}
