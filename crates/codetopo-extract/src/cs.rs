// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! C# extractor (`.cs`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, Java, Go, Shell, C++, Swift, Ruby, R,
//! and Julia.
//!
//! Node kinds below were verified against
//! `tree-sitter-c-sharp-0.24.0/src/node-types.json` in the
//! crates.io registry cache (the pinned crate, never guessed):
//! `compilation_unit`, `using_directive`,
//! `namespace_declaration` (fields `name`, `body`),
//! `file_scoped_namespace_declaration` (field `name`),
//! `class_declaration` / `interface_declaration` /
//! `struct_declaration` / `record_declaration` /
//! `enum_declaration` (fields `name`, `body`; a `base_list`
//! child carries heritage), `method_declaration` (fields
//! `name`, `parameters`, `returns`, `body`),
//! `constructor_declaration` (fields `name`, `parameters`,
//! `body`), `property_declaration`, `field_declaration`,
//! `enum_member_declaration` (field `name`),
//! `invocation_expression` (fields `function`, `arguments`),
//! `member_access_expression` (fields `expression`, `name`),
//! `object_creation_expression` (field `type`),
//! `assignment_expression` (fields `left`, `operator`,
//! `right`), `variable_declaration` (field `type`),
//! `variable_declarator` (field `name`), `parameter` (fields
//! `name`, `type`), `qualified_name` (fields `qualifier`,
//! `name`).
//!
//! The mapping follows the Java extractor's spec shape (the
//! languages are close kin), with the C++ namespace rule:
//! module ids are path-derived; namespace names are
//! symbol-path segments (a file-scoped namespace prefixes the
//! whole file). Symbols (§3.2 — classes, structs, and records
//! are classes; interfaces are interfaces; methods and
//! constructors are methods; enum members are variables
//! scoped under their enum; top-level statement variables are
//! variables; properties and fields are not symbols).
//! Imports (`using` directives: internal when the first
//! segment is the file's own namespace root, or matches the
//! package name case-insensitively — the root segment is
//! read as the package name, so the target is the package
//! plus the remaining segments — else `external::{first
//! segment}`; a plain using binds nothing, an alias using
//! binds the alias, a static using binds the type name).
//! Calls (§3.4 — the callee text as written; object creation
//! is a call on the type name). Heritage (base list: for a
//! class, struct, or record the first type is Inherits and
//! the rest are Implements; for an interface all are
//! Inherits). The defines/refs/writes scan (§3.6's marimo
//! discipline, Java form: nested declarations pruned, reads
//! that are also defines dropped, augmented-assignment
//! targets read not written, only the expression of a member
//! access is a read, an invoked method's name is a ref);
//! decisions (§3.2's enumeration, C# form: `if` +1,
//! `for`/`foreach`/`while`/`do` +1, `switch` contributes
//! sections − 1, ternary +1, each `&&`/`||` +1). The walk
//! runs on an explicit heap stack (the E-1 lesson).

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
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;
    let module_id = module_id_for(package, rel_path);
    let root = tree.root_node();
    // A file-scoped namespace prefixes every type in the file,
    // and the file's namespace root anchors the using rule.
    let mut initial_path: Vec<String> = Vec::new();
    let mut ns_root: Option<String> = None;
    for c in children(root) {
        if c.kind() == "file_scoped_namespace_declaration" {
            if let Some(n) = c.child_by_field_name("name") {
                initial_path = name_segments(n, source.as_bytes());
                ns_root = initial_path.first().cloned();
            }
        }
        if c.kind() == "namespace_declaration" && ns_root.is_none() {
            if let Some(n) = c.child_by_field_name("name") {
                ns_root = name_segments(n, source.as_bytes()).first().cloned();
            }
        }
    }
    let mut w = Walker {
        src: source.as_bytes(),
        pkg: package.to_string(),
        ns_root,
        module_id: module_id.clone(),
        path: initial_path,
        path_counts: Vec::new(),
        ns_counts: Vec::new(),
        enclosing: Vec::new(),
        in_class: false,
        symbols: Vec::new(),
        imports: Vec::new(),
        calls: Vec::new(),
        heritage: Vec::new(),
    };
    w.walk(root);
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
    /// The file's own namespace root segment (from its first
    /// namespace declaration), anchoring the using rule.
    ns_root: Option<String>,
    module_id: String,
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
            "namespace_declaration" => {
                let names = node
                    .child_by_field_name("name")
                    .map(|n| name_segments(n, self.src))
                    .unwrap_or_default();
                stack.push(Step::LeaveNamespace);
                if let Some(b) = node.child_by_field_name("body") {
                    self.push_children(stack, b);
                }
                stack.push(Step::EnterNamespace { names });
            }
            "class_declaration" | "struct_declaration" | "record_declaration" => {
                self.schedule_type(node, NodeKind::Class, stack)
            }
            "interface_declaration" => {
                self.schedule_type(node, NodeKind::Interface, stack)
            }
            "enum_declaration" => self.schedule_type(node, NodeKind::Class, stack),
            "method_declaration" => self.schedule_method(node, false, stack),
            "constructor_declaration" => self.schedule_method(node, true, stack),
            "enum_member_declaration" if self.in_class => {
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
            "using_directive" => self.handle_using(node),
            "local_declaration_statement" if self.enclosing.is_empty() => {
                // A top-level statement variable.
                for (name, value) in declarators_of(node, self.src) {
                    let mut refs: Vec<String> =
                        value.map(|v| body_refs(v, self.src)).unwrap_or_default();
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
                        signature_of(node, value, self.src),
                        None,
                    );
                    self.pop_symbol();
                    if let Some(v) = value {
                        stack.push(Step::Enter(v));
                    }
                }
            }
            "invocation_expression" => {
                self.record_call(node);
                self.push_children(stack, node);
            }
            "object_creation_expression" => {
                if let Some(t) = node.child_by_field_name("type") {
                    if let Some(name) = type_name_of(t, self.src) {
                        self.calls.push(RawCall {
                            from_id: self.current_scope(),
                            to_name: name,
                            line: line_of(node),
                        });
                    }
                }
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

    /// One `invocation_expression` → one RawCall; `to_name` is
    /// the callee's text as written (§3.4).
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

    /// Schedule a type declaration: heritage is read from the
    /// base list now; the body walks under the opened symbol.
    fn schedule_type<'t>(&mut self, node: Node<'t>, kind: NodeKind, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        // Heritage (§3.3): for a class/struct/record the first
        // base type is Inherits and the rest are Implements;
        // for an interface all are Inherits. An enum's base
        // list is its underlying type — not heritage.
        if node.kind() != "enum_declaration" {
            for c in children(node) {
                if c.kind() != "base_list" {
                    continue;
                }
                let types: Vec<Node> = children(c)
                    .into_iter()
                    .filter(|t| !matches!(t.kind(), "attribute_list" | "argument_list"))
                    .collect();
                for (i, t) in types.iter().enumerate() {
                    if let Some(target) = type_name_of(*t, self.src) {
                        let edge = if kind == NodeKind::Interface || i == 0 {
                            EdgeKind::Inherits
                        } else {
                            EdgeKind::Implements
                        };
                        self.heritage.push(RawHeritage {
                            type_name: name.clone(),
                            target_name: target,
                            kind: edge,
                            line: line_of(c),
                        });
                    }
                }
            }
        }
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

    /// Schedule a method or constructor.
    fn schedule_method<'t>(&mut self, node: Node<'t>, is_ctor: bool, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let params = node.child_by_field_name("parameters");
        let body = node.child_by_field_name("body");
        let (defines, mut refs, writes) = fn_body_info(self.src, params, body);
        if !is_ctor {
            if let Some(t) = node.child_by_field_name("returns") {
                let mut set: HashSet<String> = refs.iter().cloned().collect();
                scan_type_names(t, self.src, &mut set);
                refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
                refs.sort();
            }
        }
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

    /// A `using` directive → one RawImport. Plain: the dotted
    /// path is the module, no bindings. Alias (`using Foo =
    /// X.Y;`): binds the alias. Static (`using static X.Y;`):
    /// the path minus its last segment is the module, the last
    /// segment binds.
    fn handle_using(&mut self, node: Node) {
        let mut cursor = node.walk();
        let is_static = node
            .children(&mut cursor)
            .any(|c| c.kind() == "static");
        let kids = children(node);
        // The type node is the child that is not an alias
        // identifier: with an alias there are two named children
        // (alias identifier, then the type).
        let (segs, bindings): (Vec<String>, Vec<String>) = if kids.len() >= 2
            && kids[0].kind() == "identifier"
        {
            (
                name_segments(kids[1], self.src),
                vec![self.text(kids[0])],
            )
        } else if let Some(first) = kids.first() {
            let s = name_segments(*first, self.src);
            if is_static && s.len() > 1 {
                (s[..s.len() - 1].to_vec(), vec![s[s.len() - 1].clone()])
            } else {
                (s, Vec::new())
            }
        } else {
            return;
        };
        if segs.is_empty() {
            return;
        }
        // Internal when the first segment is the file's own
        // namespace root (or the package name, compared
        // case-insensitively — C# namespace roots are
        // PascalCase where package names are not): the root
        // segment is read as the package name, so the target
        // is the package plus the remaining segments.
        let internal = self
            .ns_root
            .as_deref()
            .map(|r| r == segs[0])
            .unwrap_or(false)
            || segs[0].eq_ignore_ascii_case(&self.pkg);
        let to = if internal {
            if segs.len() > 1 {
                format!("{}::{}", self.pkg, segs[1..].join("::"))
            } else {
                self.pkg.clone()
            }
        } else {
            format!("external::{}", segs[0])
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings,
            line: line_of(node),
        });
    }
}

/// The segments of a qualified name (or a plain identifier).
fn name_segments(node: Node, src: &[u8]) -> Vec<String> {
    match node.kind() {
        "identifier" => vec![txt(node, src)],
        "qualified_name" => {
            let mut out = Vec::new();
            if let Some(q) = node.child_by_field_name("qualifier") {
                out.extend(name_segments(q, src));
            }
            if let Some(n) = node.child_by_field_name("name") {
                out.extend(name_segments(n, src));
            }
            out
        }
        "generic_name" => (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .find(|c| c.kind() == "identifier")
            .map(|n| vec![txt(n, src)])
            .unwrap_or_default(),
        "alias_qualified_name" => {
            let mut out = Vec::new();
            for c in children(node) {
                out.extend(name_segments(c, src));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// The name a type node denotes: the last segment.
fn type_name_of(node: Node, src: &[u8]) -> Option<String> {
    let segs = name_segments(node, src);
    if !segs.is_empty() {
        return segs.into_iter().last();
    }
    match node.kind() {
        "predefined_type" => Some(txt(node, src)),
        _ => {
            // Other type forms (nullable, array, tuple…): the
            // first identifier inside is the element type name.
            (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .find_map(|c| type_name_of(c, src))
        }
    }
}

/// The (name, initializer) pairs of a declaration statement's
/// variable declarators. Accepts a statement wrapping a
/// variable_declaration, or a variable_declaration itself.
fn declarators_of<'t>(stmt: Node<'t>, src: &[u8]) -> Vec<(String, Option<Node<'t>>)> {
    let mut out = Vec::new();
    let decls: Vec<Node> = if stmt.kind() == "variable_declaration" {
        vec![stmt]
    } else {
        children(stmt)
            .into_iter()
            .filter(|c| c.kind() == "variable_declaration")
            .collect()
    };
    for c in decls {
        for d in children(c) {
            if d.kind() != "variable_declarator" {
                continue;
            }
            // The initializer is the named child that is
            // neither the name nor a bracketed argument list.
            let name_node = d.child_by_field_name("name");
            let value = (0..d.named_child_count())
                .filter_map(|i| d.named_child(i))
                .find(|ch| {
                    Some(*ch) != name_node
                        && !matches!(ch.kind(), "bracketed_argument_list" | "tuple_pattern")
                });
            if let Some(n) = name_node {
                out.push((txt(n, src), value));
            } else {
                // A deconstruction: every identifier in the
                // tuple pattern is a name.
                for ch in children(d) {
                    if ch.kind() == "tuple_pattern" {
                        let mut stack = vec![ch];
                        while let Some(x) = stack.pop() {
                            if x.kind() == "identifier" {
                                out.push((txt(x, src), value));
                                continue;
                            }
                            for g in children(x) {
                                stack.push(g);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Decision points in a body (the C# form of §3.2's
/// enumeration). Iterative (E-1); nested declarations pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "method_declaration"
                | "constructor_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "struct_declaration"
                | "record_declaration"
                | "enum_declaration"
        ) {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "for_statement" | "foreach_statement" | "while_statement"
            | "do_statement" | "conditional_expression" => 1,
            "switch_statement" => {
                let sections = count_kind_in(node, "switch_section");
                sections.saturating_sub(1)
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
// The defines/refs/writes scan (spec §3.6), C# form — the Java scan
// ---------------------------------------------------------------------------

/// Compute one method's (defines, refs, writes): the parameter
/// names, the body's own scan, the parameter types' names, minus
/// the defines from the refs.
fn fn_body_info(
    src: &[u8],
    params_list: Option<Node>,
    body: Option<Node>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
    if let Some(pl) = params_list {
        for p in children(pl) {
            if p.kind() != "parameter" {
                continue;
            }
            if let Some(n) = p.child_by_field_name("name") {
                defines.insert(txt(n, src));
            }
            if let Some(t) = p.child_by_field_name("type") {
                scan_type_names(t, src, &mut refs);
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

/// The reads of one expression (a top-level variable's
/// initializer refs).
fn body_refs(root: Node, src: &[u8]) -> Vec<String> {
    let mut defines = HashSet::new();
    let mut refs = HashSet::new();
    let mut writes = HashSet::new();
    scan_w(root, src, &mut defines, &mut refs, &mut writes);
    refs.into_iter().collect()
}

/// Collect the names a type node mentions as refs.
fn scan_type_names(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    if let Some(name) = type_name_of(node, src) {
        refs.insert(name);
        return;
    }
    for c in children(node) {
        scan_type_names(c, src, refs);
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
        MemberExpr(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::MemberExpr(m) => {
                // Only the expression of a member access is a read.
                if let Some(e) = m.child_by_field_name("expression") {
                    stack.push(S::N(e));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested declarations are separate symbols.
            "method_declaration" | "constructor_declaration" | "class_declaration"
            | "interface_declaration" | "struct_declaration" | "record_declaration"
            | "enum_declaration" | "local_function_statement" => {}
            "local_declaration_statement" | "variable_declaration" => {
                if let Some(t) = node.child_by_field_name("type") {
                    scan_type_names(t, src, refs);
                }
                for (name, value) in declarators_of(node, src) {
                    defines.insert(name.clone());
                    if let Some(v) = value {
                        writes.insert(name);
                        stack.push(S::N(v));
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
                        "member_access_expression" => {
                            if op == "=" {
                                if let Some(n) = l.child_by_field_name("name") {
                                    writes.insert(txt(n, src));
                                }
                            }
                            stack.push(S::MemberExpr(l));
                        }
                        _ => stack.push(S::N(l)),
                    }
                }
            }
            "postfix_unary_expression" | "prefix_unary_expression" => {
                for c in children(node) {
                    if c.kind() == "member_access_expression" {
                        stack.push(S::MemberExpr(c));
                    } else if c.kind() != "identifier" {
                        stack.push(S::N(c));
                    }
                }
            }
            "member_access_expression" => stack.push(S::MemberExpr(node)),
            "invocation_expression" => {
                // The invoked name is a ref (the Java rule);
                // the object and the arguments are reads.
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "identifier" => {
                            refs.insert(txt(f, src));
                        }
                        "member_access_expression" => {
                            if let Some(n) = f.child_by_field_name("name") {
                                refs.insert(txt(n, src));
                            }
                            stack.push(S::MemberExpr(f));
                        }
                        _ => stack.push(S::N(f)),
                    }
                }
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "object_creation_expression" => {
                if let Some(t) = node.child_by_field_name("type") {
                    scan_type_names(t, src, refs);
                }
                for c in children(node) {
                    if c.kind() != "type" {
                        stack.push(S::N(c));
                    }
                }
            }
            "cast_expression" => {
                for c in children(node) {
                    if let Some(t) = node.child_by_field_name("type") {
                        if c == t {
                            scan_type_names(c, src, refs);
                            continue;
                        }
                    }
                    stack.push(S::N(c));
                }
            }
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
    fn module_ids_follow_the_path_rule() {
        let f = one("Core/Engine.cs", "class Engine {}\n");
        assert_eq!(f.module_id, "quarry::Core::Engine");
    }

    #[test]
    fn namespaces_are_path_segments() {
        let f = one(
            "E.cs",
            "namespace Quarry.Core\n{\n    class Engine\n    {\n        public void Render() {}\n    }\n}\n",
        );
        let ids: Vec<&str> = f.symbols.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "quarry::E::Quarry::Core::Engine",
                "quarry::E::Quarry::Core::Engine::Render"
            ]
        );
    }

    #[test]
    fn file_scoped_namespace_prefixes() {
        let f = one(
            "E.cs",
            "namespace Quarry.Core;\nclass Engine { public void Render() {} }\n",
        );
        assert_eq!(f.symbols[0].id, "quarry::E::Quarry::Core::Engine");
    }

    #[test]
    fn heritage_first_is_inherits_rest_implements() {
        let f = one(
            "C.cs",
            "class Circle : Shape, IDrawable, IPoolable {}\ninterface I : IShape, IPoolable {}\n",
        );
        let rows: Vec<(&str, &str, EdgeKind)> = f
            .heritage
            .iter()
            .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Circle", "Shape", EdgeKind::Inherits),
                ("Circle", "IDrawable", EdgeKind::Implements),
                ("Circle", "IPoolable", EdgeKind::Implements),
                ("I", "IShape", EdgeKind::Inherits),
                ("I", "IPoolable", EdgeKind::Inherits),
            ]
        );
    }

    #[test]
    fn usings_internal_external_alias_static() {
        let f = one(
            "U.cs",
            "using System;\nusing Quarry.Shapes;\nusing Foo = Quarry.Core.Engine;\nusing static System.Math;\nnamespace Quarry { class U {} }\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("external::System", vec![]),
                ("quarry::Shapes", vec![]),
                ("quarry::Core::Engine", vec!["Foo".to_string()]),
                ("external::System", vec!["Math".to_string()]),
            ]
        );
    }

    #[test]
    fn scan_method_body() {
        let f = one(
            "E.cs",
            "class E\n{\n    public double Render(Circle wheel)\n    {\n        _renders += 1;\n        return wheel.Area();\n    }\n}\n",
        );
        let render = f.symbols.iter().find(|s| s.label == "Render").unwrap();
        assert_eq!(render.defines, vec!["wheel".to_string()]);
        assert_eq!(
            render.refs,
            vec![
                "Area".to_string(),
                "Circle".to_string(),
                "_renders".to_string(),
                "double".to_string()
            ]
        );
        assert!(render.writes.is_empty());
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("class D { int F(int x) { return ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1; } }\n");
        let f = one("deep.cs", &src);
        assert_eq!(f.symbols.len(), 2);
    }
}
