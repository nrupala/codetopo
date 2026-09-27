// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! TypeScript / JavaScript extractor (`.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`).
//!
//! Node kinds below were verified against
//! `tree-sitter-typescript-0.23.2/typescript/src/node-types.json` in the
//! crates.io registry cache (never guessed): `function_declaration`,
//! `generator_function_declaration`, `class_declaration`,
//! `abstract_class_declaration`, `interface_declaration`, `method_definition`,
//! `method_signature`, `lexical_declaration`, `variable_declarator`,
//! `import_statement`, `call_expression`, `new_expression`, `export_statement`,
//! `module` (a `namespace` block), `formal_parameters`, `arrow_function`.
//!
//! `.tsx`/`.jsx` files parse with the TSX grammar (JSX syntax); everything
//! else uses the plain TypeScript grammar, which also accepts plain JS.

use std::path::Path;

use tree_sitter::{Node, Parser};

use codetopo_core::emit::{FileSymbols, RawCall, RawImport, RawSymbol};
use codetopo_core::schema::NodeKind;

use crate::{children, display_path, line_of, text_of, ExtractError};

/// Extensions the TypeScript grammar strips when computing module ids.
const STRIP_EXTS: &[&str] = &["d.ts", "ts", "tsx", "js", "jsx", "mts", "cts"];

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
    tsx: bool,
) -> Result<FileSymbols, ExtractError> {
    let module_id = module_id_for(package, rel_path);
    let mut parser = Parser::new();
    let language: tree_sitter::Language = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    parser
        .set_language(&language)
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;

    let mut w = Walker {
        src: source.as_bytes(),
        pkg: package.to_string(),
        module_id: module_id.clone(),
        rel: display_path(rel_path),
        path: Vec::new(),
        enclosing: Vec::new(),
        in_assoc: false,
        symbols: Vec::new(),
        imports: Vec::new(),
        calls: Vec::new(),
    };
    for child in children(tree.root_node()) {
        w.walk(child);
    }
    Ok(FileSymbols {
        package: package.to_string(),
        file: display_path(rel_path),
        module_id,
        symbols: w.symbols,
        imports: w.imports,
        calls: w.calls,
    })
}

/// `src/util.ts` → `{pkg}::src::util`; `src/index.ts` → `{pkg}::src`;
/// `index.ts` → `{pkg}`. (Unlike Rust, the leading `src/` is kept.)
fn module_id_for(pkg: &str, rel_path: &Path) -> String {
    let mut s = display_path(rel_path);
    for ext in STRIP_EXTS {
        if let Some(stripped) = s.strip_suffix(&format!(".{ext}")) {
            s = stripped.to_string();
            break;
        }
    }
    if s == "index" {
        s.clear();
    } else if let Some(stripped) = s.strip_suffix("/index") {
        s = stripped.to_string();
    }
    if s.is_empty() {
        pkg.to_string()
    } else {
        format!("{}::{}", pkg, s.replace('/', "::"))
    }
}

struct Walker<'a> {
    src: &'a [u8],
    pkg: String,
    module_id: String,
    /// Importing file's rel path (forward slashes), for `./x` resolution.
    rel: String,
    /// Current item path; symbol id = `{module_id}::{path.join("::")}`.
    path: Vec<String>,
    /// Enclosing symbol ids, innermost last; call `from_id` source.
    enclosing: Vec<String>,
    /// Inside a class/interface body: `method_definition` is expected.
    in_assoc: bool,
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
    calls: Vec<RawCall>,
}

fn field_text(node: Node, field: &str, src: &[u8]) -> Option<String> {
    node.child_by_field_name(field)
        .and_then(|n| text_of(n, src))
        .map(|s| s.to_string())
}

impl<'a> Walker<'a> {
    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    let body = node.child_by_field_name("body");
                    let params = node.child_by_field_name("parameters");
                    let (defines, refs) = self.fn_body_info(params, body);
                    let kind = if self.in_assoc {
                        NodeKind::Method
                    } else {
                        NodeKind::Function
                    };
                    self.with_symbol(kind, &name, node, defines, refs, body, false);
                } else {
                    self.walk_children(node);
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    let body = node.child_by_field_name("body");
                    let refs = self.body_refs(body);
                    self.with_symbol(NodeKind::Class, &name, node, vec![], refs, body, true);
                } else {
                    // Anonymous default-exported class: walk members without a scope name.
                    self.walk_children(node);
                }
            }
            "interface_declaration" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    let body = node.child_by_field_name("body");
                    let refs = self.body_refs(body);
                    self.with_symbol(NodeKind::Interface, &name, node, vec![], refs, body, true);
                } else {
                    self.walk_children(node);
                }
            }
            "method_definition" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    let body = node.child_by_field_name("body");
                    let params = node.child_by_field_name("parameters");
                    let (defines, refs) = self.fn_body_info(params, body);
                    let kind = if self.in_assoc {
                        NodeKind::Method
                    } else {
                        NodeKind::Function
                    };
                    self.with_symbol(kind, &name, node, defines, refs, body, false);
                } else {
                    self.walk_children(node);
                }
            }
            "method_signature" => {
                // Interface member declaration (no body). Always a method.
                if let Some(name) = field_text(node, "name", self.src) {
                    let params = node.child_by_field_name("parameters");
                    let (defines, _) = self.fn_body_info(params, None);
                    self.with_symbol(NodeKind::Method, &name, node, defines, vec![], None, false);
                }
            }
            "lexical_declaration" => {
                // Top-level `const`/`let`/`var` only; locals belong to their function.
                if self.enclosing.is_empty() {
                    for child in children(node) {
                        if child.kind() == "variable_declarator" {
                            self.handle_declarator(child);
                        }
                    }
                } else {
                    self.walk_children(node);
                }
            }
            "import_statement" => self.handle_import(node),
            "call_expression" => {
                self.handle_call(node, "function");
                self.walk_children(node);
            }
            // `new Server()` is a dependency on the constructor; record it
            // as a call so blast-radius analysis sees it.
            "new_expression" => {
                self.handle_call(node, "constructor");
                self.walk_children(node);
            }
            "export_statement" => self.walk_children(node),
            // `namespace N { ... }` scopes ids like a module; not a symbol itself.
            "module" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    self.path.push(name);
                    if let Some(body) = node.child_by_field_name("body") {
                        self.walk_children(body);
                    }
                    self.path.pop();
                } else {
                    self.walk_children(node);
                }
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: Node) {
        for child in children(node) {
            self.walk(child);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn with_symbol(
        &mut self,
        kind: NodeKind,
        name: &str,
        node: Node,
        defines: Vec<String>,
        refs: Vec<String>,
        body: Option<Node>,
        assoc_scope: bool,
    ) {
        self.path.push(name.to_string());
        let id = format!("{}::{}", self.module_id, self.path.join("::"));
        self.symbols.push(RawSymbol {
            id: id.clone(),
            kind,
            label: name.to_string(),
            line: line_of(node),
            column: None,
            defines,
            refs,
        });
        self.enclosing.push(id);
        let saved = self.in_assoc;
        if assoc_scope {
            self.in_assoc = true;
        }
        if let Some(b) = body {
            self.walk_children(b);
        }
        self.in_assoc = saved;
        self.enclosing.pop();
        self.path.pop();
    }

    fn handle_declarator(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        if name_node.kind() != "identifier" {
            // Destructuring at top level: still walk the value for calls.
            self.walk_children(node);
            return;
        }
        let name = match text_of(name_node, self.src) {
            Some(t) => t.to_string(),
            None => return,
        };
        let value = node.child_by_field_name("value");
        let refs = self.body_refs(value);
        self.with_symbol(NodeKind::Variable, &name, node, vec![], refs, value, false);
    }

    fn fn_body_info(&self, params: Option<Node>, body: Option<Node>) -> (Vec<String>, Vec<String>) {
        let mut defines = Vec::new();
        if let Some(p) = params {
            for child in children(p) {
                self.collect_pattern(child, &mut defines);
            }
        }
        let mut refs = Vec::new();
        if let Some(b) = body {
            self.scan(b, &mut defines, &mut refs);
        }
        finalize(&mut defines, &mut refs);
        (defines, refs)
    }

    fn body_refs(&self, body: Option<Node>) -> Vec<String> {
        let mut defines = Vec::new();
        let mut refs = Vec::new();
        if let Some(b) = body {
            self.scan(b, &mut defines, &mut refs);
        }
        finalize(&mut defines, &mut refs);
        refs
    }

    /// Identifiers bound by a parameter or destructuring pattern.
    /// Only descends through pattern-shaped nodes so type annotations
    /// (e.g. `typeof foo`) don't leak into defines.
    fn collect_pattern(&self, node: Node, defines: &mut Vec<String>) {
        match node.kind() {
            "identifier" => {
                if let Some(t) = text_of(node, self.src) {
                    defines.push(t.to_string());
                }
            }
            "required_parameter" | "optional_parameter" => {
                if let Some(p) = node.child_by_field_name("pattern") {
                    self.collect_pattern(p, defines);
                }
            }
            k if k.ends_with("_pattern") || k == "formal_parameters" => {
                for child in children(node) {
                    self.collect_pattern(child, defines);
                }
            }
            _ => {}
        }
    }

    /// Walk a body collecting locally-bound names into `defines` and other
    /// identifiers into `refs`. Nested declarations are skipped: their
    /// bindings belong to their own symbols. JSX subtrees are skipped to
    /// keep tag names (`div`, `span`) out of refs.
    fn scan(&self, node: Node, defines: &mut Vec<String>, refs: &mut Vec<String>) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" | "class_declaration"
            | "abstract_class_declaration" | "interface_declaration" | "import_statement"
            | "method_definition" | "method_signature" => return,
            "jsx_element" | "jsx_self_closing_element" | "jsx_fragment" => return,
            "variable_declarator" => {
                if let Some(n) = node.child_by_field_name("name") {
                    self.collect_pattern(n, defines);
                }
                if let Some(v) = node.child_by_field_name("value") {
                    self.scan(v, defines, refs);
                }
                return;
            }
            "arrow_function" => {
                for child in children(node) {
                    if child.kind() == "formal_parameters" {
                        for p in children(child) {
                            self.collect_pattern(p, defines);
                        }
                    } else {
                        self.scan(child, defines, refs);
                    }
                }
                return;
            }
            "identifier" => {
                if let Some(t) = text_of(node, self.src) {
                    refs.push(t.to_string());
                }
                return;
            }
            _ => {}
        }
        for child in children(node) {
            self.scan(child, defines, refs);
        }
    }

    /// Record a call; `callee_field` is `function` (call) or `constructor` (new).
    fn handle_call(&mut self, node: Node, callee_field: &str) {
        let from_id = self
            .enclosing
            .last()
            .cloned()
            .unwrap_or_else(|| self.module_id.clone());
        if let Some(callee) = node.child_by_field_name(callee_field) {
            if let Some(t) = text_of(callee, self.src).map(str::trim) {
                if !t.is_empty() {
                    self.calls.push(RawCall {
                        from_id,
                        to_name: t.to_string(),
                        line: line_of(node),
                    });
                }
            }
        }
    }

    // ---------------- `import` handling ----------------

    fn handle_import(&mut self, node: Node) {
        let line = line_of(node);
        let spec = node
            .child_by_field_name("source")
            .and_then(|s| text_of(s, self.src))
            .map(unquote)
            .unwrap_or_default();
        let mut bindings: Vec<String> = Vec::new();
        for child in children(node) {
            if child.kind() != "import_clause" {
                continue;
            }
            for item in children(child) {
                match item.kind() {
                    // `import def from 'm'`
                    "identifier" => {
                        if let Some(t) = text_of(item, self.src) {
                            bindings.push(t.to_string());
                        }
                    }
                    // `import * as ns from 'm'`
                    "namespace_import" => {
                        for n in children(item) {
                            if n.kind() == "identifier" {
                                if let Some(t) = text_of(n, self.src) {
                                    bindings.push(t.to_string());
                                }
                            }
                        }
                    }
                    // `import { a, b as c } from 'm'`
                    "named_imports" => {
                        for spec_node in children(item) {
                            if spec_node.kind() != "import_specifier" {
                                continue;
                            }
                            let name = field_text(spec_node, "name", self.src);
                            let alias = field_text(spec_node, "alias", self.src);
                            if let Some(b) = alias.or(name) {
                                bindings.push(b);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let to_module_id = if spec.starts_with('.') {
            self.resolve_relative(&spec)
        } else {
            format!("external::{spec}")
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id,
            bindings,
            line,
        });
    }

    /// Resolve `./x` / `../x` against the importing file's directory into a
    /// module id. Purely syntactic (no filesystem access): `x` is treated as
    /// `{pkg}::<dir>::x`, a trailing `/index` collapses to the parent, and
    /// `..` past the package root becomes `external::unresolved`.
    fn resolve_relative(&self, spec: &str) -> String {
        let mut parts: Vec<String> = self.rel.split('/').map(|s| s.to_string()).collect();
        parts.pop(); // the importing file's own name
        for seg in spec.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return "external::unresolved".to_string();
                    }
                }
                s => parts.push(s.to_string()),
            }
        }
        if let Some(last) = parts.last_mut() {
            let mut owned = last.clone();
            for ext in STRIP_EXTS {
                if let Some(stripped) = owned.strip_suffix(&format!(".{ext}")) {
                    owned = stripped.to_string();
                    break;
                }
            }
            *last = owned;
        }
        if parts.last().map(|s| s == "index").unwrap_or(false) {
            parts.pop();
        }
        if parts.is_empty() {
            self.pkg.clone()
        } else {
            format!("{}::{}", self.pkg, parts.join("::"))
        }
    }
}

/// `"./util"` → `./util` (quotes stripped).
fn unquote(s: &str) -> String {
    let t = s.trim();
    let b = t.as_bytes();
    if t.len() >= 2
        && ((b[0] == b'\'' && b[t.len() - 1] == b'\'')
            || (b[0] == b'"' && b[t.len() - 1] == b'"'))
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

/// Dedupe preserving first-seen order; drop refs that are also defines.
fn finalize(defines: &mut Vec<String>, refs: &mut Vec<String>) {
    dedupe(defines);
    dedupe(refs);
    refs.retain(|r| !defines.contains(r));
}

fn dedupe(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| seen.insert(s.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract_file;

    const SRC: &str = r#"import { helper } from './util';
import * as path from 'path';

export class Server {
    start(port: number): void {
        const addr = helper(port);
        console.log(addr);
    }
}

export function boot(): void {
    const s = new Server();
    s.start(8080);
}
"#;

    fn ids(fs: &FileSymbols) -> Vec<(&str, NodeKind)> {
        fs.symbols.iter().map(|s| (s.id.as_str(), s.kind)).collect()
    }

    #[test]
    fn ts_module_id_rules() {
        let cases = [
            ("src/util.ts", "mypkg::src::util"),
            ("src/index.ts", "mypkg::src"),
            ("index.ts", "mypkg"),
            ("src/a.tsx", "mypkg::src::a"),
            ("lib/x.mts", "mypkg::lib::x"),
            ("lib/y.cts", "mypkg::lib::y"),
            ("app.jsx", "mypkg::app"),
            ("types.d.ts", "mypkg::types"),
        ];
        for (rel, want) in cases {
            assert_eq!(module_id_for("mypkg", Path::new(rel)), want, "{rel}");
        }
    }

    #[test]
    fn ts_symbols_ids_and_kinds() {
        let fs = extract_file("mypkg", Path::new("src/app.ts"), SRC).unwrap();
        assert_eq!(fs.module_id, "mypkg::src::app");
        assert_eq!(
            ids(&fs),
            vec![
                ("mypkg::src::app::Server", NodeKind::Class),
                ("mypkg::src::app::Server::start", NodeKind::Method),
                ("mypkg::src::app::boot", NodeKind::Function),
            ]
        );
    }

    #[test]
    fn ts_imports() {
        let fs = extract_file("mypkg", Path::new("src/app.ts"), SRC).unwrap();
        assert_eq!(fs.imports.len(), 2);
        assert_eq!(fs.imports[0].from_module_id, "mypkg::src::app");
        assert_eq!(fs.imports[0].to_module_id, "mypkg::src::util");
        assert_eq!(fs.imports[0].bindings, vec!["helper"]);
        assert_eq!(fs.imports[0].line, 1);
        assert_eq!(fs.imports[1].to_module_id, "external::path");
        assert_eq!(fs.imports[1].bindings, vec!["path"]);
    }

    #[test]
    fn ts_calls() {
        let fs = extract_file("mypkg", Path::new("src/app.ts"), SRC).unwrap();
        let calls: Vec<(&str, &str)> = fs
            .calls
            .iter()
            .map(|c| (c.from_id.as_str(), c.to_name.as_str()))
            .collect();
        assert!(calls.contains(&("mypkg::src::app::Server::start", "helper")), "{calls:?}");
        assert!(calls.contains(&("mypkg::src::app::Server::start", "console.log")), "{calls:?}");
        // `new Server()` is recorded as a constructor call.
        assert!(calls.contains(&("mypkg::src::app::boot", "Server")), "{calls:?}");
        assert!(calls.contains(&("mypkg::src::app::boot", "s.start")), "{calls:?}");
    }

    #[test]
    fn ts_defines_and_refs() {
        let fs = extract_file("mypkg", Path::new("src/app.ts"), SRC).unwrap();
        let by_id = |id: &str| fs.symbols.iter().find(|s| s.id == id).unwrap();
        let start = by_id("mypkg::src::app::Server::start");
        assert!(start.defines.contains(&"port".to_string()), "{:?}", start.defines);
        assert!(start.defines.contains(&"addr".to_string()), "{:?}", start.defines);
        assert!(start.refs.contains(&"helper".to_string()), "{:?}", start.refs);
        assert!(start.refs.contains(&"console".to_string()), "{:?}", start.refs);
        assert!(!start.refs.contains(&"addr".to_string()));
    }

    #[test]
    fn ts_top_level_const_is_variable() {
        let src = "export const VERSION = '1.0';\n\nexport interface Config {\n    port: number;\n}\n";
        let fs = extract_file("mypkg", Path::new("src/cfg.ts"), src).unwrap();
        assert_eq!(
            ids(&fs),
            vec![
                ("mypkg::src::cfg::VERSION", NodeKind::Variable),
                ("mypkg::src::cfg::Config", NodeKind::Interface),
            ]
        );
    }

    #[test]
    fn ts_relative_import_resolution() {
        let src = "import { a } from '../shared/a';\nimport { b } from './deep/index';\nimport def from './def.js';\n";
        let fs = extract_file("mypkg", Path::new("src/sub/mod.ts"), src).unwrap();
        let tos: Vec<(&str, Vec<&str>)> = fs
            .imports
            .iter()
            .map(|i| {
                (
                    i.to_module_id.as_str(),
                    i.bindings.iter().map(|s| s.as_str()).collect(),
                )
            })
            .collect();
        assert!(tos.contains(&("mypkg::src::shared::a", vec!["a"])), "{tos:?}");
        assert!(tos.contains(&("mypkg::src::sub::deep", vec!["b"])), "{tos:?}");
        assert!(tos.contains(&("mypkg::src::sub::def", vec!["def"])), "{tos:?}");
    }

    #[test]
    fn tsx_parses_jsx() {
        let src = "export function Card() {\n    return <div className=\"c\">hi</div>;\n}\n";
        let fs = extract_file("mypkg", Path::new("src/card.tsx"), src).unwrap();
        assert_eq!(
            ids(&fs),
            vec![("mypkg::src::card::Card", NodeKind::Function),]
        );
    }
}
