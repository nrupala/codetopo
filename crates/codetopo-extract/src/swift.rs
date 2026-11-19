// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Swift extractor (`.swift`) — beside Rust,
//! TypeScript/JavaScript, Python, Kotlin, C, Java, Go, Shell,
//! and C++.
//!
//! Node kinds below were verified against
//! `tree-sitter-swift-0.7.4/src/node-types.json` in the
//! crates.io registry cache and against parse trees of the
//! fixture (the pinned crate, never guessed): `source_file`,
//! `import_declaration`, `class_declaration` (fields
//! `declaration_kind` — class/struct/enum/actor/extension —
//! `name`, `body`; `inheritance_specifier` children),
//! `protocol_declaration` (fields `name`, `body`),
//! `protocol_function_declaration`, `function_declaration`
//! (fields `name`, `body`; `parameter` children; the return
//! type rides a second `name` field in this grammar version —
//! the first `name` field that is a simple identifier is the
//! function's name), `init_declaration`, `deinit_declaration`,
//! `property_declaration` (fields `name` — a `pattern` — and
//! `value`), `enum_entry` (field `name`),
//! `typealias_declaration` (fields `name`, `value`),
//! `call_expression` (callee and `call_suffix` children),
//! `navigation_expression` (fields `target`, `suffix`),
//! `assignment` (fields `target`, `result`; the operator is an
//! unnamed token between them), `value_binding_pattern`.
//!
//! The mapping follows the Python extractor's spec shape:
//! module ids are path-derived (the shared rule). Symbols
//! (§3.2 — classes, structs, actors, and enums are classes;
//! protocols are interfaces; functions outside a type are
//! functions and inside are methods, as are initializers;
//! protocol requirements are methods; enum entries are
//! variables scoped under their enum; top-level `let`/`var`
//! are variables; a typealias is a class. Properties inside a
//! type are fields, not symbols. An `extension` emits no
//! symbol of its own: it opens a scope under the extended
//! type's name so its methods nest there, and its conformances
//! are heritage rows for that type). Imports (Swift imports
//! name external modules — the language has no in-package
//! import syntax — so every import is `external::{first
//! segment}` with the last segment as binding). Calls (§3.4 —
//! the callee text as written). Heritage (inheritance
//! specifiers are Inherits, for conformances and superclass
//! alike — Swift has one inheritance list). The
//! defines/refs/writes scan (§3.6's marimo discipline: nested
//! declarations pruned, reads that are also defines dropped,
//! augmented-assignment targets read not written, only the
//! target of a navigation is a read, type names are refs);
//! decisions (§3.2's enumeration, Swift form: `if` +1, `guard`
//! +1, `for`/`while`/`repeat-while` +1, `switch` contributes
//! entries − 1, ternary +1, each conjunction/disjunction
//! (`&&`/`||`) +1). The walk runs on an explicit heap stack
//! (the E-1 lesson).

use std::collections::HashSet;
use std::path::Path;

use codetopo_core::emit::{FileSymbols, RawCall, RawHeritage, RawImport, RawSymbol};
use codetopo_core::schema::{EdgeKind, NodeKind};

use crate::{
    children, display_path, end_line_of, line_of, signature_of, text_of, ExtractError,
};

type Node<'a> = tree_sitter::Node<'a>;

fn txt(node: Node, src: &[u8]) -> String {
    text_of(node, src).unwrap_or("").to_string()
}

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_swift::LANGUAGE.into())
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
        module_id: module_id.clone(),
        path: Vec::new(),
        enclosing: Vec::new(),
        in_class: false,
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

/// Path-derived module id — the shared rule (Python §3.1).
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
    if !stem.is_empty() {
        parts.push(stem);
    }
    parts.join("::")
}

struct Walker<'a> {
    src: &'a [u8],
    module_id: String,
    path: Vec<String>,
    enclosing: Vec<String>,
    in_class: bool,
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
    calls: Vec<RawCall>,
    heritage: Vec<RawHeritage>,
}

/// One step of the walker's explicit work stack (the E-1 lesson).
enum Step<'t> {
    Enter(Node<'t>),
    OpenSymbol {
        kind: NodeKind,
        name: String,
        node: Node<'t>,
        defines: Vec<String>,
        refs: Vec<String>,
        writes: Vec<String>,
        end_line: Option<u32>,
        signature: Option<String>,
        decisions: Option<u32>,
        in_class: bool,
    },
    LeaveSymbol {
        in_class: bool,
    },
    /// Open a scope that is not a symbol (an extension body):
    /// pushes one path segment and sets the class-scope flag.
    OpenScope {
        name: String,
    },
    LeaveScope {
        in_class: bool,
    },
}

impl<'a> Walker<'a> {
    fn text(&self, n: Node) -> String {
        txt(n, self.src)
    }

    fn walk<'t>(&mut self, root: Node<'t>) {
        let mut stack = vec![Step::Enter(root)];
        while let Some(step) = stack.pop() {
            match step {
                Step::Enter(node) => self.visit(node, &mut stack),
                Step::OpenSymbol {
                    kind,
                    name,
                    node,
                    defines,
                    refs,
                    writes,
                    end_line,
                    signature,
                    decisions,
                    in_class,
                } => {
                    self.push_symbol(
                        kind, &name, node, defines, refs, writes, end_line, signature, decisions,
                    );
                    self.in_class = in_class;
                }
                Step::LeaveSymbol { in_class } => {
                    self.in_class = in_class;
                    self.pop_symbol();
                }
                Step::OpenScope { name } => {
                    self.path.push(name);
                    self.in_class = true;
                }
                Step::LeaveScope { in_class } => {
                    self.in_class = in_class;
                    self.path.pop();
                }
            }
        }
    }

    fn push_children<'t>(&self, stack: &mut Vec<Step<'t>>, node: Node<'t>) {
        for c in children(node).into_iter().rev() {
            stack.push(Step::Enter(c));
        }
    }

    fn visit<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        match node.kind() {
            "class_declaration" => self.schedule_class(node, stack),
            "protocol_declaration" => {
                self.schedule_type(node, NodeKind::Interface, stack)
            }
            "function_declaration" => self.schedule_function(node, stack),
            "protocol_function_declaration" => {
                self.schedule_function(node, stack)
            }
            "init_declaration" => self.schedule_init(node, "init", stack),
            "deinit_declaration" => self.schedule_init(node, "deinit", stack),
            "subscript_declaration" => {
                self.schedule_init(node, "subscript", stack)
            }
            "typealias_declaration" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = type_name_text(n, self.src);
                    self.push_symbol(
                        NodeKind::Class,
                        &name,
                        node,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Some(end_line_of(node)),
                        signature_of(node, node.child_by_field_name("value"), self.src),
                        None,
                    );
                    self.pop_symbol();
                }
            }
            "property_declaration"
                if self.enclosing.is_empty() && self.path.is_empty() =>
            {
                // A top-level let/var is a variable.
                for name in pattern_names(node, self.src) {
                    let value = node.child_by_field_name("value");
                    let mut refs: Vec<String> =
                        value.map(|v| body_refs(v, self.src)).unwrap_or_default();
                    refs.sort();
                    refs.dedup();
                    self.push_symbol(
                        NodeKind::Variable,
                        &name,
                        node,
                        Vec::new(),
                        refs,
                        Vec::new(),
                        Some(end_line_of(node)),
                        signature_of(node, value, self.src),
                        None,
                    );
                    self.pop_symbol();
                    if let Some(v) = value {
                        stack.push(Step::Enter(v));
                    }
                }
            }
            "enum_entry" if self.in_class => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    self.push_symbol(
                        NodeKind::Variable,
                        &name,
                        node,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Some(end_line_of(node)),
                        None,
                        None,
                    );
                    self.pop_symbol();
                }
                self.push_children(stack, node);
            }
            "import_declaration" => self.handle_import(node),
            "call_expression" => {
                self.record_call(node);
                self.push_children(stack, node);
            }
            _ => self.push_children(stack, node),
        }
    }

    fn current_scope(&self) -> String {
        self.enclosing
            .last()
            .cloned()
            .unwrap_or_else(|| self.module_id.clone())
    }

    /// One `call_expression` → one RawCall; `to_name` is the
    /// callee's text as written (§3.4).
    fn record_call(&mut self, node: Node) {
        if let Some(callee) = children(node).into_iter().next() {
            let t = self.text(callee).trim().to_string();
            if !t.is_empty() {
                self.calls.push(RawCall {
                    from_id: self.current_scope(),
                    to_name: t,
                    line: line_of(node),
                });
            }
        }
    }

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
            measures: Default::default(),
            loc_file: String::new(),
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

    /// Schedule a class_declaration: classes, structs, actors,
    /// and enums are Class symbols; an extension opens a scope
    /// under the extended type's name without emitting a symbol.
    fn schedule_class<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = type_name_text(name_node, self.src);
        let decl_kind = node
            .child_by_field_name("declaration_kind")
            .map(|k| self.text(k))
            .unwrap_or_default();
        self.collect_heritage(node, &name);
        let body = node.child_by_field_name("body");
        if decl_kind == "extension" {
            stack.push(Step::LeaveScope {
                in_class: self.in_class,
            });
            if let Some(b) = body {
                self.push_children(stack, b);
            }
            stack.push(Step::OpenScope { name });
            return;
        }
        let signature = signature_of(node, body, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Class,
            name,
            node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature,
            decisions: None,
            in_class: true,
        });
    }

    /// Schedule a protocol (an Interface symbol).
    fn schedule_type<'t>(&mut self, node: Node<'t>, kind: NodeKind, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = type_name_text(name_node, self.src);
        self.collect_heritage(node, &name);
        let body = node.child_by_field_name("body");
        let signature = signature_of(node, body, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind,
            name,
            node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature,
            decisions: None,
            in_class: true,
        });
    }

    /// Inheritance specifiers → Inherits rows (§3.3).
    fn collect_heritage(&mut self, node: Node, type_name: &str) {
        for c in children(node) {
            if c.kind() != "inheritance_specifier" {
                continue;
            }
            if let Some(from) = c.child_by_field_name("inherits_from") {
                let target = type_name_text(from, self.src);
                if !target.is_empty() {
                    self.heritage.push(RawHeritage {
                        type_name: type_name.to_string(),
                        target_name: target,
                        kind: EdgeKind::Inherits,
                        line: line_of(c),
                    });
                }
            }
        }
    }

    /// Schedule a function or protocol function declaration.
    fn schedule_function<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name) = function_name(node, self.src) else {
            self.push_children(stack, node);
            return;
        };
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(
            self.src,
            node,
            return_type_node(node),
            body,
        );
        let kind = if self.in_class {
            NodeKind::Method
        } else {
            NodeKind::Function
        };
        let signature = signature_of(node, body, self.src);
        let decisions = body.map(|b| count_decisions(b, self.src));
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind,
            name,
            node,
            defines,
            refs,
            writes,
            end_line: Some(end_line_of(node)),
            signature,
            decisions,
            in_class: false,
        });
    }

    /// Schedule an initializer / deinitializer / subscript —
    /// methods with grammar-given names.
    fn schedule_init<'t>(&mut self, node: Node<'t>, name: &str, stack: &mut Vec<Step<'t>>) {
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(
            self.src,
            node,
            return_type_node(node),
            body,
        );
        let signature = signature_of(node, body, self.src);
        let decisions = body.map(|b| count_decisions(b, self.src));
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Method,
            name: name.to_string(),
            node,
            defines,
            refs,
            writes,
            end_line: Some(end_line_of(node)),
            signature,
            decisions,
            in_class: false,
        });
    }

    /// `import Foundation` → external::Foundation, binding
    /// [Foundation]. Swift imports name external modules.
    fn handle_import(&mut self, node: Node) {
        let mut dotted: Option<String> = None;
        for c in children(node) {
            if c.kind() == "identifier" {
                dotted = Some(self.text(c));
            }
        }
        let Some(dotted) = dotted else { return };
        let segs: Vec<&str> = dotted.split('.').collect();
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: format!("external::{}", segs[0]),
            bindings: vec![segs[segs.len() - 1].to_string()],
            line: line_of(node),
        });
    }
}

/// A function's name: the first `name` field that is a simple
/// identifier (this grammar also parks the return type under a
/// `name` field — the type nodes are never simple identifiers).
fn function_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for c in node.children_by_field_name("name", &mut cursor) {
        if c.kind() == "simple_identifier" {
            return Some(txt(c, src));
        }
    }
    None
}

/// A declaration's return type node: the `return_type` field,
/// or the `name`-field child that is a type (see function_name).
fn return_type_node(node: Node) -> Option<Node> {
    if let Some(t) = node.child_by_field_name("return_type") {
        return Some(t);
    }
    let mut cursor = node.walk();
    for c in node.children_by_field_name("name", &mut cursor) {
        if matches!(
            c.kind(),
            "user_type" | "array_type" | "dictionary_type" | "optional_type" | "tuple_type"
                | "function_type" | "metatype" | "opaque_type" | "existential_type"
        ) {
            return Some(c);
        }
    }
    None
}

/// The name a type node denotes: the last type identifier in
/// its subtree.
fn type_name_text(node: Node, src: &[u8]) -> String {
    if matches!(node.kind(), "type_identifier" | "simple_identifier") {
        return txt(node, src);
    }
    let mut last = String::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "type_identifier" {
            last = txt(n, src);
        }
        for c in children(n) {
            stack.push(c);
        }
    }
    if last.is_empty() {
        txt(node, src).trim().to_string()
    } else {
        last
    }
}

/// The names a property declaration's pattern binds. (In this
/// grammar `bound_identifier` is a FIELD name on the pattern's
/// simple_identifier children, not a node kind — the bound
/// names are the pattern's simple identifiers.)
fn pattern_names(decl: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(pat) = decl.child_by_field_name("name") else {
        return out;
    };
    let mut stack = vec![pat];
    while let Some(n) = stack.pop() {
        if n.kind() == "simple_identifier" {
            out.push(txt(n, src));
            continue;
        }
        for c in children(n) {
            stack.push(c);
        }
    }
    out
}

/// Decision points in a body (the Swift form of §3.2's
/// enumeration). Iterative (E-1); nested declarations pruned.
fn count_decisions(body: Node, _src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "function_declaration"
                | "init_declaration"
                | "deinit_declaration"
                | "class_declaration"
                | "protocol_declaration"
        ) {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "guard_statement" | "for_statement" | "while_statement"
            | "repeat_while_statement" | "ternary_expression" | "conjunction_expression"
            | "disjunction_expression" => 1,
            "switch_statement" => {
                let entries = count_kind_in(node, "switch_entry");
                entries.saturating_sub(1)
            }
            _ => 0,
        };
        for c in children(node) {
            stack.push(c);
        }
    }
    n
}

/// Count descendants (inclusive) of `node` with the given kind.
fn count_kind_in(root: Node, kind: &str) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == kind {
            n += 1;
        }
        for c in children(node) {
            stack.push(c);
        }
    }
    n
}

// ---------------------------------------------------------------------------
// The defines/refs/writes scan (spec §3.6), Swift form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): the parameter
/// names (the declaration's `parameter` children), the body's
/// own scan, the return type's names, minus the defines from
/// the refs.
fn fn_body_info(
    src: &[u8],
    decl: Node,
    return_type: Option<Node>,
    body: Option<Node>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
    for p in children(decl) {
        if p.kind() != "parameter" {
            continue;
        }
        if let Some(n) = p.child_by_field_name("name") {
            if n.kind() == "simple_identifier" {
                let name = txt(n, src);
                if name != "_" {
                    defines.insert(name);
                }
            }
        }
        // The parameter's type rides its second `name` field in
        // this grammar, or a plain `type` field.
        if let Some(t) = p.child_by_field_name("type") {
            scan_type_names(t, src, &mut refs);
        } else if let Some(t) = return_type_node(p) {
            scan_type_names(t, src, &mut refs);
        }
    }
    if let Some(t) = return_type {
        scan_type_names(t, src, &mut refs);
    }
    if let Some(b) = body {
        scan_w(b, src, &mut defines, &mut refs, &mut writes);
    }
    refs = refs.difference(&defines).cloned().collect();
    let mut d: Vec<String> = defines.into_iter().collect();
    d.sort();
    let mut r: Vec<String> = refs.into_iter().collect();
    r.sort();
    let mut w: Vec<String> = writes.into_iter().collect();
    w.sort();
    (d, r, w)
}

/// The reads of one expression (a top-level variable's
/// initializer refs).
fn body_refs(root: Node, src: &[u8]) -> Vec<String> {
    let mut defines = HashSet::new();
    let mut refs = HashSet::new();
    let mut writes = HashSet::new();
    scan_w(root, src, &mut defines, &mut refs, &mut writes);
    refs.into_iter().collect()
}

/// Collect the names a type node mentions as refs: a user type
/// contributes its last type identifier; other type forms
/// contribute their type identifiers.
fn scan_type_names(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    if node.kind() == "user_type" {
        let name = type_name_text(node, src);
        if !name.is_empty() {
            refs.insert(name);
        }
        return;
    }
    if node.kind() == "type_identifier" {
        refs.insert(txt(node, src));
        return;
    }
    for c in children(node) {
        scan_type_names(c, src, refs);
    }
}

/// The operator of an assignment node, read from the source
/// between its target and its result (the operator is an
/// unnamed token in this grammar).
fn assignment_operator(node: Node, src: &[u8]) -> String {
    let target = node.child_by_field_name("target");
    let result = node.child_by_field_name("result");
    if let (Some(t), Some(r)) = (target, result) {
        let slice = &src[t.end_byte()..r.start_byte()];
        let text = String::from_utf8_lossy(slice);
        return text.trim().to_string();
    }
    "=".to_string()
}

/// One walk computing defines/refs/writes for a body. Iterative.
fn scan_w(
    root: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
    writes: &mut HashSet<String>,
) {
    enum S<'t> {
        N(Node<'t>),
        NavTarget(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::NavTarget(nav) => {
                // Only the target of a navigation is a read.
                if let Some(t) = nav.child_by_field_name("target") {
                    stack.push(S::N(t));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested declarations are separate symbols.
            "function_declaration" | "init_declaration" | "deinit_declaration"
            | "class_declaration" | "protocol_declaration" => {}
            "property_declaration" => {
                for name in pattern_names(node, src) {
                    defines.insert(name.clone());
                    if node.child_by_field_name("value").is_some() {
                        writes.insert(name);
                    }
                }
                for c in children(node) {
                    if c.kind() == "type_annotation" {
                        if let Some(t) = c.child_by_field_name("name") {
                            scan_type_names(t, src, refs);
                        }
                    }
                }
                if let Some(v) = node.child_by_field_name("value") {
                    stack.push(S::N(v));
                }
            }
            "assignment" => {
                let op = assignment_operator(node, src);
                if let Some(r) = node.child_by_field_name("result") {
                    stack.push(S::N(r));
                }
                if let Some(t) = node.child_by_field_name("target") {
                    // target is a directly_assignable_expression
                    // wrapping the real target.
                    let inner = (0..t.named_child_count())
                        .filter_map(|i| t.named_child(i))
                        .next()
                        .unwrap_or(t);
                    match inner.kind() {
                        "simple_identifier" => {
                            let name = txt(inner, src);
                            if op == "=" {
                                defines.insert(name.clone());
                                writes.insert(name);
                            } else {
                                refs.insert(name);
                            }
                        }
                        "navigation_expression" => {
                            if op == "=" {
                                if let Some(suffix) = inner.child_by_field_name("suffix") {
                                    writes.insert(type_name_text(suffix, src));
                                }
                            }
                            stack.push(S::NavTarget(inner));
                        }
                        _ => stack.push(S::N(inner)),
                    }
                }
            }
            "navigation_expression" => stack.push(S::NavTarget(node)),
            "call_expression" => {
                let kids = children(node);
                if let Some(callee) = kids.first() {
                    if callee.kind() == "navigation_expression" {
                        stack.push(S::NavTarget(*callee));
                    } else {
                        stack.push(S::N(*callee));
                    }
                }
                for c in kids.iter().skip(1) {
                    stack.push(S::N(*c));
                }
            }
            "for_statement" => {
                if let Some(item) = node.child_by_field_name("item") {
                    // The item is a pattern: its simple
                    // identifiers are the bound names.
                    let mut s = vec![item];
                    while let Some(n) = s.pop() {
                        if n.kind() == "simple_identifier" {
                            defines.insert(txt(n, src));
                            continue;
                        }
                        for c in children(n) {
                            s.push(c);
                        }
                    }
                }
                for c in children(node) {
                    stack.push(S::N(c));
                }
            }
            "user_type" => {
                let name = type_name_text(node, src);
                if !name.is_empty() {
                    refs.insert(name);
                }
            }
            "simple_identifier" => {
                refs.insert(txt(node, src));
            }
            _ => {
                for c in children(node) {
                    stack.push(S::N(c));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn one(rel: &str, src: &str) -> FileSymbols {
        extract("quarry", &PathBuf::from(rel), src).unwrap()
    }

    #[test]
    fn module_ids_follow_the_path_rule() {
        let f = one("Core/Engine.swift", "final class Engine {}\n");
        assert_eq!(f.module_id, "quarry::Core::Engine");
    }

    #[test]
    fn protocol_is_an_interface_and_requirements_are_methods() {
        let f = one(
            "Shape.swift",
            "protocol Shape {\n    func area() -> Double\n}\n",
        );
        let kinds: Vec<(&str, NodeKind)> =
            f.symbols.iter().map(|s| (s.label.as_str(), s.kind)).collect();
        assert_eq!(
            kinds,
            vec![("Shape", NodeKind::Interface), ("area", NodeKind::Method)]
        );
        assert_eq!(f.symbols[1].refs, vec!["Double".to_string()]);
        assert_eq!(f.symbols[1].decisions, None);
    }

    #[test]
    fn extension_opens_a_scope_without_a_symbol() {
        let f = one(
            "E.swift",
            "extension Engine {\n    func render() -> Double { 1.0 }\n}\n",
        );
        assert_eq!(f.symbols.len(), 1);
        assert_eq!(f.symbols[0].id, "quarry::E::Engine::render");
        assert_eq!(f.symbols[0].kind, NodeKind::Method);
    }

    #[test]
    fn scan_augmented_target_is_a_ref() {
        let f = one(
            "E.swift",
            "final class E {\n    var count = 0\n    func bump(by amount: Int) {\n        count += amount\n    }\n}\n",
        );
        let bump = f.symbols.iter().find(|s| s.label == "bump").unwrap();
        assert_eq!(bump.defines, vec!["amount".to_string()]);
        assert_eq!(bump.refs, vec!["Int".to_string(), "count".to_string()]);
        assert!(bump.writes.is_empty());
    }

    #[test]
    fn enum_entries_are_variables() {
        let f = one(
            "S.swift",
            "enum Suit {\n    case spades\n    case hearts\n}\n",
        );
        let names: Vec<&str> = f.symbols.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(names, vec!["Suit", "spades", "hearts"]);
        assert_eq!(f.symbols[1].id, "quarry::S::Suit::spades");
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("func f(_ x: Int) -> Int {\n    ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1\n}\n");
        let f = one("deep.swift", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
