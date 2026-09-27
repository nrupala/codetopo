// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Rust extractor.
//!
//! Node kinds below were verified against
//! `tree-sitter-rust-0.24.2/src/node-types.json` in the crates.io registry
//! cache (never guessed): `function_item`, `function_signature_item`,
//! `struct_item`, `enum_item`, `union_item`, `trait_item`, `impl_item`,
//! `mod_item`, `const_item`, `static_item`, `use_declaration`,
//! `call_expression`, `let_declaration`, `parameters`, `declaration_list`,
//! `scoped_use_list`, `use_list`, `use_wildcard`.

use std::path::{Component, Path};

use tree_sitter::{Node, Parser};

use codetopo_core::emit::{FileSymbols, RawCall, RawImport, RawSymbol};
use codetopo_core::schema::NodeKind;

use crate::{children, display_path, line_of, text_of, ExtractError};

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    let module_id = module_id_for(package, rel_path);
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
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
        modpath: initial_modpath(package, &module_id),
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

/// `src/net.rs` → `{pkg}::net`; `src/lib.rs`/`src/mod.rs` → parent;
/// `src/main.rs` → `{pkg}::main`; `src/a/b.rs` → `{pkg}::a::b`.
fn module_id_for(pkg: &str, rel_path: &Path) -> String {
    let mut comps: Vec<String> = rel_path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str().map(|s| s.to_string()),
            _ => None,
        })
        .collect();
    if comps.first().map(|s| s == "src").unwrap_or(false) {
        comps.remove(0);
    }
    if let Some(last) = comps.last_mut() {
        if let Some(stripped) = last.strip_suffix(".rs") {
            *last = stripped.to_string();
        }
    }
    match comps.last().map(|s| s.as_str()) {
        Some("mod") | Some("lib") => {
            comps.pop();
        }
        _ => {}
    }
    if comps.is_empty() {
        pkg.to_string()
    } else {
        format!("{}::{}", pkg, comps.join("::"))
    }
}

/// The module's own path segments, used to resolve `self::` / `super::`.
/// `{pkg}::a::b` → `["a", "b"]`; `{pkg}` → `[]`.
fn initial_modpath(pkg: &str, module_id: &str) -> Vec<String> {
    module_id
        .strip_prefix(pkg)
        .unwrap_or("")
        .split("::")
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

struct Walker<'a> {
    src: &'a [u8],
    pkg: String,
    module_id: String,
    /// Current module nesting (`mod` items push here); starts as the file's
    /// own module path so `super::` at file top level resolves.
    modpath: Vec<String>,
    /// Current item path; symbol id = `{module_id}::{path.join("::")}`.
    path: Vec<String>,
    /// Enclosing symbol ids, innermost last; call `from_id` source.
    enclosing: Vec<String>,
    /// Inside an `impl`/`trait` body: `fn` items are methods.
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
            "function_item" => {
                let name = field_text(node, "name", self.src);
                let body = node.child_by_field_name("body");
                let params = node.child_by_field_name("parameters");
                if let Some(name) = name {
                    let kind = if self.in_assoc {
                        NodeKind::Method
                    } else {
                        NodeKind::Function
                    };
                    let (defines, refs) = self.fn_body_info(params, body);
                    self.with_symbol(kind, &name, node, defines, refs, body, false);
                } else {
                    self.walk_children(node);
                }
            }
            "function_signature_item" => {
                // Trait method declaration (no body). Always a method.
                if let Some(name) = field_text(node, "name", self.src) {
                    let params = node.child_by_field_name("parameters");
                    let (defines, _) = self.fn_body_info(params, None);
                    self.with_symbol(NodeKind::Method, &name, node, defines, vec![], None, false);
                }
            }
            "struct_item" | "enum_item" | "union_item" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    let body = node.child_by_field_name("body");
                    let refs = self.body_refs(body);
                    self.with_symbol(NodeKind::Class, &name, node, vec![], refs, body, false);
                } else {
                    self.walk_children(node);
                }
            }
            "trait_item" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    let body = node.child_by_field_name("body");
                    let refs = self.body_refs(body);
                    // Interface scope: members walked with in_assoc set.
                    self.with_symbol(NodeKind::Interface, &name, node, vec![], refs, body, true);
                } else {
                    self.walk_children(node);
                }
            }
            "impl_item" => self.handle_impl(node),
            "mod_item" => {
                let body = node.child_by_field_name("body");
                match (field_text(node, "name", self.src), body) {
                    (Some(name), Some(body)) => {
                        // Inline module: nest symbol path and module path.
                        self.path.push(name.clone());
                        self.modpath.push(name);
                        self.walk_children(body);
                        self.modpath.pop();
                        self.path.pop();
                    }
                    _ => self.walk_children(node),
                }
            }
            "const_item" | "static_item" => {
                if let Some(name) = field_text(node, "name", self.src) {
                    let value = node.child_by_field_name("value");
                    let refs = self.body_refs(value);
                    self.with_symbol(NodeKind::Variable, &name, node, vec![], refs, value, false);
                } else {
                    self.walk_children(node);
                }
            }
            "use_declaration" => self.handle_use(node),
            "call_expression" => {
                self.handle_call(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: Node) {
        for child in children(node) {
            self.walk(child);
        }
    }

    /// Emit a symbol, walk `body`'s children with it as the enclosing scope,
    /// then restore path/enclosing. When `assoc_scope` is set, `fn` items in
    /// the body become methods.
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

    /// Parameter names → defines; identifier/type references in the body → refs.
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

    /// Best-effort refs for a body with no defines of its own.
    fn body_refs(&self, body: Option<Node>) -> Vec<String> {
        let mut defines = Vec::new();
        let mut refs = Vec::new();
        if let Some(b) = body {
            self.scan(b, &mut defines, &mut refs);
        }
        finalize(&mut defines, &mut refs);
        refs
    }

    /// Collect identifiers bound by a pattern (`let` patterns, fn params).
    /// Only descends through pattern-shaped nodes so types don't leak in.
    fn collect_pattern(&self, node: Node, defines: &mut Vec<String>) {
        match node.kind() {
            "identifier" => {
                if let Some(t) = text_of(node, self.src) {
                    defines.push(t.to_string());
                }
            }
            "self_parameter" => {}
            "parameter" => {
                if let Some(p) = node.child_by_field_name("pattern") {
                    self.collect_pattern(p, defines);
                }
            }
            k if k.ends_with("_pattern") || k == "parameters" => {
                for child in children(node) {
                    self.collect_pattern(child, defines);
                }
            }
            _ => {}
        }
    }

    /// Walk a body collecting `let`-bound names into `defines` and other
    /// identifiers / type names into `refs`. Nested items are skipped: their
    /// bindings belong to their own symbols.
    fn scan(&self, node: Node, defines: &mut Vec<String>, refs: &mut Vec<String>) {
        match node.kind() {
            "function_item" | "function_signature_item" | "struct_item" | "enum_item"
            | "union_item" | "trait_item" | "impl_item" | "mod_item" | "const_item"
            | "static_item" | "use_declaration" => return,
            "let_declaration" => {
                if let Some(pat) = node.child_by_field_name("pattern") {
                    self.collect_pattern(pat, defines);
                }
                // The value and type can still reference outer names.
                for field in ["value", "type"] {
                    if let Some(ch) = node.child_by_field_name(field) {
                        self.scan(ch, defines, refs);
                    }
                }
                return;
            }
            "identifier" | "type_identifier" => {
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

    fn handle_impl(&mut self, node: Node) {
        let trait_node = node.child_by_field_name("trait");
        let body = node.child_by_field_name("body");
        // Self type: the named child that is neither the trait nor the body.
        let mut self_ty = None;
        for child in children(node) {
            if Some(child) == trait_node || Some(child) == body {
                continue;
            }
            match child.kind() {
                "type_parameters" | "where_clause" => continue,
                _ => {
                    self_ty = Some(child);
                    break;
                }
            }
        }
        let name = self_ty
            .and_then(|t| self.base_type_name(t))
            .unwrap_or_else(|| "<unknown>".to_string());
        // The impl block itself is not a symbol; its type name scopes the methods.
        self.path.push(name);
        let saved = self.in_assoc;
        self.in_assoc = true;
        if let Some(b) = body {
            self.walk_children(b);
        }
        self.in_assoc = saved;
        self.path.pop();
    }

    /// `Client` from `Client`, `Client<T>`, or a plain path fallback.
    fn base_type_name(&self, node: Node) -> Option<String> {
        match node.kind() {
            "type_identifier" | "identifier" => {
                text_of(node, self.src).map(|s| s.to_string())
            }
            "generic_type" => node
                .named_child(0)
                .and_then(|c| self.base_type_name(c)),
            "scoped_type_identifier" => node
                .child_by_field_name("name")
                .and_then(|c| self.base_type_name(c)),
            _ => text_of(node, self.src).map(|s| s.trim().to_string()),
        }
    }

    fn handle_call(&mut self, node: Node) {
        let from_id = self
            .enclosing
            .last()
            .cloned()
            .unwrap_or_else(|| self.module_id.clone());
        if let Some(fun) = node.child_by_field_name("function") {
            if let Some(t) = text_of(fun, self.src).map(str::trim) {
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

    // ---------------- `use` handling ----------------

    fn handle_use(&mut self, node: Node) {
        let line = line_of(node);
        let Some(arg) = node.child_by_field_name("argument") else {
            return;
        };
        let mut pairs: Vec<(String, String)> = Vec::new();
        self.expand_use(arg, &UseRoot::Bare, &[], &mut pairs);
        // Group consecutive pairs by target; each becomes one RawImport.
        let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
        for (to, binding) in pairs {
            match grouped.last_mut() {
                Some((t, bindings)) if *t == to => bindings.push(binding),
                _ => grouped.push((to, vec![binding])),
            }
        }
        for (to, mut bindings) in grouped {
            bindings.retain(|b| !b.is_empty());
            self.imports.push(RawImport {
                from_module_id: self.module_id.clone(),
                to_module_id: to,
                bindings,
                line,
            });
        }
    }

    /// Expand a `use` argument into `(to_module_id, binding)` pairs.
    /// `root`/`prefix` carry the path accumulated from enclosing
    /// `scoped_use_list` structure.
    ///
    /// Real tree shapes (verified by parsing, not assumed):
    /// - `use a::b;` → argument = `scoped_identifier`
    /// - `use a::{b, c as d};` → argument = `scoped_use_list`(`a`, `use_list`)
    /// - `use a::*;` → argument = `use_wildcard`(`a`)
    fn expand_use(
        &mut self,
        node: Node,
        root: &UseRoot,
        prefix: &[String],
        pairs: &mut Vec<(String, String)>,
    ) {
        match node.kind() {
            "scoped_use_list" => {
                let mut root = root.clone();
                let mut prefix = prefix.to_vec();
                let mut list = None;
                for child in children(node) {
                    if child.kind() == "use_list" {
                        list = Some(child);
                        continue;
                    }
                    let (r, segs, _) = self.path_of(child);
                    root = merge_root(&r, &root);
                    prefix.extend(segs);
                }
                if let Some(l) = list {
                    self.expand_list(l, &root, &prefix, pairs);
                } else {
                    // No list (shouldn't happen); treat the path as a plain import.
                    let to = self.resolve_use(&root, &prefix);
                    let binding = prefix.last().cloned().unwrap_or_default();
                    pairs.push((to, binding));
                }
            }
            "use_list" => self.expand_list(node, root, prefix, pairs),
            "use_wildcard" => {
                // The wildcard wraps its path: `use a::*` → use_wildcard(`a`).
                let mut root = root.clone();
                let mut prefix = prefix.to_vec();
                for child in children(node) {
                    let (r, segs, _) = self.path_of(child);
                    root = merge_root(&r, &root);
                    prefix.extend(segs);
                }
                pairs.push((self.resolve_use(&root, &prefix), String::new()));
            }
            _ => self.expand_use_leaf(node, root, prefix, pairs),
        }
    }

    fn expand_list(
        &mut self,
        list: Node,
        root: &UseRoot,
        prefix: &[String],
        pairs: &mut Vec<(String, String)>,
    ) {
        for child in children(list) {
            match child.kind() {
                "use_list" | "scoped_use_list" | "use_wildcard" => {
                    self.expand_use(child, root, prefix, pairs)
                }
                _ => self.expand_use_leaf(child, root, prefix, pairs),
            }
        }
    }

    fn expand_use_leaf(
        &mut self,
        node: Node,
        root: &UseRoot,
        prefix: &[String],
        pairs: &mut Vec<(String, String)>,
    ) {
        let (r, segs, alias) = self.path_of(node);
        let root = merge_root(&r, root);
        let mut full: Vec<String> = prefix.to_vec();
        full.extend(segs);
        let to = self.resolve_use(&root, &full);
        let binding = alias.or_else(|| full.last().cloned()).unwrap_or_default();
        pairs.push((to, binding));
    }

    /// `(root, segments, alias)` for the non-list part of a use tree.
    fn path_of(&self, node: Node) -> (UseRoot, Vec<String>, Option<String>) {
        match node.kind() {
            "use_as_clause" => {
                let alias = field_text(node, "alias", self.src);
                let (root, segs, _) = node
                    .child_by_field_name("path")
                    .map(|p| self.path_of(p))
                    .unwrap_or((UseRoot::Bare, Vec::new(), None));
                (root, segs, alias)
            }
            "scoped_identifier" => {
                let (root, mut segs, alias) = node
                    .child_by_field_name("path")
                    .map(|p| self.path_of(p))
                    .unwrap_or((UseRoot::Bare, Vec::new(), None));
                if let Some(n) = node.child_by_field_name("name") {
                    match n.kind() {
                        // List/wildcard tails are handled by the caller.
                        "use_list" | "use_wildcard" => {}
                        _ => {
                            if let Some(t) = text_of(n, self.src) {
                                segs.push(t.to_string());
                            }
                        }
                    }
                }
                (root, segs, alias)
            }
            "identifier" => (
                UseRoot::Bare,
                vec![text_of(node, self.src).unwrap_or("").to_string()],
                None,
            ),
            "crate" => (UseRoot::Crate, Vec::new(), None),
            "self" => (UseRoot::Slf, Vec::new(), None),
            "super" => (UseRoot::Super, Vec::new(), None),
            _ => (UseRoot::Bare, Vec::new(), None),
        }
    }

    fn resolve_use(&self, root: &UseRoot, segs: &[String]) -> String {
        match root {
            UseRoot::Crate => join_id(&self.pkg, segs),
            UseRoot::Slf => {
                let full: Vec<String> =
                    self.modpath.iter().cloned().chain(segs.iter().cloned()).collect();
                join_id(&self.pkg, &full)
            }
            UseRoot::Super => {
                if self.modpath.is_empty() {
                    "external::unresolved".to_string()
                } else {
                    let full: Vec<String> = self.modpath[..self.modpath.len() - 1]
                        .iter()
                        .cloned()
                        .chain(segs.iter().cloned())
                        .collect();
                    join_id(&self.pkg, &full)
                }
            }
            UseRoot::Bare => match segs.first() {
                Some(first) if first == &self.pkg => join_id(&self.pkg, &segs[1..]),
                Some(first) => format!("external::{first}"),
                None => "external::unresolved".to_string(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum UseRoot {
    Crate,
    Slf,
    Super,
    Bare,
}

/// An explicitly parsed root (`crate`/`self`/`super`) wins over an inherited one.
fn merge_root(parsed: &UseRoot, inherited: &UseRoot) -> UseRoot {
    match parsed {
        UseRoot::Bare => inherited.clone(),
        other => other.clone(),
    }
}

fn join_id(pkg: &str, segs: &[String]) -> String {
    if segs.is_empty() {
        pkg.to_string()
    } else {
        format!("{}::{}", pkg, segs.join("::"))
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

    const SRC: &str = r#"use crate::config::Config;
use std::time::Duration;

pub struct Client {
    timeout: Duration,
}

impl Client {
    pub fn new(timeout: Duration) -> Self {
        let cfg = Config::default();
        Client { timeout }
    }

    pub fn connect(&self) -> bool {
        let stream = TcpStream::connect("127.0.0.1:8080");
        stream.set_timeout(self.timeout);
        true
    }
}

pub fn ping(c: &Client) -> bool {
    c.connect()
}
"#;

    fn ids(fs: &FileSymbols) -> Vec<(&str, NodeKind)> {
        fs.symbols.iter().map(|s| (s.id.as_str(), s.kind)).collect()
    }

    #[test]
    fn rust_module_id_rules() {
        let cases = [
            ("src/net.rs", "mycrate::net"),
            ("src/lib.rs", "mycrate"),
            ("src/main.rs", "mycrate::main"),
            ("src/mod.rs", "mycrate"),
            ("src/a/mod.rs", "mycrate::a"),
            ("src/a/b.rs", "mycrate::a::b"),
            ("tests/it.rs", "mycrate::tests::it"),
            ("build.rs", "mycrate::build"),
        ];
        for (rel, want) in cases {
            assert_eq!(module_id_for("mycrate", Path::new(rel)), want, "{rel}");
        }
    }

    #[test]
    fn rust_symbols_ids_and_kinds() {
        let fs = extract_file("mycrate", Path::new("src/client.rs"), SRC).unwrap();
        assert_eq!(fs.module_id, "mycrate::client");
        assert_eq!(
            ids(&fs),
            vec![
                ("mycrate::client::Client", NodeKind::Class),
                ("mycrate::client::Client::new", NodeKind::Method),
                ("mycrate::client::Client::connect", NodeKind::Method),
                ("mycrate::client::ping", NodeKind::Function),
            ]
        );
    }

    #[test]
    fn rust_imports() {
        let fs = extract_file("mycrate", Path::new("src/client.rs"), SRC).unwrap();
        assert_eq!(fs.imports.len(), 2);
        assert_eq!(fs.imports[0].from_module_id, "mycrate::client");
        // `use crate::a::b` -> {pkg}::a::b (full path, per the ID rules).
        assert_eq!(fs.imports[0].to_module_id, "mycrate::config::Config");
        assert_eq!(fs.imports[0].bindings, vec!["Config"]);
        assert_eq!(fs.imports[0].line, 1);
        assert_eq!(fs.imports[1].to_module_id, "external::std");
        assert_eq!(fs.imports[1].bindings, vec!["Duration"]);
    }

    #[test]
    fn rust_calls() {
        let fs = extract_file("mycrate", Path::new("src/client.rs"), SRC).unwrap();
        let calls: Vec<(&str, &str)> = fs
            .calls
            .iter()
            .map(|c| (c.from_id.as_str(), c.to_name.as_str()))
            .collect();
        assert!(calls.contains(&("mycrate::client::Client::new", "Config::default")), "{calls:?}");
        assert!(calls.contains(&("mycrate::client::Client::connect", "TcpStream::connect")), "{calls:?}");
        assert!(calls.contains(&("mycrate::client::ping", "c.connect")), "{calls:?}");
        // `stream.set_timeout(...)` is also a call, from the method.
        assert!(calls.contains(&("mycrate::client::Client::connect", "stream.set_timeout")), "{calls:?}");
    }

    #[test]
    fn rust_defines_and_refs() {
        let fs = extract_file("mycrate", Path::new("src/client.rs"), SRC).unwrap();
        let by_id = |id: &str| fs.symbols.iter().find(|s| s.id == id).unwrap();
        let new = by_id("mycrate::client::Client::new");
        assert!(new.defines.contains(&"timeout".to_string()), "{:?}", new.defines);
        assert!(new.defines.contains(&"cfg".to_string()), "{:?}", new.defines);
        assert!(new.refs.contains(&"Config".to_string()), "{:?}", new.refs);
        let connect = by_id("mycrate::client::Client::connect");
        assert!(connect.defines.contains(&"stream".to_string()), "{:?}", connect.defines);
        assert!(connect.refs.contains(&"TcpStream".to_string()), "{:?}", connect.refs);
        // A defined name is not also reported as a ref.
        assert!(!connect.refs.contains(&"stream".to_string()));
    }

    #[test]
    fn rust_use_shapes() {
        let src = r#"use a::{b, c as d};
use a::*;
use super::sib;
use self::here;
use mycrate::top::Thing;
"#;
        let fs = extract_file("mycrate", Path::new("src/mid/inner.rs"), src).unwrap();
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
        // `a` is not the package: external.
        assert!(tos.contains(&("external::a", vec!["b", "d"])), "{tos:?}");
        assert!(tos.contains(&("external::a", vec![])), "{tos:?}");
        // super::sib from mycrate::mid::inner -> mycrate::mid::sib
        assert!(tos.contains(&("mycrate::mid::sib", vec!["sib"])), "{tos:?}");
        // self::here -> mycrate::mid::inner::here
        assert!(tos.contains(&("mycrate::mid::inner::here", vec!["here"])), "{tos:?}");
        // explicit crate name == package -> self-crate
        assert!(tos.contains(&("mycrate::top::Thing", vec!["Thing"])), "{tos:?}");
    }

    #[test]
    fn rust_trait_and_const() {
        let src = r#"pub trait Greeter {
    fn hello(&self) -> String;
}

pub const LIMIT: usize = 10;
"#;
        let fs = extract_file("mycrate", Path::new("src/greet.rs"), src).unwrap();
        assert_eq!(
            ids(&fs),
            vec![
                ("mycrate::greet::Greeter", NodeKind::Interface),
                ("mycrate::greet::Greeter::hello", NodeKind::Method),
                ("mycrate::greet::LIMIT", NodeKind::Variable),
            ]
        );
    }

    #[test]
    fn rust_nested_mod() {
        let src = "pub mod inner {\n    pub fn f() {}\n}\n";
        let fs = extract_file("mycrate", Path::new("src/m.rs"), src).unwrap();
        assert_eq!(
            ids(&fs),
            vec![("mycrate::m::inner::f", NodeKind::Function),]
        );
    }
}
