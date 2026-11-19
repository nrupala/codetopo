// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! C++ extractor (`.cpp`, `.hpp`, `.cc`, `.cxx`, `.hh`, `.hxx`) —
//! beside Rust, TypeScript/JavaScript, Python, Kotlin, C, Java,
//! Go, and Shell. (`.h` files are extracted as C: one extension,
//! one extractor.)
//!
//! Node kinds below were verified against
//! `tree-sitter-cpp-0.23.4/src/node-types.json` in the crates.io
//! registry cache (the pinned crate, never guessed):
//! `translation_unit`, `namespace_definition` (fields `name`,
//! `body`), `class_specifier` / `struct_specifier` /
//! `union_specifier` / `enum_specifier` (fields `name`, `body`;
//! a `base_class_clause` child carries heritage),
//! `function_definition` (fields `body`, `declarator`, `type`),
//! `field_declaration` (fields `declarator`, `type`,
//! `default_value`), `declaration`, `type_definition`,
//! `alias_declaration`, `using_declaration`,
//! `qualified_identifier` (fields `scope`, `name`),
//! `preproc_include` (field `path`), `preproc_def`,
//! `preproc_function_def`, `call_expression` (field `function`),
//! `assignment_expression` (fields `left`, `operator`, `right`),
//! `field_expression` (fields `argument`, `field`),
//! `field_initializer`, `parameter_declaration`.
//!
//! The mapping extends the C extractor's (the grammars share
//! their C core): module ids are path-derived, and a header and
//! its implementation share a module. Namespace names are
//! symbol-path segments (a namespace is a scope, not a symbol).
//! Symbols (§3.2 — function definitions and declarations are
//! functions; a qualified or in-class function is a method, so
//! an out-of-line `Circle::area()` definition composes the same
//! id as its in-class declaration; class/struct/union/enum
//! definitions, typedef names, and `using` aliases are classes;
//! enumerators are variables scoped under their enum; file- and
//! namespace-scope variables are variables; function-like
//! macros are functions and object-like macros with a value are
//! variables; fields are not symbols — a field_declaration whose
//! declarator is a function declarator is a method declaration,
//! any other is a field). Imports (`#include`, exactly the C
//! rule; plus `using` declarations: a using of a qualified name
//! is an import of the qualifier's module with the last segment
//! as its binding, and `using namespace` binds nothing).
//! Calls (§3.4 — the callee text as written). Heritage
//! (base-class clauses are Inherits — C++ has one inheritance
//! relation). The defines/refs/writes scan is the C scan plus
//! member initializers (a field initializer writes its field).
//! Decisions are the C set plus range-`for` +1. Template
//! declarations are transparent: the declared name is the
//! symbol, parameters are not part of the id. The walk runs on
//! an explicit heap stack (the E-1 lesson).

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
        .set_language(&tree_sitter_cpp::LANGUAGE.into())
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
        rel_path: rel_path.to_path_buf(),
        path: Vec::new(),
        path_counts: Vec::new(),
        ns_counts: Vec::new(),
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
    pkg: String,
    module_id: String,
    rel_path: std::path::PathBuf,
    path: Vec<String>,
    path_counts: Vec<usize>,
    ns_counts: Vec<usize>,
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
    EnterNamespace {
        names: Vec<String>,
    },
    LeaveNamespace,
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
                Step::EnterNamespace { names } => {
                    self.ns_counts.push(names.len());
                    for n in names {
                        self.path.push(n);
                    }
                }
                Step::LeaveNamespace => {
                    let n = self.ns_counts.pop().unwrap_or(0);
                    for _ in 0..n {
                        self.path.pop();
                    }
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
            "namespace_definition" => {
                let names = namespace_names(node, self.src);
                stack.push(Step::LeaveNamespace);
                if let Some(b) = node.child_by_field_name("body") {
                    self.push_children(stack, b);
                }
                stack.push(Step::EnterNamespace { names });
            }
            "function_definition" => self.schedule_function_def(node, stack),
            "declaration" if self.enclosing.is_empty() => {
                self.handle_file_declaration(node, stack)
            }
            "type_definition" if self.enclosing.is_empty() => {
                self.handle_typedef(node, stack)
            }
            "alias_declaration" if self.enclosing.is_empty() => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    self.push_symbol(
                        NodeKind::Class,
                        &[name],
                        node,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Some(end_line_of(node)),
                        signature_of(node, node.child_by_field_name("type"), self.src),
                        None,
                    );
                    self.pop_symbol();
                }
            }
            "using_declaration" if self.enclosing.is_empty() => {
                self.handle_using(node)
            }
            // A bare specifier at file/namespace scope is a
            // translation-unit child in its own right.
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier"
                if self.enclosing.is_empty()
                    && node.child_by_field_name("body").is_some() =>
            {
                self.schedule_specifier_class(node, node, stack)
            }
            // A method declaration inside a class body.
            "field_declaration" if self.in_class => {
                self.handle_field_declaration(node, stack)
            }
            // A declaration inside a class body: a constructor
            // (no return type, so the grammar uses `declaration`,
            // not `field_declaration`), a nested type, or a
            // static member (a field — not a symbol).
            "declaration" if self.in_class => {
                self.handle_class_declaration(node, stack)
            }
            "enumerator" if self.in_class => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    self.push_symbol(
                        NodeKind::Variable,
                        &[name],
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
            "preproc_include" => self.handle_include(node),
            "preproc_def" if self.enclosing.is_empty() => {
                if node.child_by_field_name("value").is_some() {
                    if let Some(n) = node.child_by_field_name("name") {
                        let name = self.text(n);
                        self.push_symbol(
                            NodeKind::Variable,
                            &[name],
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
                }
            }
            "preproc_function_def" if self.enclosing.is_empty() => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    self.push_symbol(
                        NodeKind::Function,
                        &[name],
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
            .unwrap_or_else(|| {
                if self.path.is_empty() {
                    self.module_id.clone()
                } else {
                    format!("{}::{}", self.module_id, self.path.join("::"))
                }
            })
    }

    /// One `call_expression` → one RawCall; `to_name` is the
    /// `function` field's text as written, trimmed (§3.4).
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

    /// Schedule a function definition. A qualified declarator
    /// name (`Circle::area`) makes a method whose id composes
    /// the qualifier segments — the same id its in-class
    /// declaration carries.
    fn schedule_function_def<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(decl) = node.child_by_field_name("declarator") else {
            self.push_children(stack, node);
            return;
        };
        let segments = declarator_segments(decl, self.src);
        if segments.is_empty() {
            self.push_children(stack, node);
            return;
        }
        let params = find_params(decl);
        let body = node.child_by_field_name("body");
        // Member initializers scan with the body (a field
        // initializer writes its field).
        let inits: Vec<Node> = (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .filter(|c| c.kind() == "field_initializer_list")
            .collect();
        let (defines, mut refs, writes) = fn_body_info(self.src, params, body, &inits);
        if let Some(t) = node.child_by_field_name("type") {
            let mut set: HashSet<String> = refs.iter().cloned().collect();
            scan_type_names(t, self.src, &mut set);
            refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
            refs.sort();
        }
        let kind = if self.in_class || segments.len() > 1 {
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
            names: segments,
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

    /// A declaration inside a class body: a function declarator
    /// makes a method declaration (constructors land here — they
    /// have no return type); a specifier with a body is a nested
    /// class; anything else is a static member or a field alias,
    /// not a symbol.
    fn handle_class_declaration<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        if let Some(spec) = specifier_with_body(node) {
            self.schedule_specifier_class(spec, node, stack);
            return;
        }
        for decl in declarator_children(node) {
            if declarator_core(decl).kind() == "function_declarator" {
                let segments = declarator_segments(decl, self.src);
                if segments.is_empty() {
                    continue;
                }
                let (defines, mut refs, writes) =
                    fn_body_info(self.src, find_params(decl), None, &[]);
                if let Some(t) = node.child_by_field_name("type") {
                    let mut set: HashSet<String> = refs.iter().cloned().collect();
                    scan_type_names(t, self.src, &mut set);
                    refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
                    refs.sort();
                }
                self.push_symbol(
                    NodeKind::Method,
                    &segments,
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
    }

    /// A field_declaration inside a class body: a function
    /// declarator makes a method declaration; anything else is a
    /// field (not a symbol — its default value still walks, so
    /// calls in member initializers are recorded).
    fn handle_field_declaration<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        for decl in declarator_children(node) {
            let core = declarator_core(decl);
            if core.kind() == "function_declarator" {
                let segments = declarator_segments(decl, self.src);
                if segments.is_empty() {
                    continue;
                }
                let (defines, mut refs, writes) =
                    fn_body_info(self.src, find_params(decl), None, &[]);
                if let Some(t) = node.child_by_field_name("type") {
                    let mut set: HashSet<String> = refs.iter().cloned().collect();
                    scan_type_names(t, self.src, &mut set);
                    refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
                    refs.sort();
                }
                self.push_symbol(
                    NodeKind::Method,
                    &segments,
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
        // Default values and initializers walk in the class scope.
        for decl in declarator_children(node) {
            if let Some(v) = init_value(decl) {
                stack.push(Step::Enter(v));
            }
        }
        if let Some(v) = node.child_by_field_name("default_value") {
            stack.push(Step::Enter(v));
        }
    }

    /// A file/namespace-scope `declaration`: a specifier with a
    /// body (a class), function prototypes (functions — methods
    /// when qualified), and variables, in any combination.
    fn handle_file_declaration<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let ty = node.child_by_field_name("type");
        for decl in declarator_children(node) {
            let core = declarator_core(decl);
            if core.kind() == "function_declarator" {
                let segments = declarator_segments(decl, self.src);
                if segments.is_empty() {
                    continue;
                }
                let (defines, mut refs, writes) =
                    fn_body_info(self.src, find_params(decl), None, &[]);
                if let Some(t) = ty {
                    let mut set: HashSet<String> = refs.iter().cloned().collect();
                    scan_type_names(t, self.src, &mut set);
                    refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
                    refs.sort();
                }
                let kind = if segments.len() > 1 {
                    NodeKind::Method
                } else {
                    NodeKind::Function
                };
                self.push_symbol(
                    kind,
                    &segments,
                    node,
                    defines,
                    refs,
                    writes,
                    Some(end_line_of(node)),
                    signature_of(node, None, self.src),
                    None,
                );
                self.pop_symbol();
            } else if let Some(name) = declarator_name(decl, self.src) {
                let init = init_value(decl);
                let mut refs: Vec<String> =
                    init.map(|v| body_refs(v, self.src)).unwrap_or_default();
                refs.sort();
                refs.dedup();
                self.push_symbol(
                    NodeKind::Variable,
                    &[name],
                    node,
                    Vec::new(),
                    refs,
                    Vec::new(),
                    Some(end_line_of(node)),
                    signature_of(node, init, self.src),
                    None,
                );
                self.pop_symbol();
                if let Some(v) = init {
                    stack.push(Step::Enter(v));
                }
            }
        }
        if let Some(spec) = specifier_with_body(node) {
            self.schedule_specifier_class(spec, node, stack);
        }
    }

    /// `typedef … Name;` — the alias name is a class (the Python
    /// PEP 695 precedent). When the underlying specifier has a
    /// body, the class opens so enumerators nest.
    fn handle_typedef<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(decl) = node.child_by_field_name("declarator") else {
            self.push_children(stack, node);
            return;
        };
        let Some(name) = declarator_name(decl, self.src) else {
            self.push_children(stack, node);
            return;
        };
        if let Some(spec) = specifier_with_body(node) {
            self.collect_heritage(&name, spec);
            let body = spec.child_by_field_name("body");
            let signature = signature_of(node, body, self.src);
            stack.push(Step::LeaveSymbol {
                in_class: self.in_class,
            });
            if let Some(b) = body {
                self.push_children(stack, b);
            }
            stack.push(Step::OpenSymbol {
                kind: NodeKind::Class,
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
            return;
        }
        self.push_symbol(
            NodeKind::Class,
            &[name],
            node,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Some(end_line_of(node)),
            signature_of(node, None, self.src),
            None,
        );
        self.pop_symbol();
    }

    /// Schedule a class/struct/union/enum definition as a class
    /// symbol, opening its scope so enum members and methods
    /// nest under it. Heritage is collected first.
    fn schedule_specifier_class<'t>(
        &mut self,
        spec: Node<'t>,
        decl_node: Node<'t>,
        stack: &mut Vec<Step<'t>>,
    ) {
        let Some(name_node) = spec.child_by_field_name("name") else {
            return;
        };
        let segments = type_segments(name_node, self.src);
        if segments.is_empty() {
            return;
        }
        let name = segments.last().cloned().unwrap_or_default();
        self.collect_heritage(&name, spec);
        let body = spec.child_by_field_name("body");
        let signature = signature_of(decl_node, body, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Class,
            names: segments,
            node: decl_node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(decl_node)),
            signature,
            decisions: None,
            in_class: true,
        });
    }

    /// Base-class clause → Inherits rows (§3.3).
    fn collect_heritage(&mut self, type_name: &str, spec: Node) {
        for c in children(spec) {
            if c.kind() != "base_class_clause" {
                continue;
            }
            for part in children(c) {
                let target = match part.kind() {
                    "type_identifier" => Some(self.text(part)),
                    "qualified_identifier" | "template_type" => {
                        type_segments(part, self.src).last().cloned()
                    }
                    _ => None,
                };
                if let Some(target) = target {
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

    /// A `using` declaration → one RawImport: the qualifier path
    /// is the module, the last segment is the binding.
    /// `using namespace a::b;` targets the namespace module and
    /// binds nothing. A path whose first segment is the package
    /// name is already a module path; any other path is resolved
    /// under the package.
    fn handle_using(&mut self, node: Node) {
        let mut cursor = node.walk();
        let is_namespace = node
            .children(&mut cursor)
            .any(|c| c.kind() == "namespace");
        for c in children(node) {
            let segs = type_segments(c, self.src);
            if segs.is_empty() {
                continue;
            }
            let (module_segs, bindings) = if is_namespace {
                (segs.clone(), Vec::new())
            } else if segs.len() > 1 {
                (segs[..segs.len() - 1].to_vec(), vec![segs[segs.len() - 1].clone()])
            } else {
                (segs.clone(), Vec::new())
            };
            let to = if module_segs.is_empty() {
                self.module_id.clone()
            } else if module_segs[0] == self.pkg {
                module_segs.join("::")
            } else {
                format!("{}::{}", self.pkg, module_segs.join("::"))
            };
            self.imports.push(RawImport {
                from_module_id: self.module_id.clone(),
                to_module_id: to,
                bindings,
                line: line_of(node),
            });
        }
    }

    /// `#include` → one RawImport (exactly the C rule).
    fn handle_include(&mut self, node: Node) {
        let Some(path_node) = node.child_by_field_name("path") else {
            return;
        };
        let raw = self.text(path_node);
        let to = match path_node.kind() {
            "system_lib_string" => {
                let inner = raw.trim_matches(|c| c == '<' || c == '>');
                let first = inner.split('/').next().unwrap_or("");
                let stem = match first.rsplit_once('.') {
                    Some((s, _)) => s,
                    None => first,
                };
                format!("external::{stem}")
            }
            _ => {
                let inner = raw.trim_matches('"');
                self.resolve_include(inner)
            }
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings: Vec::new(),
            line: line_of(node),
        });
    }

    /// Resolve a quoted include path against the including
    /// file's directory segments.
    fn resolve_include(&self, include: &str) -> String {
        let mut segs: Vec<String> = Vec::new();
        if let Some(parent) = self.rel_path.parent() {
            for comp in parent.components() {
                if let std::path::Component::Normal(s) = comp {
                    segs.push(s.to_string_lossy().into_owned());
                }
            }
        }
        for part in include.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if segs.pop().is_none() {
                        return "external::unresolved".to_string();
                    }
                }
                p => segs.push(p.to_string()),
            }
        }
        if let Some(last) = segs.last_mut() {
            if let Some((stem, _)) = last.rsplit_once('.') {
                *last = stem.to_string();
            }
        }
        if segs.is_empty() {
            "external::unresolved".to_string()
        } else {
            format!("{}::{}", self.pkg, segs.join("::"))
        }
    }
}

/// The segments of a namespace name: a plain identifier, or a
/// nested namespace specifier (`a::b::c` as text).
fn namespace_names(node: Node, src: &[u8]) -> Vec<String> {
    match node.child_by_field_name("name") {
        Some(n) => txt(n, src)
            .split("::")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect(),
        None => Vec::new(),
    }
}

/// The segments a type reference denotes: a plain identifier is
/// one segment; a qualified identifier contributes its scope
/// segments plus its name; a template type contributes its base.
fn type_segments(node: Node, src: &[u8]) -> Vec<String> {
    match node.kind() {
        "type_identifier" | "identifier" | "namespace_identifier" | "field_identifier" => {
            vec![txt(node, src)]
        }
        "qualified_identifier" => {
            let mut out = Vec::new();
            if let Some(scope) = node.child_by_field_name("scope") {
                out.extend(type_segments(scope, src));
            }
            if let Some(name) = node.child_by_field_name("name") {
                out.extend(type_segments(name, src));
            }
            out
        }
        "template_type" => (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .next()
            .map(|n| type_segments(n, src))
            .unwrap_or_default(),
        "destructor_name" | "operator_name" => vec![txt(node, src)],
        _ => Vec::new(),
    }
}

/// The core declarator under pointer / reference / array /
/// parenthesized wrappers.
fn declarator_core(node: Node) -> Node {
    let mut cur = node;
    loop {
        match cur.kind() {
            "init_declarator" | "pointer_declarator" | "array_declarator"
            | "parenthesized_declarator" | "attributed_declarator" => {
                match cur.child_by_field_name("declarator") {
                    Some(d) => cur = d,
                    None => return cur,
                }
            }
            "reference_declarator" => {
                match (0..cur.named_child_count())
                    .filter_map(|i| cur.named_child(i))
                    .next()
                {
                    Some(d) => cur = d,
                    None => return cur,
                }
            }
            _ => return cur,
        }
    }
}

/// The name segments a declarator binds: the plain name, or the
/// qualifier segments plus the name for a qualified declarator.
fn declarator_segments(node: Node, src: &[u8]) -> Vec<String> {
    let core = declarator_core(node);
    match core.kind() {
        "function_declarator" => match core.child_by_field_name("declarator") {
            Some(inner) => declarator_segments(inner, src),
            None => Vec::new(),
        },
        "qualified_identifier" => type_segments(core, src),
        "identifier" | "type_identifier" | "field_identifier" | "destructor_name"
        | "operator_name" => vec![txt(core, src)],
        "operator_cast" => vec![txt(core, src)],
        _ => Vec::new(),
    }
}

/// The plain name a declarator binds (variables, typedefs).
fn declarator_name(node: Node, src: &[u8]) -> Option<String> {
    declarator_segments(node, src).into_iter().last()
}

/// The parameter list of a function declarator (unwrapping
/// pointer/reference declarators to reach it).
fn find_params(node: Node) -> Option<Node> {
    let core = declarator_core(node);
    if core.kind() == "function_declarator" {
        return core.child_by_field_name("parameters");
    }
    None
}

/// The declarator children of a declaration (the `declarator`
/// field may repeat).
fn declarator_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children_by_field_name("declarator", &mut cursor)
        .collect()
}

/// The initializer value of a declarator, if it is an
/// init_declarator.
fn init_value(node: Node) -> Option<Node> {
    if node.kind() == "init_declarator" {
        node.child_by_field_name("value")
    } else {
        None
    }
}

/// The struct/union/enum/class specifier with a body among a
/// declaration's type field, if any.
fn specifier_with_body<'t>(node: Node<'t>) -> Option<Node<'t>> {
    let ty = node.child_by_field_name("type")?;
    let mut stack = vec![ty];
    while let Some(n) = stack.pop() {
        if matches!(
            n.kind(),
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier"
        ) && n.child_by_field_name("body").is_some()
        {
            return Some(n);
        }
        for i in 0..n.named_child_count() {
            if let Some(c) = n.named_child(i) {
                stack.push(c);
            }
        }
    }
    None
}

/// Decision points in a body (the C enumeration plus range-for).
/// Iterative (E-1); nested function definitions are pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_definition" {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "for_statement" | "while_statement" | "do_statement"
            | "for_range_loop" | "conditional_expression" => 1,
            "switch_statement" => {
                let cases = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .filter(|c| c.kind() == "compound_statement")
                    .flat_map(children)
                    .filter(|c| c.kind() == "case_statement")
                    .count() as u32;
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

// ---------------------------------------------------------------------------
// The defines/refs/writes scan (spec §3.6), C++ form — the C scan
// plus member initializers
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): the parameter
/// names, the body's own scan, the member initializers' writes,
/// the parameter types' names, minus the defines from the refs.
fn fn_body_info(
    src: &[u8],
    params_list: Option<Node>,
    body: Option<Node>,
    init_lists: &[Node],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
    if let Some(pl) = params_list {
        for p in children(pl) {
            if p.kind() != "parameter_declaration" {
                continue;
            }
            if let Some(d) = p.child_by_field_name("declarator") {
                if let Some(name) = declarator_name(d, src) {
                    defines.insert(name);
                }
            }
            if let Some(t) = p.child_by_field_name("type") {
                scan_type_names(t, src, &mut refs);
            }
        }
    }
    for list in init_lists {
        for init in children(*list) {
            if init.kind() == "field_initializer" {
                scan_w(init, src, &mut defines, &mut refs, &mut writes);
            }
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

/// The reads of one expression (a file-scope initializer's refs).
fn body_refs(root: Node, src: &[u8]) -> Vec<String> {
    let mut defines = HashSet::new();
    let mut refs = HashSet::new();
    let mut writes = HashSet::new();
    scan_w(root, src, &mut defines, &mut refs, &mut writes);
    refs.into_iter().collect()
}

/// Collect the names a type node mentions (type identifiers and
/// primitive types) as refs.
fn scan_type_names(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "type_identifier" | "primitive_type" => {
                refs.insert(txt(n, src));
            }
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => {
                if let Some(name) = n.child_by_field_name("name") {
                    if let Some(seg) = type_segments(name, src).last() {
                        refs.insert(seg.clone());
                    }
                }
            }
            _ => {
                for c in children(n) {
                    stack.push(c);
                }
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
        FieldArg(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::FieldArg(f) => {
                if let Some(a) = f.child_by_field_name("argument") {
                    stack.push(S::N(a));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested function definitions are separate symbols;
            // a local class's fields are not reads either.
            "function_definition" | "field_declaration" => {}
            "struct_specifier" | "union_specifier" | "enum_specifier" | "class_specifier" => {
                scan_type_names(node, src, refs);
            }
            "field_initializer" => {
                // The initialized field is a write; its
                // arguments are reads.
                let named: Vec<Node> = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .collect();
                if let Some(first) = named.first() {
                    if matches!(first.kind(), "field_identifier" | "identifier") {
                        writes.insert(txt(*first, src));
                    }
                }
                for c in named.iter().skip(1) {
                    stack.push(S::N(*c));
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    stack.push(S::N(args));
                }
            }
            "declaration" => {
                if let Some(t) = node.child_by_field_name("type") {
                    scan_type_names(t, src, refs);
                }
                for decl in declarator_children(node) {
                    if declarator_core(decl).kind() == "function_declarator" {
                        if let Some(name) = declarator_name(decl, src) {
                            defines.insert(name);
                        }
                        continue;
                    }
                    if let Some(name) = declarator_name(decl, src) {
                        defines.insert(name.clone());
                        if let Some(v) = init_value(decl) {
                            writes.insert(name);
                            stack.push(S::N(v));
                        }
                    }
                }
            }
            "assignment_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| txt(o, src))
                    .unwrap_or_default();
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(S::N(r));
                }
                if let Some(l) = node.child_by_field_name("left") {
                    match l.kind() {
                        "identifier" => {
                            let name = txt(l, src);
                            if op == "=" {
                                defines.insert(name.clone());
                                writes.insert(name);
                            } else {
                                refs.insert(name);
                            }
                        }
                        "field_expression" => {
                            if let Some(f) = l.child_by_field_name("field") {
                                let fname = txt(f, src);
                                if op == "=" {
                                    writes.insert(fname);
                                }
                            }
                            stack.push(S::FieldArg(l));
                        }
                        _ => stack.push(S::N(l)),
                    }
                }
            }
            "update_expression" => {
                if let Some(a) = node.child_by_field_name("argument") {
                    if a.kind() == "field_expression" {
                        stack.push(S::FieldArg(a));
                    } else if a.kind() != "identifier" {
                        stack.push(S::N(a));
                    }
                }
            }
            "field_expression" => stack.push(S::FieldArg(node)),
            "identifier" => {
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
    fn namespaces_are_path_segments_not_symbols() {
        let f = one(
            "core/Engine.hpp",
            "namespace quarry { class Engine { public: void render(); }; }\n",
        );
        let ids: Vec<&str> = f.symbols.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "quarry::core::Engine::quarry::Engine",
                "quarry::core::Engine::quarry::Engine::render"
            ]
        );
        assert_eq!(f.symbols[1].kind, NodeKind::Method);
    }

    #[test]
    fn out_of_line_definition_composes_the_declaration_id() {
        let f = one(
            "core/Engine.cpp",
            "namespace quarry { double Engine::render() { return 1.0; } }\n",
        );
        assert_eq!(f.symbols.len(), 1);
        assert_eq!(
            f.symbols[0].id,
            "quarry::core::Engine::quarry::Engine::render"
        );
        assert_eq!(f.symbols[0].kind, NodeKind::Method);
    }

    #[test]
    fn heritage_from_base_class_clause() {
        let f = one(
            "C.hpp",
            "class Circle : public Shape, private Tag {};\n",
        );
        let rows: Vec<(&str, &str)> = f
            .heritage
            .iter()
            .map(|h| (h.type_name.as_str(), h.target_name.as_str()))
            .collect();
        assert_eq!(rows, vec![("Circle", "Shape"), ("Circle", "Tag")]);
        assert!(f.heritage.iter().all(|h| h.kind == EdgeKind::Inherits));
    }

    #[test]
    fn using_declaration_is_an_import() {
        let f = one(
            "u.cpp",
            "using quarry::shapes::Circle;\nusing namespace quarry::core;\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("quarry::shapes", vec!["Circle".to_string()]),
                ("quarry::core", vec![]),
            ]
        );
    }

    #[test]
    fn member_initializer_writes_the_field() {
        let f = one(
            "C.cpp",
            "struct P { int x; };\nP::P(int v) : x(v) {}\n",
        );
        let ctor = f
            .symbols
            .iter()
            .find(|s| s.label == "P" && s.kind == NodeKind::Method)
            .unwrap();
        assert_eq!(ctor.defines, vec!["v".to_string()]);
        assert_eq!(ctor.writes, vec!["x".to_string()]);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("int f(int x) { return ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1; }\n");
        let f = one("deep.cpp", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
