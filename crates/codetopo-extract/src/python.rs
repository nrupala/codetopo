// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

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
//! `attribute`), `block`, `except_clause` (fields `value`, `alias`),
//! `type_alias_statement` (fields `left`, `right`). One delta against
//! the spec pass's research copy of the grammar: `tuple_expression`
//! exists there but not in the pinned 0.25.0; nothing in the mapping
//! depends on it.
//!
//! The mapping is specified in
//! `~/workspace/codetopo-work/python-extractor/SPEC.md` §3: module ids
//! (§3.1, the `__init__` collapse mirrors the TS `index` rule), symbols
//! (§3.2), the defines/refs/writes scan (§3.6, the marimo discipline
//! shared with the TS/Rust walkers — nested declarations pruned,
//! reads that are also defines dropped), imports (§3.3), calls
//! (§3.4), and heritage (§3.5, Inherits only; `Protocol` bases flip
//! the class kind to `interface` per decision D2). The measured
//! fields (§3.7) land in the following part of the series.

use std::collections::HashSet;
use std::path::Path;

use codetopo_core::emit::{FileSymbols, RawCall, RawHeritage, RawImport, RawSymbol};
use codetopo_core::schema::{EdgeKind, NodeKind};

use crate::{
    children, display_path, end_line_of, line_of, measured_fields, signature_of, text_of,
    ExtractError,
};

type Node<'a> = tree_sitter::Node<'a>;

/// Node text as an owned string (empty on invalid UTF-8, which the
/// walk tolerates the way the TS/Rust walkers tolerate partial parses).
fn txt(node: Node, src: &[u8]) -> String {
    text_of(node, src).unwrap_or("").to_string()
}

pub(crate) fn extract(package: &str, rel_path: &Path, source: &str) -> Result<FileSymbols, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;
    let module_id = module_id_for(package, rel_path);
    let mut w = Walker {
        src: source.as_bytes(),
        pkg: package.to_string(),
        module_id: module_id.clone(),
        is_init: rel_path.file_stem().map(|s| s == "__init__").unwrap_or(false),
        path: Vec::new(),
        enclosing: Vec::new(),
        in_class: false,
        pending_refs: Vec::new(),
        symbols: Vec::new(),
        imports: Vec::new(),
        calls: Vec::new(),
        heritage: Vec::new(),
    };
    w.walk(tree.root_node());
    Ok(FileSymbols {
        package: package.to_string(),
        file: display_path(rel_path),
        module_id,
        symbols: w.symbols,
        imports: w.imports,
        calls: w.calls,
        heritage: w.heritage,
    })
}

/// `__init__.py` collapses to its package path; other files append
/// their stem (spec §3.1). `.pyi` files follow the same rule.
fn module_id_for(package: &str, rel_path: &Path) -> String {
    let mut parts: Vec<String> = vec![package.to_string()];
    if let Some(parent) = rel_path.parent() {
        for comp in parent.components() {
            parts.push(comp.as_os_str().to_string_lossy().into_owned());
        }
    }
    let stem = rel_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if stem != "__init__" {
        parts.push(stem);
    }
    parts.join("::")
}

struct Walker<'a> {
    src: &'a [u8],
    pkg: String,
    module_id: String,
    /// True when the file is `__init__.py`/`__init__.pyi` — its module
    /// id already IS its package (relative-import resolution, §3.3).
    is_init: bool,
    /// Symbol-name path segments: class → method → nested fn → …
    path: Vec<String>,
    /// Enclosing symbol ids.
    enclosing: Vec<String>,
    /// True while walking a class body directly (defs become methods).
    in_class: bool,
    /// Refs from bare decorators (`@cache`, `@x.setter`), collected by
    /// the decorated-definition arm and merged into the symbol it
    /// pushes next (spec D3).
    pending_refs: Vec<String>,
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
    calls: Vec<RawCall>,
    heritage: Vec<RawHeritage>,
}

impl<'a> Walker<'a> {
    fn text(&self, n: Node) -> String {
        txt(n, self.src)
    }

    fn walk_children(&mut self, node: Node) {
        for i in 0..node.named_child_count() {
            if let Some(c) = node.named_child(i) {
                self.walk(c);
            }
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => self.handle_function(node),
            "class_definition" => self.handle_class(node),
            "decorated_definition" => {
                // Decorators are evaluated in the ENCLOSING scope
                // (spec D3): their calls record against it, and a bare
                // decorator (name or attribute, not a call) becomes a
                // ref on the decorated symbol. The symbol itself is
                // the inner definition — line and span come from it.
                for i in 0..node.named_child_count() {
                    if let Some(c) = node.named_child(i) {
                        if c.kind() == "decorator" {
                            self.handle_decorator(c);
                        }
                    }
                }
                if let Some(def) = node.child_by_field_name("definition") {
                    match def.kind() {
                        "function_definition" => self.handle_function(def),
                        "class_definition" => self.handle_class(def),
                        _ => self.walk_children(node),
                    }
                }
            }
            "type_alias_statement" => self.handle_type_alias(node),
            "assignment" if self.enclosing.is_empty() => self.handle_module_assignment(node),
            "import_statement" => self.handle_import(node),
            "future_import_statement" => self.handle_future_import(node),
            "import_from_statement" => self.handle_from_import(node),
            "call" => {
                self.record_call(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    /// The id a call/import is attributed to: the enclosing symbol,
    /// or the module itself at top level.
    fn current_scope(&self) -> String {
        self.enclosing
            .last()
            .cloned()
            .unwrap_or_else(|| self.module_id.clone())
    }

    /// One `call` node → one RawCall; `to_name` is the callee text as
    /// written, trimmed (spec §3.4 — the builder suffix-matches it).
    fn record_call(&mut self, node: Node) {
        if let Some(f) = node.child_by_field_name("function") {
            let t = self.text(f).trim().to_string();
            if !t.is_empty() {
                self.calls.push(RawCall {
                    from_id: self.current_scope(),
                    to_name: t,
                    line: line_of(node),
                });
            }
        }
    }

    /// A `decorator` node: a call inside it records against the
    /// enclosing scope; a bare name/attribute becomes a pending ref
    /// on the decorated symbol (spec D3).
    fn handle_decorator(&mut self, node: Node) {
        for i in 0..node.named_child_count() {
            if let Some(expr) = node.named_child(i) {
                match expr.kind() {
                    "call" => self.walk(expr),
                    "identifier" => self.pending_refs.push(self.text(expr)),
                    "attribute" => {
                        if let Some(attr) = expr.child_by_field_name("attribute") {
                            self.pending_refs.push(self.text(attr));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Consume the pending decorator refs, merging them into a
    /// symbol's ref list (deduped, define-filtered, sorted).
    fn take_pending_refs(&mut self, refs: &mut Vec<String>, defines: &[String]) {
        if self.pending_refs.is_empty() {
            return;
        }
        let mut set: HashSet<String> = refs.iter().cloned().collect();
        for r in std::mem::take(&mut self.pending_refs) {
            if !defines.contains(&r) {
                set.insert(r);
            }
        }
        *refs = set.into_iter().collect();
        refs.sort();
    }

    /// Push a symbol record; the caller walks the body for nested
    /// symbols between `push_symbol` and `pop_symbol`. The measured
    /// fields are computed by the caller (they differ per symbol
    /// kind — §3.7) and passed in.
    #[allow(clippy::too_many_arguments)]
    fn push_symbol(
        &mut self,
        kind: NodeKind,
        name: &str,
        node: Node,
        defines: Vec<String>,
        refs: Vec<String>,
        writes: Vec<String>,
        end_line: Option<u32>,
        signature: Option<String>,
        decisions: Option<u32>,
    ) {
        self.path.push(name.to_string());
        let id = format!("{}::{}", self.module_id, self.path.join("::"));
        self.symbols.push(RawSymbol {
            id: id.clone(),
            kind,
            label: name.to_string(),
            line: line_of(node),
            column: None,
            end_line,
            signature,
            decisions,
            defines,
            refs,
            writes,
        });
        self.enclosing.push(id);
    }

    fn pop_symbol(&mut self) {
        self.enclosing.pop();
        self.path.pop();
    }

    fn handle_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let kind = if self.in_class {
            NodeKind::Method
        } else {
            NodeKind::Function
        };
        let (defines, mut refs, writes) = fn_body_info(
            self.src,
            node.child_by_field_name("parameters"),
            node.child_by_field_name("body"),
        );
        // Return annotation names are refs of the function (spec D5).
        if let Some(rt) = node.child_by_field_name("return_type") {
            let mut set: HashSet<String> = refs.iter().cloned().collect();
            scan_type_refs(rt, self.src, &mut set);
            refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
            refs.sort();
        }
        self.take_pending_refs(&mut refs, &defines);
        // Default values are evaluated at definition time, in the
        // enclosing scope: their calls belong to that scope (§3.4).
        if let Some(p) = node.child_by_field_name("parameters") {
            self.walk_children(p);
        }
        let (end_line, signature) = py_measured(node, self.src);
        let decisions = node.child_by_field_name("body").map(count_decisions);
        self.push_symbol(
            kind,
            &name,
            node,
            defines,
            refs,
            writes,
            end_line,
            signature,
            decisions,
        );
        let prev_class = self.in_class;
        self.in_class = false;
        if let Some(body) = node.child_by_field_name("body") {
            self.walk_children(body);
        }
        self.in_class = prev_class;
        self.pop_symbol();
    }

    fn handle_class(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        // Heritage (§3.5): base names off the superclasses list; a
        // `Protocol` base (plain, `typing.Protocol`, or subscripted)
        // flips the kind to interface (decision D2).
        let mut targets: Vec<String> = Vec::new();
        if let Some(supers) = node.child_by_field_name("superclasses") {
            // Base expressions are evaluated in the enclosing scope.
            self.walk_children(supers);
            for i in 0..supers.named_child_count() {
                if let Some(c) = supers.named_child(i) {
                    if let Some(t) = base_name(c, self.src) {
                        targets.push(t);
                    }
                }
            }
        }
        let kind = if targets.iter().any(|t| t == "Protocol") {
            NodeKind::Interface
        } else {
            NodeKind::Class
        };
        for t in &targets {
            self.heritage.push(RawHeritage {
                type_name: name.clone(),
                target_name: t.clone(),
                kind: EdgeKind::Inherits,
                line: line_of(node),
            });
        }
        let body = node.child_by_field_name("body");
        let mut refs = body.map(|b| body_refs(b, self.src)).unwrap_or_default();
        self.take_pending_refs(&mut refs, &[]);
        let (end_line, signature) = py_measured(node, self.src);
        self.push_symbol(
            kind,
            &name,
            node,
            Vec::new(),
            refs,
            Vec::new(),
            end_line,
            signature,
            None,
        );
        let prev_class = self.in_class;
        self.in_class = true;
        if let Some(b) = body {
            self.walk_children(b);
        }
        self.in_class = prev_class;
        self.pop_symbol();
    }

    /// PEP 695 `type X = …` — an alias is a class-kind symbol (spec §3.2).
    fn handle_type_alias(&mut self, node: Node) {
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        let name = self.text(left);
        let mut set = HashSet::new();
        if let Some(right) = node.child_by_field_name("right") {
            scan_type_refs(right, self.src, &mut set);
        }
        let mut refs: Vec<String> = set.into_iter().collect();
        refs.sort();
        let (end_line, signature) = py_measured(node, self.src);
        self.push_symbol(
            NodeKind::Class,
            &name,
            node,
            Vec::new(),
            refs,
            Vec::new(),
            end_line,
            signature,
            None,
        );
        self.pop_symbol();
    }

    /// Module-level plain-name assignments are `variable` symbols
    /// (spec §3.2). Chained `a = b = …` yields one symbol per name;
    /// pattern/attribute/subscript targets yield none (the TS
    /// destructuring precedent).
    fn handle_module_assignment(&mut self, mut node: Node) {
        let final_right: Option<Node> = loop {
            let left = node.child_by_field_name("left");
            let right = node.child_by_field_name("right");
            if let Some(l) = left {
                if l.kind() == "identifier" {
                    let name = self.text(l);
                    let mut set: HashSet<String> = HashSet::new();
                    if let Some(r) = right {
                        if r.kind() != "assignment" {
                            set.extend(body_refs(r, self.src));
                        }
                    }
                    if let Some(t) = node.child_by_field_name("type") {
                        scan_type_refs(t, self.src, &mut set);
                    }
                    let mut refs: Vec<String> = set.into_iter().collect();
                    refs.sort();
                    // The variable's signature cut is the assignment's
                    // `right` field — Python's field name, which the
                    // shared measured_fields does not look for (§3.7):
                    // the initializer never appears (`X: int`, never
                    // `X: int = 3`).
                    let signature = signature_of(node, right, self.src);
                    self.push_symbol(
                        NodeKind::Variable,
                        &name,
                        node,
                        Vec::new(),
                        refs,
                        Vec::new(),
                        Some(end_line_of(node)),
                        signature,
                        None,
                    );
                    self.pop_symbol();
                }
            }
            match right {
                Some(r) if r.kind() == "assignment" => node = r,
                _ => break right,
            }
        };
        // Calls in the initializer belong to the module (§3.4); the
        // assignment arm consumed the statement, so walk the final
        // right-hand side for them explicitly.
        if let Some(r) = final_right {
            self.walk(r);
        }
    }

    /// Resolve a dotted module path to a module id (spec §3.3, the
    /// Rust walker's bare-root rule): the first segment decides — the
    /// workspace package's own name means internal (the segment IS
    /// the package segment of the id), anything else is
    /// `external::{first}`.
    fn resolve_dotted(&self, dotted: &str) -> String {
        let segs: Vec<&str> = dotted.split('.').filter(|s| !s.is_empty()).collect();
        match segs.first() {
            Some(first) if *first == self.pkg => {
                if segs.len() == 1 {
                    self.pkg.clone()
                } else {
                    format!("{}::{}", self.pkg, segs[1..].join("::"))
                }
            }
            Some(first) => format!("external::{first}"),
            None => "external::unresolved".to_string(),
        }
    }

    /// `import a.b.c` / `import a.b as d` — one RawImport per dotted
    /// name (spec §3.3); the plain form binds the first segment, the
    /// aliased form binds the alias.
    fn handle_import(&mut self, node: Node) {
        let line = line_of(node);
        for i in 0..node.named_child_count() {
            let Some(c) = node.named_child(i) else { continue };
            match c.kind() {
                "dotted_name" => {
                    let dotted = self.text(c);
                    let binding = dotted.split('.').next().unwrap_or("").to_string();
                    self.imports.push(RawImport {
                        from_module_id: self.module_id.clone(),
                        to_module_id: self.resolve_dotted(&dotted),
                        bindings: vec![binding],
                        line,
                    });
                }
                "aliased_import" => {
                    if let Some(n) = c.child_by_field_name("name") {
                        let dotted = self.text(n);
                        let binding = c
                            .child_by_field_name("alias")
                            .map(|a| self.text(a))
                            .unwrap_or_default();
                        self.imports.push(RawImport {
                            from_module_id: self.module_id.clone(),
                            to_module_id: self.resolve_dotted(&dotted),
                            bindings: vec![binding],
                            line,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    /// `from __future__ import annotations` parses as its own node
    /// kind in this grammar; the emitted row is the spec table's —
    /// `external::__future__`, bindings as written, no special case
    /// in the output (§3.3).
    fn handle_future_import(&mut self, node: Node) {
        let mut bindings: Vec<String> = Vec::new();
        for i in 0..node.named_child_count() {
            if let Some(c) = node.named_child(i) {
                if c.kind() == "dotted_name" {
                    bindings.push(self.text(c));
                }
            }
        }
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: "external::__future__".to_string(),
            bindings,
            line: line_of(node),
        });
    }

    /// `from X import a, b as c` / `from .rel import …` — one
    /// RawImport for the statement, bindings = the local names
    /// (alias wins); a glob import binds nothing (spec §3.3).
    fn handle_from_import(&mut self, node: Node) {
        let line = line_of(node);
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return;
        };
        let to = match module_node.kind() {
            "dotted_name" => self.resolve_dotted(&self.text(module_node)),
            "relative_import" => self.resolve_relative(module_node),
            _ => return,
        };
        let mut bindings: Vec<String> = Vec::new();
        let mut glob = false;
        for i in 0..node.named_child_count() {
            let Some(c) = node.named_child(i) else { continue };
            if c.id() == module_node.id() {
                continue;
            }
            match c.kind() {
                "dotted_name" | "identifier" => bindings.push(self.text(c)),
                "aliased_import" => {
                    if let Some(a) = c.child_by_field_name("alias") {
                        bindings.push(self.text(a));
                    }
                }
                "wildcard_import" => glob = true,
                _ => {}
            }
        }
        if glob {
            bindings.clear();
        }
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings,
            line,
        });
    }

    /// Relative imports resolve on module-id segments (spec §3.3):
    /// the importing module's package is its id minus the last
    /// segment — except `__init__.py`, whose id already is its
    /// package. One dot = that package; each extra dot ascends one
    /// segment; ascending past the root segment is
    /// `external::unresolved`. Purely syntactic.
    fn resolve_relative(&self, node: Node) -> String {
        let text = self.text(node);
        let trimmed = text.trim();
        let dots = trimmed.chars().take_while(|c| *c == '.').count();
        let rest = trimmed[dots..].trim();
        let segs: Vec<&str> = self.module_id.split("::").collect();
        let package_len = if self.is_init {
            segs.len()
        } else {
            segs.len().saturating_sub(1)
        };
        let ascend = dots.saturating_sub(1);
        if ascend >= package_len {
            return "external::unresolved".to_string();
        }
        let mut out: Vec<&str> = segs[..package_len - ascend].to_vec();
        if !rest.is_empty() {
            out.extend(rest.split('.').filter(|s| !s.is_empty()));
        }
        out.join("::")
    }
}

/// The name a class base contributes (spec §3.5): a plain identifier,
/// the attribute of a dotted name (`pkg.Base` → `Base`), or the value
/// of a subscript (`Generic[T]` → `Generic`, recursively). Keyword
/// arguments and call bases yield nothing.
fn base_name(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" => Some(txt(node, src)),
        "attribute" => node
            .child_by_field_name("attribute")
            .map(|a| txt(a, src)),
        "subscript" => node
            .child_by_field_name("value")
            .and_then(|v| base_name(v, src)),
        _ => None,
    }
}

/// (end_line, signature) for a definition node, via the shared
/// measured-fields helper — plus one Python adjustment: a def/class
/// header ends with a colon the shared strip set does not cover, so
/// a single trailing `:` is removed (`def f(x) -> int`, not
/// `def f(x) -> int:`).
fn py_measured(node: Node, src: &[u8]) -> (Option<u32>, Option<String>) {
    let (end_line, signature) = measured_fields(node, src);
    let signature = signature.map(|sig| match sig.strip_suffix(':') {
        Some(stripped) => stripped.trim_end().to_string(),
        None => sig,
    });
    (end_line, signature)
}

/// Decision points in a body, per the Python enumeration of
/// CODE_SCHEMA §3.2 (spec §3.7): if +1, each elif +1; for/while +1;
/// each except clause +1; conditional expression +1; each boolean
/// operator +1; comprehension for/if clauses +1 each; match
/// contributes (cases − 1) with guards not counted. Nested
/// declarations are pruned — their decisions belong to their own
/// symbols; lambdas and comprehensions are descended into.
fn count_decisions(node: Node) -> u32 {
    match node.kind() {
        "function_definition" | "class_definition" | "decorated_definition" => return 0,
        _ => {}
    }
    let mut n = match node.kind() {
        "if_statement" | "elif_clause" | "for_statement" | "while_statement" | "except_clause"
        | "conditional_expression" | "boolean_operator" | "for_in_clause" | "if_clause" => 1,
        "match_statement" => node
            .child_by_field_name("body")
            .map(|b| {
                children(b)
                    .iter()
                    .filter(|c| c.kind() == "case_clause")
                    .count() as u32
            })
            .unwrap_or(0)
            .saturating_sub(1),
        _ => 0,
    };
    let guard = if node.kind() == "case_clause" {
        node.child_by_field_name("guard")
    } else {
        None
    };
    for child in children(node) {
        // A case guard is an if_clause node but is NOT counted
        // (spec §3.7 — the switch-cases precedent).
        if let Some(g) = guard {
            if g.id() == child.id() {
                continue;
            }
        }
        n += count_decisions(child);
    }
    n
}

/// Accumulators for the defines/refs/writes scan.
#[derive(Default)]
struct Acc {
    defines: HashSet<String>,
    refs: HashSet<String>,
    writes: HashSet<String>,
    /// Names declared `global`/`nonlocal` in the scanned body: their
    /// defines are dropped at finalize time (spec §3.6).
    excluded: HashSet<String>,
}

/// Collect the binding names of a pattern (parameter or target):
/// plain identifiers and every name inside list/tuple/splat patterns.
fn collect_pattern(node: Node, src: &[u8], out: &mut HashSet<String>) {
    if node.kind() == "identifier" {
        out.insert(txt(node, src));
        return;
    }
    for i in 0..node.named_child_count() {
        if let Some(c) = node.named_child(i) {
            collect_pattern(c, src, out);
        }
    }
}

/// Every identifier inside a type expression is a ref (spec D5 —
/// annotation names are reads of the names they mention).
fn scan_type_refs(node: Node, src: &[u8], out: &mut HashSet<String>) {
    if node.kind() == "identifier" {
        out.insert(txt(node, src));
        return;
    }
    for i in 0..node.named_child_count() {
        if let Some(c) = node.named_child(i) {
            scan_type_refs(c, src, out);
        }
    }
}

/// One parameter node → its binding names (defines) plus, for typed /
/// defaulted parameters, the annotation names (refs) and the default
/// value's reads.
fn collect_param(node: Node, src: &[u8], acc: &mut Acc) {
    match node.kind() {
        "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
            if let Some(n) = node.child_by_field_name("name") {
                collect_pattern(n, src, &mut acc.defines);
            } else if let Some(first) = node.named_child(0) {
                collect_pattern(first, src, &mut acc.defines);
            }
            if let Some(t) = node.child_by_field_name("type") {
                scan_type_refs(t, src, &mut acc.refs);
            }
            if let Some(v) = node.child_by_field_name("value") {
                scan_w(v, src, acc);
            }
        }
        _ => collect_pattern(node, src, &mut acc.defines),
    }
}

/// Record an assignment/for target. Plain names define AND write
/// (spec §3.6 lists them under both); attribute targets write their
/// attribute name; pattern targets fan out to their names; subscript
/// targets are not recorded. `binding_only` (for/with/comprehension/
/// walrus targets) defines without writing.
fn record_target(node: Node, src: &[u8], acc: &mut Acc, binding_only: bool) {
    match node.kind() {
        "identifier" => {
            let name = txt(node, src);
            if acc.excluded.contains(&name) {
                // global/nonlocal: a write, never a local define.
                acc.writes.insert(name);
            } else {
                acc.defines.insert(name.clone());
                if !binding_only {
                    acc.writes.insert(name);
                }
            }
        }
        "attribute" => {
            if !binding_only {
                if let Some(attr) = node.child_by_field_name("attribute") {
                    acc.writes.insert(txt(attr, src));
                }
            }
        }
        "list_pattern" | "tuple_pattern" | "list_splat_pattern" | "dictionary_splat_pattern"
        | "splat_pattern" => {
            let mut names = HashSet::new();
            collect_pattern(node, src, &mut names);
            for n in names {
                if acc.excluded.contains(&n) {
                    acc.writes.insert(n);
                } else {
                    acc.defines.insert(n.clone());
                    if !binding_only {
                        acc.writes.insert(n);
                    }
                }
            }
        }
        // Subscript and anything else: not recorded (spec §3.6).
        _ => {}
    }
}

/// The marimo scan over a body: collect defines / refs / writes with
/// nested declarations pruned (spec §3.6).
fn scan_w(node: Node, src: &[u8], acc: &mut Acc) {
    match node.kind() {
        // Nested declarations are their own symbols.
        "function_definition" | "class_definition" | "decorated_definition" | "import_statement"
        | "import_from_statement" => {}
        "identifier" => {
            acc.refs.insert(txt(node, src));
        }
        // Only the root of an attribute chain is a read (spec §3.6).
        "attribute" => {
            if let Some(obj) = node.child_by_field_name("object") {
                scan_w(obj, src, acc);
            }
        }
        "assignment" => {
            if let Some(t) = node.child_by_field_name("type") {
                scan_type_refs(t, src, &mut acc.refs);
            }
            if let Some(l) = node.child_by_field_name("left") {
                record_target(l, src, acc, false);
            }
            if let Some(r) = node.child_by_field_name("right") {
                scan_w(r, src, acc);
            }
        }
        // Augmented assignment: the target is READ, not written
        // (spec §3.6, the disclosed limit shared with §9).
        "augmented_assignment" => {
            if let Some(l) = node.child_by_field_name("left") {
                scan_w(l, src, acc);
            }
            if let Some(r) = node.child_by_field_name("right") {
                scan_w(r, src, acc);
            }
        }
        "for_statement" => {
            if let Some(l) = node.child_by_field_name("left") {
                record_target(l, src, acc, true);
            }
            for f in ["right", "body", "alternative"] {
                if let Some(n) = node.child_by_field_name(f) {
                    scan_w(n, src, acc);
                }
            }
        }
        "with_item" => {
            // `open(…) as fh`: the alias field binds; anything else
            // in the item is read.
            let alias = node.child_by_field_name("alias");
            if let Some(a) = alias {
                collect_pattern(a, src, &mut acc.defines);
            }
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if alias.map(|a| a.id() != c.id()).unwrap_or(true) {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "as_pattern" => {
            // `expr as name` where the grammar wraps it (match arms,
            // some with/except shapes): expression read, alias binds.
            let alias = node.child_by_field_name("alias");
            if let Some(a) = alias {
                collect_pattern(a, src, &mut acc.defines);
            }
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if alias.map(|a| a.id() != c.id()).unwrap_or(true) {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "except_clause" => {
            if let Some(v) = node.child_by_field_name("value") {
                scan_w(v, src, acc);
            }
            if let Some(a) = node.child_by_field_name("alias") {
                collect_pattern(a, src, &mut acc.defines);
            }
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if c.kind() == "block" {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "list_comprehension" | "dictionary_comprehension" | "set_comprehension"
        | "generator_expression" => {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if c.kind() == "for_in_clause" {
                        if let Some(l) = c.child_by_field_name("left") {
                            record_target(l, src, acc, true);
                        }
                        if let Some(r) = c.child_by_field_name("right") {
                            scan_w(r, src, acc);
                        }
                    } else {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "named_expression" => {
            if let Some(n) = node.child_by_field_name("name") {
                record_target(n, src, acc, true);
            }
            if let Some(v) = node.child_by_field_name("value") {
                scan_w(v, src, acc);
            }
        }
        "global_statement" | "nonlocal_statement" => {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if c.kind() == "identifier" {
                        acc.excluded.insert(txt(c, src));
                    }
                }
            }
        }
        "lambda" => {
            if let Some(p) = node.child_by_field_name("parameters") {
                for i in 0..p.named_child_count() {
                    if let Some(c) = p.named_child(i) {
                        collect_param(c, src, acc);
                    }
                }
            }
            if let Some(b) = node.child_by_field_name("body") {
                scan_w(b, src, acc);
            }
        }
        _ => {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    scan_w(c, src, acc);
                }
            }
        }
    }
}

/// (defines, refs, writes) of a function: parameters bind, the body
/// scans, global/nonlocal names lose their defines, reads that are
/// also defines are dropped (the finalize step shared with TS).
fn fn_body_info(
    src: &[u8],
    params: Option<Node>,
    body: Option<Node>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut acc = Acc::default();
    if let Some(p) = params {
        for i in 0..p.named_child_count() {
            if let Some(c) = p.named_child(i) {
                collect_param(c, src, &mut acc);
            }
        }
    }
    if let Some(b) = body {
        scan_w(b, src, &mut acc);
    }
    finalize(acc)
}

/// Refs read in an initializer or class body, with locally bound
/// names filtered out (the TS `body_refs`).
fn body_refs(node: Node, src: &[u8]) -> Vec<String> {
    let mut acc = Acc::default();
    scan_w(node, src, &mut acc);
    let (_, refs, _) = finalize(acc);
    refs
}

fn finalize(mut acc: Acc) -> (Vec<String>, Vec<String>, Vec<String>) {
    for name in &acc.excluded {
        acc.defines.remove(name);
    }
    let mut defines: Vec<String> = acc.defines.into_iter().collect();
    defines.sort();
    let define_set: HashSet<&String> = defines.iter().collect();
    let mut refs: Vec<String> = acc
        .refs
        .into_iter()
        .filter(|r| !define_set.contains(r))
        .collect();
    refs.sort();
    let mut writes: Vec<String> = acc.writes.into_iter().collect();
    writes.sort();
    (defines, refs, writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn run(rel: &str, src: &str) -> FileSymbols {
        extract("engine", Path::new(rel), src).expect("extract")
    }

    fn by_id(fs: &FileSymbols) -> BTreeMap<&str, &RawSymbol> {
        fs.symbols.iter().map(|s| (s.id.as_str(), s)).collect()
    }

    fn sv(names: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn py_module_id_rules() {
        assert_eq!(module_id_for("engine", Path::new("engine.py")), "engine::engine");
        assert_eq!(module_id_for("engine", Path::new("__init__.py")), "engine");
        assert_eq!(module_id_for("engine", Path::new("core/__init__.py")), "engine::core");
        assert_eq!(
            module_id_for("engine", Path::new("core/graph.py")),
            "engine::core::graph"
        );
        assert_eq!(module_id_for("engine", Path::new("main.py")), "engine::main");
        assert_eq!(
            module_id_for("engine", Path::new("core/graph.pyi")),
            "engine::core::graph"
        );
        assert_eq!(
            module_id_for("engine", Path::new("a/b/c/util.py")),
            "engine::a::b::c::util"
        );
    }

    const SYMBOLS_PY: &str = r#"
CONST = 1
LIMIT: int = 10
A = B = 2
pair_a, pair_b = (1, 2)
handler = lambda x: x

async def fetch(url, timeout=30):
    return url

def outer(a, b: int, *args, key=None, **kw):
    def inner(c):
        return c + a
    return inner

class Engine:
    kind = "v8"
    fallback = build_default()

    def start(self):
        return 1

    @property
    def level(self):
        return 2

    @staticmethod
    def make():
        return Engine()

    class Inner:
        def deep(self):
            return 3

type Alias = int
"#;

    #[test]
    fn py_symbols_and_kinds() {
        let fs = run("core/engine.py", SYMBOLS_PY);
        assert_eq!(fs.module_id, "engine::core::engine");
        let m = by_id(&fs);
        let want: BTreeMap<&str, NodeKind> = [
            ("engine::core::engine::CONST", NodeKind::Variable),
            ("engine::core::engine::LIMIT", NodeKind::Variable),
            ("engine::core::engine::A", NodeKind::Variable),
            ("engine::core::engine::B", NodeKind::Variable),
            ("engine::core::engine::handler", NodeKind::Variable),
            ("engine::core::engine::fetch", NodeKind::Function),
            ("engine::core::engine::outer", NodeKind::Function),
            ("engine::core::engine::outer::inner", NodeKind::Function),
            ("engine::core::engine::Engine", NodeKind::Class),
            ("engine::core::engine::Engine::start", NodeKind::Method),
            ("engine::core::engine::Engine::level", NodeKind::Method),
            ("engine::core::engine::Engine::make", NodeKind::Method),
            ("engine::core::engine::Engine::Inner", NodeKind::Class),
            ("engine::core::engine::Engine::Inner::deep", NodeKind::Method),
            ("engine::core::engine::Alias", NodeKind::Class),
        ]
        .into_iter()
        .collect();
        let got: BTreeMap<&str, NodeKind> = m.iter().map(|(k, v)| (*k, v.kind)).collect();
        assert_eq!(got, want);
        // Class-level assignments and tuple targets are NOT symbols,
        // and the lambda itself is not a function symbol.
        assert!(!m.contains_key("engine::core::engine::Engine::kind"));
        assert!(!m.contains_key("engine::core::engine::pair_a"));
        assert_eq!(m["engine::core::engine::handler"].kind, NodeKind::Variable);
    }

    #[test]
    fn py_symbol_lines_and_variable_refs() {
        let fs = run("core/engine.py", SYMBOLS_PY);
        let m = by_id(&fs);
        // CONST is on source line 2 (the fixture starts with a blank line).
        assert_eq!(m["engine::core::engine::CONST"].line, 2);
        // LIMIT's annotation name is its ref (D5); CONST has none.
        assert_eq!(m["engine::core::engine::LIMIT"].refs, vec!["int".to_string()]);
        assert!(m["engine::core::engine::CONST"].refs.is_empty());
        // Class body refs: the class-level initializer's callee, with
        // the assignment target filtered out; method bodies pruned.
        assert_eq!(
            m["engine::core::engine::Engine"].refs,
            vec!["build_default".to_string()]
        );
    }

    const SCAN_PY: &str = r#"
def compute(items, factor: int = 2) -> int:
    total = 0
    total += factor
    self_count = len(items)
    obj.attr = total
    for it in items:
        total = total + it
    with open("x") as fh:
        data = fh.read()
    squares = [n * n for n in items if n > 0]
    if (m := len(items)) > 1:
        total = m
    try:
        risky()
    except ValueError as err:
        report(err)
    global counter
    counter = 1
    return total
"#;

    #[test]
    fn py_defines_refs_writes_hand_derived() {
        let fs = run("calc.py", SCAN_PY);
        let m = by_id(&fs);
        let s = m["engine::calc::compute"];
        // defines: params + every binding target; `counter` is
        // global-declared, so its define is dropped.
        assert_eq!(
            s.defines,
            sv(&[
                "items", "factor", "total", "self_count", "it", "fh", "data", "squares", "n", "m",
                "err"
            ])
        );
        // writes: plain assignment targets (define+write), the
        // attribute name of `obj.attr = …`, and the global's write.
        // NOT: for/with/comprehension/walrus targets (defines only),
        // NOT the augmented-assignment target (recorded as a ref).
        assert_eq!(
            s.writes,
            sv(&["total", "self_count", "attr", "data", "squares", "counter"])
        );
        // refs: call roots, the except type, annotation names; every
        // read of a defined name is filtered out; `obj` (the object
        // of a write target) is not a ref, per the TS precedent.
        assert_eq!(s.refs, sv(&["len", "open", "risky", "ValueError", "report", "int"]));
    }

    const EDGES_PY: &str = r#"
import os
import engine.core.graph as graph
import requests.sessions
from engine.core import graph as g, util
from . import sibling
from .local import thing
from ..other import deep
from ...beyond import nope
from collections import *
from __future__ import annotations

def run():
    graph.draw()
    helper()
    return Engine()

class Engine(Base, mixins.Mixin, Generic[T], metaclass=ABCMeta):
    pass

class Shape(Protocol):
    pass

class TypedShape(typing.Protocol):
    pass

@app.get("/x")
def handler():
    pass

@cache
def cached():
    pass

@decorator
class Wrapped(Base):
    pass
"#;

    #[test]
    fn py_imports_all_rows() {
        let fs = run("core/engine.py", EDGES_PY);
        let got: Vec<(&str, Vec<String>)> = fs
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        let want: Vec<(&str, Vec<String>)> = vec![
            ("external::os", sv(&["os"])),
            ("engine::core::graph", sv(&["graph"])),
            ("external::requests", sv(&["requests"])),
            ("engine::core", sv(&["g", "util"])),
            ("engine::core", sv(&["sibling"])),
            ("engine::core::local", sv(&["thing"])),
            ("engine::other", sv(&["deep"])),
            ("external::unresolved", sv(&["nope"])),
            ("external::collections", sv(&[])),
            ("external::__future__", sv(&["annotations"])),
        ];
        assert_eq!(got, want);
        assert!(fs.imports.iter().all(|i| i.from_module_id == "engine::core::engine"));
    }

    #[test]
    fn py_relative_imports_from_init() {
        // In `core/__init__.py` the module id IS the package, so one
        // dot stays and two dots reach the root package.
        let src = "from . import sib\nfrom .mod import x\nfrom ..up import y\n";
        let fs = run("core/__init__.py", src);
        assert_eq!(fs.module_id, "engine::core");
        let got: Vec<&str> = fs.imports.iter().map(|i| i.to_module_id.as_str()).collect();
        assert_eq!(got, vec!["engine::core", "engine::core::mod", "engine::up"]);
    }

    #[test]
    fn py_calls_and_decorator_edges() {
        let fs = run("core/engine.py", EDGES_PY);
        let got: Vec<(&str, &str)> = fs
            .calls
            .iter()
            .map(|c| (c.from_id.as_str(), c.to_name.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("engine::core::engine::run", "graph.draw"),
                ("engine::core::engine::run", "helper"),
                ("engine::core::engine::run", "Engine"),
                // The decorator call belongs to the enclosing (module)
                // scope, per D3 — not to `handler`.
                ("engine::core::engine", "app.get"),
            ]
        );
        let m = by_id(&fs);
        // Bare decorators land as refs on the decorated symbol (D3).
        assert_eq!(m["engine::core::engine::cached"].refs, sv(&["cache"]));
        assert_eq!(m["engine::core::engine::Wrapped"].refs, sv(&["decorator"]));
        assert!(m["engine::core::engine::handler"].refs.is_empty());
    }

    #[test]
    fn py_heritage_and_protocol_kind() {
        let fs = run("core/engine.py", EDGES_PY);
        let got: Vec<(&str, &str)> = fs
            .heritage
            .iter()
            .map(|h| (h.type_name.as_str(), h.target_name.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Engine", "Base"),
                ("Engine", "Mixin"),
                ("Engine", "Generic"),
                ("Shape", "Protocol"),
                ("TypedShape", "Protocol"),
                ("Wrapped", "Base"),
            ]
        );
        assert!(fs.heritage.iter().all(|h| h.kind == EdgeKind::Inherits));
        let m = by_id(&fs);
        assert_eq!(m["engine::core::engine::Engine"].kind, NodeKind::Class);
        // Protocol classes — plain, dotted, in either spelling — are
        // interfaces (D2); the detection is name-based.
        assert_eq!(m["engine::core::engine::Shape"].kind, NodeKind::Interface);
        assert_eq!(m["engine::core::engine::TypedShape"].kind, NodeKind::Interface);
    }

    // MEASURED_PY — the hand-counted measured-fields fixture (spec
    // §3.7, the MEASURED_TS pattern). Line numbers count the fixture
    // as written: line 1 is blank, `LIMIT` is line 2, `PLAIN` line 3,
    // `class C` line 5, `def classify` line 8 with its body ending at
    // line 14, `async def fetch` line 16, `def tally` line 19,
    // `def pick` line 36, `def stub` line 45.
    const MEASURED_PY: &str = r#"
LIMIT: int = 10
PLAIN = 5

class C(Base, metaclass=M):
    pass

def classify(n, flag) -> str:
    if n < 0:
        return "neg"
    elif n == 0:
        return "zero"
    else:
        return "pos"

async def fetch(url):
    return url

def tally(items):
    s = 0
    for it in items:
        s += it
    while s > 100:
        s -= 10
    try:
        risky(s)
    except ValueError:
        s = -1
    except TypeError:
        s = -2
    label = "big" if s > 50 else "small"
    both = check(s) and other(s) or third(s)
    squares = [x * x for x in items if x > 0]
    return label

def pick(kind):
    match kind:
        case 1:
            return "one"
        case 2 if kind > 0:
            return "two"
        case _:
            return "other"

def stub(x) -> int: ...
"#;

    #[test]
    fn py_measured_fields_hand_counted() {
        let fs = run("measured.py", MEASURED_PY);
        let m = by_id(&fs);
        // Variables: the initializer never appears in the signature.
        let limit = m["engine::measured::LIMIT"];
        assert_eq!(limit.signature.as_deref(), Some("LIMIT: int"));
        assert_eq!(limit.end_line, Some(2));
        assert_eq!(limit.decisions, None);
        assert_eq!(m["engine::measured::PLAIN"].signature.as_deref(), Some("PLAIN"));
        // Class header, bases and keyword bases included, colon cut.
        let c = m["engine::measured::C"];
        assert_eq!(c.signature.as_deref(), Some("class C(Base, metaclass=M)"));
        assert_eq!(c.end_line, Some(6));
        // classify: if +1, elif +1 → 2 decisions; span lines 8–14.
        let classify = m["engine::measured::classify"];
        assert_eq!(
            classify.signature.as_deref(),
            Some("def classify(n, flag) -> str")
        );
        assert_eq!(classify.line, 8);
        assert_eq!(classify.end_line, Some(14));
        assert_eq!(classify.decisions, Some(2));
        // async keeps its prefix verbatim (D4); branch-free → 0.
        let fetch = m["engine::measured::fetch"];
        assert_eq!(fetch.signature.as_deref(), Some("async def fetch(url)"));
        assert_eq!(fetch.decisions, Some(0));
        // tally: for 1 + while 1 + except 2 + conditional 1
        //        + boolean operators 2 (and, or)
        //        + comprehension for 1 + comprehension if 1 = 9.
        assert_eq!(m["engine::measured::tally"].decisions, Some(9));
        // pick: match with 3 cases → 3 − 1 = 2; the case guard
        // (`if kind > 0`, an if_clause node) is NOT counted.
        assert_eq!(m["engine::measured::pick"].decisions, Some(2));
        // A stub body counts 0 — measured, not absent.
        let stub = m["engine::measured::stub"];
        assert_eq!(stub.signature.as_deref(), Some("def stub(x) -> int"));
        assert_eq!(stub.decisions, Some(0));
    }
}

