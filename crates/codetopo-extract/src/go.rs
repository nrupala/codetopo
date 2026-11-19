// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Go extractor (`.go`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, and Java.
//!
//! Node kinds below were verified against
//! `tree-sitter-go-0.25.0/src/node-types.json` in the crates.io
//! registry cache and against parse trees of the fixture (the
//! pinned crate, never guessed): `source_file`, `package_clause`,
//! `import_declaration` (`import_spec`: fields `name`, `path`;
//! grouped under `import_spec_list`), `function_declaration`
//! (fields `name`, `parameters`, `result`, `body`),
//! `method_declaration` (fields `receiver`, `name`,
//! `parameters`, `result`, `body`), `type_declaration`
//! (`type_spec`: fields `name`, `type`; `type_alias`),
//! `struct_type`, `interface_type` (`method_elem`, `type_elem`),
//! `field_declaration` (fields `name`, `type`), `var_declaration`
//! / `const_declaration` (`var_spec` / `const_spec`: fields
//! `name`, `type`, `value`), `call_expression` (field
//! `function`), `selector_expression` (fields `operand`,
//! `field`), `assignment_statement` (fields `left`, `right`),
//! `short_var_declaration` (fields `left`, `right`),
//! `inc_statement`, `composite_literal` (fields `type`, `body`),
//! `qualified_type` (fields `package`, `name`).
//!
//! The mapping follows the Python extractor's spec shape, with
//! Go's own module rule: **all files in a directory form one
//! package, so the module id is the package name plus the
//! directory segments, with no file stem** — `shapes/*.go` is one
//! module, and an import path (which IS a module path) resolves
//! to exactly that id. Symbols (§3.2 — functions are functions,
//! methods nest under their receiver type's name, structs are
//! classes, interfaces are interfaces, other named types and
//! aliases are classes, interface method elements are methods,
//! package-level vars and consts are variables; struct fields
//! are not symbols); imports (the path with `/` → `::`, internal
//! when its first segment is the package name, else
//! `external::{first segment}`; bindings are the package name,
//! or the alias when written, or none for a dot import); calls
//! (§3.4 — for a selector call, the field name, the Python
//! attribute rule); heritage (an interface embedding another
//! interface is Inherits; struct embedding is NOT heritage — it
//! is composition, and method promotion is a call-resolution
//! concern, not a type-hierarchy edge); the defines/refs/writes
//! scan (§3.6's marimo discipline: nested declarations pruned,
//! reads that are also defines dropped, augmented-assignment
//! targets and `++`/`--` targets read not written, only the
//! operand of a selector is a read, composite-literal keys are
//! field names, not reads); decisions (§3.2's enumeration, Go
//! form: `if` +1, `for` +1, each switch/select contributes its
//! cases − 1, each `&&`/`||` +1). The walk runs on an explicit
//! heap stack (the E-1 lesson).

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
        .set_language(&tree_sitter_go::LANGUAGE.into())
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
        path: Vec::new(),
        path_counts: Vec::new(),
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

/// Module id: the package name plus the DIRECTORY segments — all
/// files in a directory form one Go package (no file stem).
fn module_id_for(package: &str, rel_path: &Path) -> String {
    let mut parts: Vec<String> = vec![package.to_string()];
    if let Some(parent) = rel_path.parent() {
        for comp in parent.components() {
            if let std::path::Component::Normal(s) = comp {
                parts.push(s.to_string_lossy().into_owned());
            }
        }
    }
    parts.join("::")
}

struct Walker<'a> {
    src: &'a [u8],
    pkg: String,
    module_id: String,
    path: Vec<String>,
    /// How many path segments each open symbol owns (a method
    /// owns two: its receiver type's name and its own).
    path_counts: Vec<usize>,
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
        /// Path segments for this symbol: one for most symbols,
        /// [receiver type, method name] for a method.
        names: Vec<String>,
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
                    names,
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
                        kind, &names, node, defines, refs, writes, end_line, signature, decisions,
                    );
                    self.in_class = in_class;
                }
                Step::LeaveSymbol { in_class } => {
                    self.in_class = in_class;
                    self.pop_symbol();
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
            "function_declaration" => self.schedule_function(node, stack),
            "method_declaration" => self.schedule_method(node, stack),
            "type_declaration" => {
                for c in children(node) {
                    if matches!(c.kind(), "type_spec" | "type_alias") {
                        self.schedule_type_spec(c, stack);
                    }
                }
            }
            "import_declaration" => {
                for c in children(node) {
                    match c.kind() {
                        "import_spec" => self.handle_import_spec(c),
                        "import_spec_list" => {
                            for s in children(c) {
                                if s.kind() == "import_spec" {
                                    self.handle_import_spec(s);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            "var_declaration" | "const_declaration" if self.enclosing.is_empty() => {
                for spec in spec_children(node) {
                    for name_node in name_children(spec) {
                        let name = self.text(name_node);
                        self.push_symbol(
                            NodeKind::Variable,
                            &[name],
                            spec,
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            Some(end_line_of(spec)),
                            signature_of(spec, spec.child_by_field_name("value"), self.src),
                            None,
                        );
                        self.pop_symbol();
                    }
                    // Initializer calls belong to the module scope.
                    if let Some(v) = spec.child_by_field_name("value") {
                        stack.push(Step::Enter(v));
                    }
                }
            }
            "method_elem" if self.in_class => {
                // An interface's method element: a method with a
                // signature and no body.
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    let (defines, refs, writes) = fn_body_info(
                        self.src,
                        node.child_by_field_name("parameters"),
                        node.child_by_field_name("result"),
                        None,
                        &[],
                        &[],
                    );
                    self.push_symbol(
                        NodeKind::Method,
                        &[name],
                        node,
                        defines,
                        refs,
                        writes,
                        Some(end_line_of(node)),
                        signature_of(node, None, self.src),
                        None,
                    );
                    self.pop_symbol();
                }
            }
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
    /// callee's name — for a selector, its field (§3.4).
    fn record_call(&mut self, node: Node) {
        let Some(f) = node.child_by_field_name("function") else {
            return;
        };
        let t = match f.kind() {
            "selector_expression" => f
                .child_by_field_name("field")
                .map(|n| self.text(n))
                .unwrap_or_default(),
            _ => self.text(f).trim().to_string(),
        };
        if !t.is_empty() {
            self.calls.push(RawCall {
                from_id: self.current_scope(),
                to_name: t,
                line: line_of(node),
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_symbol(
        &mut self,
        kind: NodeKind,
        names: &[String],
        node: Node,
        defines: Vec<String>,
        refs: Vec<String>,
        writes: Vec<String>,
        end_line: Option<u32>,
        signature: Option<String>,
        decisions: Option<u32>,
    ) {
        for n in names {
            self.path.push(n.clone());
        }
        self.path_counts.push(names.len());
        let id = format!("{}::{}", self.module_id, self.path.join("::"));
        self.symbols.push(RawSymbol {
            id: id.clone(),
            kind,
            label: names.last().cloned().unwrap_or_default(),
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
        let n = self.path_counts.pop().unwrap_or(1);
        for _ in 0..n {
            self.path.pop();
        }
    }

    /// Schedule a function declaration.
    fn schedule_function<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(
            self.src,
            node.child_by_field_name("parameters"),
            node.child_by_field_name("result"),
            body,
            &[],
            &[],
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
            kind: NodeKind::Function,
            names: vec![name],
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

    /// Schedule a method declaration: the symbol nests under its
    /// receiver type's name (two path segments), wherever in the
    /// package that type is declared.
    fn schedule_method<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let mut extra_defines: Vec<String> = Vec::new();
        let mut extra_refs: Vec<String> = Vec::new();
        let mut receiver_type: Option<String> = None;
        if let Some(recv) = node.child_by_field_name("receiver") {
            for p in children(recv) {
                if let Some(n) = p.child_by_field_name("name") {
                    extra_defines.push(self.text(n));
                }
                if let Some(t) = p.child_by_field_name("type") {
                    receiver_type = base_type_name(t, self.src);
                    let mut set = HashSet::new();
                    scan_type_names(t, self.src, &mut set);
                    extra_refs.extend(set);
                }
            }
        }
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(
            self.src,
            node.child_by_field_name("parameters"),
            node.child_by_field_name("result"),
            body,
            &extra_defines,
            &extra_refs,
        );
        let signature = signature_of(node, body, self.src);
        let decisions = body.map(|b| count_decisions(b, self.src));
        let names = match receiver_type {
            Some(t) => vec![t, name],
            None => vec![name],
        };
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Method,
            names,
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

    /// Schedule one type_spec / type_alias: structs are classes,
    /// interfaces are interfaces (their method elements are
    /// methods and their embedded types are Inherits heritage),
    /// every other named type is a class.
    fn schedule_type_spec<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let ty = node.child_by_field_name("type");
        let kind = match ty.map(|t| t.kind()) {
            Some("interface_type") => NodeKind::Interface,
            _ => NodeKind::Class,
        };
        // Interface embedding is heritage (Inherits).
        if let Some(t) = ty {
            if t.kind() == "interface_type" {
                for elem in children(t) {
                    if elem.kind() == "type_elem" {
                        for part in children(elem) {
                            if let Some(target) = base_type_name(part, self.src) {
                                self.heritage.push(RawHeritage {
                                    type_name: name.clone(),
                                    target_name: target,
                                    kind: EdgeKind::Inherits,
                                    line: line_of(elem),
                                });
                            }
                        }
                    }
                }
            }
        }
        let signature = signature_of(node, ty, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(t) = ty {
            self.push_children(stack, t);
        }
        stack.push(Step::OpenSymbol {
            kind,
            names: vec![name],
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

    /// One import_spec → one RawImport.
    fn handle_import_spec(&mut self, node: Node) {
        let Some(path_node) = node.child_by_field_name("path") else {
            return;
        };
        let raw = self.text(path_node);
        let path = raw.trim_matches('"').trim_matches('`').to_string();
        let segs: Vec<&str> = path.split('/').collect();
        let internal = segs.first().map(|f| *f == self.pkg).unwrap_or(false);
        let to = if internal {
            segs.join("::")
        } else {
            format!("external::{}", segs.first().copied().unwrap_or(""))
        };
        let bindings: Vec<String> = match node.child_by_field_name("name") {
            Some(n) if n.kind() == "package_identifier" => vec![self.text(n)],
            Some(n) if n.kind() == "dot" || n.kind() == "blank_identifier" => Vec::new(),
            Some(_) => Vec::new(),
            None => vec![segs.last().copied().unwrap_or("").to_string()],
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings,
            line: line_of(node),
        });
    }
}

/// The spec children of a var/const declaration (directly, or
/// inside a spec list).
fn spec_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    for c in children(node) {
        if matches!(c.kind(), "var_spec" | "const_spec") {
            out.push(c);
        } else {
            for s in children(c) {
                if matches!(s.kind(), "var_spec" | "const_spec") {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// The name nodes of a spec (the `name` field may repeat).
fn name_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children_by_field_name("name", &mut cursor).collect()
}

/// The base type name of a type node: unwrap pointers and
/// generics; a qualified type contributes its name.
fn base_type_name(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(txt(node, src)),
        "qualified_type" => node
            .child_by_field_name("name")
            .map(|n| txt(n, src)),
        "pointer_type" | "parenthesized_type" => (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .next()
            .and_then(|n| base_type_name(n, src)),
        "generic_type" => (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .next()
            .and_then(|n| base_type_name(n, src)),
        _ => None,
    }
}

/// Decision points in a body (the Go form of §3.2's enumeration).
/// Iterative (E-1); nested declarations pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "function_declaration" | "method_declaration" | "type_declaration"
        ) {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "for_statement" => 1,
            "expression_switch_statement" => {
                let cases = count_kind_in(node, "expression_case")
                    + count_kind_in(node, "default_case");
                cases.saturating_sub(1)
            }
            "type_switch_statement" => {
                let cases =
                    count_kind_in(node, "type_case") + count_kind_in(node, "default_case");
                cases.saturating_sub(1)
            }
            "select_statement" => {
                let cases = count_kind_in(node, "communication_case")
                    + count_kind_in(node, "default_case");
                cases.saturating_sub(1)
            }
            "binary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| txt(o, src))
                    .unwrap_or_default();
                if matches!(op.as_str(), "&&" | "||") {
                    1
                } else {
                    0
                }
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
// The defines/refs/writes scan (spec §3.6), Go form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): receiver and
/// parameter names (seeded by the caller for methods), the body's
/// own scan, the parameter/result types' names, minus the
/// defines from the refs.
fn fn_body_info(
    src: &[u8],
    params_list: Option<Node>,
    result: Option<Node>,
    body: Option<Node>,
    extra_defines: &[String],
    extra_refs: &[String],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = extra_defines.iter().cloned().collect();
    let mut refs: HashSet<String> = extra_refs.iter().cloned().collect();
    let mut writes: HashSet<String> = HashSet::new();
    if let Some(pl) = params_list {
        scan_params(pl, src, &mut defines, &mut refs);
    }
    if let Some(r) = result {
        if r.kind() == "parameter_list" {
            // Named results are defines.
            scan_params(r, src, &mut defines, &mut refs);
        } else {
            scan_type_names(r, src, &mut refs);
        }
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

/// Parameter names → defines; parameter types → refs. A
/// parameter_declaration's `name` field may repeat (`a, b int`).
fn scan_params(
    list: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
) {
    for p in children(list) {
        if !matches!(p.kind(), "parameter_declaration" | "variadic_parameter_declaration") {
            continue;
        }
        for n in name_children(p) {
            let name = txt(n, src);
            if name != "_" {
                defines.insert(name);
            }
        }
        if let Some(t) = p.child_by_field_name("type") {
            scan_type_names(t, src, refs);
        }
    }
}

/// Collect the type names a type node mentions as refs.
fn scan_type_names(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    match node.kind() {
        "type_identifier" => {
            refs.insert(txt(node, src));
        }
        "qualified_type" => {
            if let Some(n) = node.child_by_field_name("name") {
                refs.insert(txt(n, src));
            }
        }
        _ => {
            for c in children(node) {
                scan_type_names(c, src, refs);
            }
        }
    }
}

/// One walk computing defines/refs/writes for a body, on an
/// explicit stack of scan steps.
fn scan_w(
    root: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
    writes: &mut HashSet<String>,
) {
    enum S<'t> {
        N(Node<'t>),
        SelectorOperand(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::SelectorOperand(s) => {
                // Only the operand of a selector is a read.
                if let Some(o) = s.child_by_field_name("operand") {
                    stack.push(S::N(o));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested declarations are separate symbols.
            "function_declaration" | "method_declaration" | "type_declaration" => {}
            "short_var_declaration" => {
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(S::N(r));
                }
                if let Some(l) = node.child_by_field_name("left") {
                    for e in children(l) {
                        if e.kind() == "identifier" {
                            let name = txt(e, src);
                            if name != "_" {
                                defines.insert(name.clone());
                                writes.insert(name);
                            }
                        } else {
                            stack.push(S::N(e));
                        }
                    }
                }
            }
            "assignment_statement" => {
                let op = assignment_operator(node, src);
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(S::N(r));
                }
                if let Some(l) = node.child_by_field_name("left") {
                    for e in children(l) {
                        match e.kind() {
                            "identifier" => {
                                let name = txt(e, src);
                                if op == "=" {
                                    if name != "_" {
                                        defines.insert(name.clone());
                                        writes.insert(name);
                                    }
                                } else {
                                    refs.insert(name);
                                }
                            }
                            "selector_expression" => {
                                if op == "=" {
                                    if let Some(f) = e.child_by_field_name("field") {
                                        writes.insert(txt(f, src));
                                    }
                                }
                                stack.push(S::SelectorOperand(e));
                            }
                            _ => stack.push(S::N(e)),
                        }
                    }
                }
            }
            "var_declaration" | "const_declaration" => {
                for spec in spec_children(node) {
                    if let Some(t) = spec.child_by_field_name("type") {
                        scan_type_names(t, src, refs);
                    }
                    let value = spec.child_by_field_name("value");
                    for n in name_children(spec) {
                        let name = txt(n, src);
                        if name != "_" {
                            defines.insert(name.clone());
                            if value.is_some() {
                                writes.insert(name);
                            }
                        }
                    }
                    if let Some(v) = value {
                        stack.push(S::N(v));
                    }
                }
            }
            "inc_statement" | "dec_statement" => {
                for c in children(node) {
                    if c.kind() == "selector_expression" {
                        stack.push(S::SelectorOperand(c));
                    } else if c.kind() != "identifier" {
                        stack.push(S::N(c));
                    }
                }
            }
            "selector_expression" => stack.push(S::SelectorOperand(node)),
            "composite_literal" => {
                if let Some(t) = node.child_by_field_name("type") {
                    scan_type_names(t, src, refs);
                }
                if let Some(b) = node.child_by_field_name("body") {
                    stack.push(S::N(b));
                }
            }
            "keyed_element" => {
                // The key is a field name, not a read; the value is.
                if let Some(v) = node.child_by_field_name("value") {
                    stack.push(S::N(v));
                }
            }
            "identifier" | "type_identifier" => {
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

/// The operator token of an assignment_statement (`=`, `+=`, …):
/// an unnamed child whose kind is the operator text.
fn assignment_operator(node: Node, src: &[u8]) -> String {
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        let k = c.kind();
        if matches!(
            k,
            "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "&=" | "|=" | "^=" | "&^=" | "<<=" | ">>="
        ) {
            return txt(c, src);
        }
    }
    "=".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn one(rel: &str, src: &str) -> FileSymbols {
        extract("quarry", &PathBuf::from(rel), src).unwrap()
    }

    #[test]
    fn module_id_is_package_plus_directory() {
        let f = one("shapes/circle.go", "package shapes\n");
        assert_eq!(f.module_id, "quarry::shapes");
        let g = one("main.go", "package main\n");
        assert_eq!(g.module_id, "quarry");
        let h = one("a/b/c.go", "package c\n");
        assert_eq!(h.module_id, "quarry::a::b");
    }

    #[test]
    fn methods_nest_under_the_receiver_type() {
        let f = one(
            "shapes/circle.go",
            "package shapes\ntype Circle struct { Radius float64 }\nfunc (c Circle) Area() float64 { return c.Radius }\n",
        );
        let ids: Vec<&str> = f.symbols.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["quarry::shapes::Circle", "quarry::shapes::Circle::Area"]
        );
        let area = &f.symbols[1];
        assert_eq!(area.kind, NodeKind::Method);
        assert_eq!(area.defines, vec!["c".to_string()]);
        assert_eq!(area.refs, vec!["Circle".to_string(), "float64".to_string()]);
    }

    #[test]
    fn imports_paths_aliases_and_external() {
        let f = one(
            "main.go",
            "package main\nimport (\n \"fmt\"\n \"quarry/core\"\n alias \"quarry/shapes\"\n . \"quarry/util\"\n _ \"quarry/blank\"\n)\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("external::fmt", vec!["fmt".to_string()]),
                ("quarry::core", vec!["core".to_string()]),
                ("quarry::shapes", vec!["alias".to_string()]),
                ("quarry::util", vec![]),
                ("quarry::blank", vec![]),
            ]
        );
    }

    #[test]
    fn interface_methods_and_embedding() {
        let f = one(
            "s/s.go",
            "package s\ntype Reader interface { Read(p []byte) (int, error) }\ntype ReadCloser interface { Reader; Close() error }\n",
        );
        let ids: Vec<&str> = f.symbols.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"quarry::s::Reader"), "{ids:?}");
        assert!(ids.contains(&"quarry::s::Reader::Read"), "{ids:?}");
        assert!(ids.contains(&"quarry::s::ReadCloser::Close"), "{ids:?}");
        assert_eq!(f.heritage.len(), 1);
        assert_eq!(f.heritage[0].type_name, "ReadCloser");
        assert_eq!(f.heritage[0].target_name, "Reader");
        assert_eq!(f.heritage[0].kind, EdgeKind::Inherits);
    }

    #[test]
    fn scan_assignment_forms() {
        let f = one(
            "f.go",
            "package f\nfunc run() int { x := 1; x = 2; x += 3; y := helper(x); return y }\n",
        );
        let run = f.symbols.iter().find(|s| s.label == "run").unwrap();
        assert_eq!(run.defines, vec!["x".to_string(), "y".to_string()]);
        assert_eq!(run.writes, vec!["x".to_string(), "y".to_string()]);
        assert_eq!(run.refs, vec!["helper".to_string(), "int".to_string()]);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("package d\nfunc f(x int) int { return ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1 }\n");
        let f = one("d.go", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
