// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! PHP extractor (`.php`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, Java, Go, Shell, C++, Swift, Ruby, R,
//! Julia, and C#.
//!
//! The crate ships two grammars; this extractor uses
//! `LANGUAGE_PHP_ONLY` (pure PHP — the mixed grammar expects
//! HTML outside the `<?php` tags). Node kinds below were
//! verified against `tree-sitter-php-0.25.2/php_only/src/
//! node-types.json` in the crates.io registry cache and
//! against parse trees of the fixture (the pinned crate,
//! never guessed): `program`, `namespace_definition` (field
//! `name`; a `body` only in the braced form),
//! `namespace_use_declaration` (`namespace_use_clause` with a
//! `qualified_name`: `prefix` a `namespace_name` of `name`
//! segments), `class_declaration` / `interface_declaration` /
//! `trait_declaration` / `enum_declaration` (fields `name`,
//! `body`; `base_clause` and `class_interface_clause`
//! children carry heritage), `enum_case` (field `name`),
//! `method_declaration` (fields `name`, `parameters`,
//! `return_type`, `body`), `function_definition`,
//! `property_declaration`, `const_declaration`,
//! `use_declaration` (the in-class trait use),
//! `function_call_expression` (field `function`),
//! `member_call_expression` / `scoped_call_expression`
//! (fields `object`/`scope`, `name`, `arguments`),
//! `object_creation_expression`, `member_access_expression`
//! (fields `object`, `name`), `assignment_expression` and
//! `augmented_assignment_expression` (fields `left`,
//! `right`), `include_expression` / `require_expression` and
//! their `_once` forms, `variable_name`, `name`,
//! `qualified_name`, `simple_parameter` (fields `name`,
//! `type`, `default_value`).
//!
//! The mapping follows the Java extractor's spec shape with
//! the C++ namespace rule: module ids are path-derived;
//! namespace names are symbol-path segments (a bracketless
//! namespace prefixes the whole file). Symbols (§3.2 —
//! classes, traits, and enums are classes; interfaces are
//! interfaces; methods are methods and plain functions are
//! functions; enum cases are variables scoped under their
//! enum; top-level variables and constants are variables;
//! properties and class constants are fields, not symbols).
//! Variable names keep their `$` (the written form, as Ruby
//! keeps its sigils); `$this` is never a ref. Imports (`use`
//! declarations: internal when the first segment is the
//! file's own namespace root or matches the package name
//! case-insensitively — the root segment is read as the
//! package name — else `external::{first segment}`; the
//! imported name binds; plus `require`/`include` file
//! imports with a string-literal argument, resolved relative
//! to the including file like a C include). A `use` inside a
//! class body is a trait use — heritage (Implements) for the
//! enclosing class, not an import. Calls (§3.4 — for member
//! and scoped calls the method's name, the Python attribute
//! rule; for plain calls the function name as written;
//! object creation is a call on the type name). Heritage
//! (a class's `extends` is Inherits, its `implements` are
//! Implements; an interface's extends are Inherits). The
//! defines/refs/writes scan (§3.6's marimo discipline:
//! nested declarations pruned, reads that are also defines
//! dropped, augmented targets read not written, only the
//! object of a member access is a read, an invoked method's
//! name is a ref); decisions (§3.2's enumeration, PHP form:
//! `if` +1, each `elseif` +1, `for`/`foreach`/`while`/`do`
//! +1, `switch` contributes cases − 1, ternary +1, each
//! `&&`/`||`/`and`/`or` +1). The walk runs on an explicit
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
        .set_language(&tree_sitter_php::LANGUAGE_PHP_ONLY.into())
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;
    let module_id = module_id_for(package, rel_path);
    let root = tree.root_node();
    // A bracketless namespace prefixes the whole file, and the
    // file's namespace root anchors the use rule.
    let mut initial_path: Vec<String> = Vec::new();
    let mut ns_root: Option<String> = None;
    for c in children(root) {
        if c.kind() == "namespace_definition" {
            if let Some(n) = c.child_by_field_name("name") {
                let segs = namespace_segments(n, source.as_bytes());
                if c.child_by_field_name("body").is_none() {
                    initial_path = segs.clone();
                }
                if ns_root.is_none() {
                    ns_root = segs.first().cloned();
                }
            }
        }
    }
    let mut w = Walker {
        src: source.as_bytes(),
        pkg: package.to_string(),
        ns_root,
        module_id: module_id.clone(),
        rel_path: rel_path.to_path_buf(),
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

/// The segments of a namespace name (`Quarry\Shapes` → two
/// `name` children).
fn namespace_segments(node: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "name" {
            out.push(txt(n, src));
            continue;
        }
        for c in children(n) {
            stack.push(c);
        }
    }
    out.reverse();
    out
}

/// The segments of a qualified name: prefix segments plus the
/// final name.
fn qualified_segments(node: Node, src: &[u8]) -> Vec<String> {
    match node.kind() {
        "name" => vec![txt(node, src)],
        "qualified_name" | "relative_name" => {
            let mut out = Vec::new();
            if let Some(p) = node.child_by_field_name("prefix") {
                out.extend(namespace_segments(p, src));
            }
            for c in children(node) {
                if c.kind() == "name" {
                    out.push(txt(c, src));
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

struct Walker<'a> {
    src: &'a [u8],
    pkg: String,
    ns_root: Option<String>,
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
            "namespace_definition" if node.child_by_field_name("body").is_some() => {
                let names = node
                    .child_by_field_name("name")
                    .map(|n| namespace_segments(n, self.src))
                    .unwrap_or_default();
                stack.push(Step::LeaveNamespace);
                if let Some(b) = node.child_by_field_name("body") {
                    self.push_children(stack, b);
                }
                stack.push(Step::EnterNamespace { names });
            }
            "class_declaration" | "trait_declaration" | "enum_declaration" => {
                self.schedule_type(node, NodeKind::Class, stack)
            }
            "interface_declaration" => {
                self.schedule_type(node, NodeKind::Interface, stack)
            }
            "method_declaration" => self.schedule_callable(node, stack),
            "function_definition" => self.schedule_callable(node, stack),
            "enum_case" if self.in_class => {
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
            "const_declaration" if self.enclosing.is_empty() && !self.in_class => {
                // A top-level constant is a variable.
                for c in children(node) {
                    if c.kind() == "const_element" {
                        if let Some(n) = (0..c.named_child_count())
                            .filter_map(|i| c.named_child(i))
                            .find(|x| x.kind() == "name")
                        {
                            let name = self.text(n);
                            self.push_symbol(
                                NodeKind::Variable,
                                &[name],
                                c,
                                Vec::new(),
                                Vec::new(),
                                Vec::new(),
                                Some(end_line_of(c)),
                                None,
                                None,
                            );
                            self.pop_symbol();
                        }
                    }
                }
                self.push_children(stack, node);
            }
            "namespace_use_declaration" => self.handle_use_import(node),
            "use_declaration" if self.in_class => {
                // A trait use: heritage for the enclosing type.
                if let Some(type_name) = self.path.last().cloned() {
                    for target in use_name_targets(node, self.src) {
                        self.heritage.push(RawHeritage {
                            type_name: type_name.clone(),
                            target_name: target,
                            kind: EdgeKind::Implements,
                            line: line_of(node),
                        });
                    }
                }
                self.push_children(stack, node);
            }
            "include_expression" | "include_once_expression" | "require_expression"
            | "require_once_expression" => {
                self.handle_include_expr(node);
                self.push_children(stack, node);
            }
            "assignment_expression" if self.enclosing.is_empty() => {
                // A top-level $variable is a variable symbol.
                if let Some(left) = node.child_by_field_name("left") {
                    if left.kind() == "variable_name" {
                        let name = self.text(left);
                        let right = node.child_by_field_name("right");
                        let mut refs: Vec<String> =
                            right.map(|v| body_refs(v, self.src)).unwrap_or_default();
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
                            signature_of(node, right, self.src),
                            None,
                        );
                        self.pop_symbol();
                        if let Some(v) = right {
                            stack.push(Step::Enter(v));
                        }
                    } else {
                        self.push_children(stack, node);
                    }
                }
            }
            "function_call_expression" => {
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
                self.push_children(stack, node);
            }
            "member_call_expression" | "nullsafe_member_call_expression"
            | "scoped_call_expression" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let t = self.text(n).trim().to_string();
                    if !t.is_empty() {
                        self.calls.push(RawCall {
                            from_id: self.current_scope(),
                            to_name: t,
                            line: line_of(node),
                        });
                    }
                }
                self.push_children(stack, node);
            }
            "object_creation_expression" => {
                if let Some(first) = children(node).into_iter().next() {
                    let segs = qualified_segments(first, self.src);
                    let t = segs.last().cloned().unwrap_or_else(|| self.text(first));
                    if !t.is_empty() {
                        self.calls.push(RawCall {
                            from_id: self.current_scope(),
                            to_name: t,
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

    /// Schedule a class-like declaration: heritage from its
    /// base/interface clauses; the body walks under it.
    fn schedule_type<'t>(&mut self, node: Node<'t>, kind: NodeKind, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        for c in children(node) {
            match c.kind() {
                "base_clause" => {
                    for t in children(c) {
                        if let Some(target) = qualified_segments(t, self.src).last().cloned() {
                            self.heritage.push(RawHeritage {
                                type_name: name.clone(),
                                target_name: target,
                                kind: EdgeKind::Inherits,
                                line: line_of(c),
                            });
                        }
                    }
                }
                "class_interface_clause" => {
                    for t in children(c) {
                        if let Some(target) = qualified_segments(t, self.src).last().cloned() {
                            self.heritage.push(RawHeritage {
                                type_name: name.clone(),
                                target_name: target,
                                kind: EdgeKind::Implements,
                                line: line_of(c),
                            });
                        }
                    }
                }
                _ => {}
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

    /// Schedule a method or a plain function.
    fn schedule_callable<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let body = node.child_by_field_name("body");
        let (defines, mut refs, writes) = fn_body_info(
            self.src,
            node.child_by_field_name("parameters"),
            body,
        );
        if let Some(t) = node.child_by_field_name("return_type") {
            let mut set: HashSet<String> = refs.iter().cloned().collect();
            scan_type_names(t, self.src, &mut set);
            refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
            refs.sort();
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

    /// A namespace `use` declaration → RawImports (one per
    /// clause). Internal when the first segment is the file's
    /// namespace root or matches the package name
    /// case-insensitively; the root segment is read as the
    /// package name.
    fn handle_use_import(&mut self, node: Node) {
        for clause in children(node) {
            if clause.kind() != "namespace_use_clause" {
                continue;
            }
            let kids = children(clause);
            let Some(first) = kids.first() else { continue };
            let segs = qualified_segments(*first, self.src);
            if segs.is_empty() {
                continue;
            }
            // An alias is a trailing `name` child after the
            // qualified name.
            let alias = kids
                .iter()
                .skip(1)
                .find(|k| k.kind() == "name")
                .map(|k| self.text(*k));
            let bindings = vec![alias.unwrap_or_else(|| segs[segs.len() - 1].clone())];
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

    /// An include/require expression with a string-literal
    /// argument → a file import (the C include rule).
    fn handle_include_expr(&mut self, node: Node) {
        let Some(first) = children(node).into_iter().next() else {
            return;
        };
        if first.kind() != "string" {
            return;
        }
        let mut lit = String::new();
        for c in children(first) {
            if c.kind() != "string_content" {
                return;
            }
            lit.push_str(&self.text(c));
        }
        let to = self.resolve_include(&lit);
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings: Vec::new(),
            line: line_of(node),
        });
    }

    /// Resolve an included path against the including file's
    /// directory segments.
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

/// The trait names a class-body `use` declaration names (last
/// segments).
fn use_name_targets(node: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if matches!(n.kind(), "name" | "qualified_name" | "relative_name") {
            if let Some(t) = qualified_segments(n, src).last().cloned() {
                out.push(t);
            }
            continue;
        }
        for c in children(n) {
            stack.push(c);
        }
    }
    out
}

/// Decision points in a body (the PHP form of §3.2's
/// enumeration). Iterative (E-1); nested declarations pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "method_declaration"
                | "function_definition"
                | "class_declaration"
                | "interface_declaration"
                | "trait_declaration"
                | "enum_declaration"
        ) {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "else_if_clause" | "for_statement" | "foreach_statement"
            | "while_statement" | "do_statement" | "conditional_expression" => 1,
            "switch_statement" => {
                let cases = count_kind_in(node, "case_statement")
                    + count_kind_in(node, "default_statement");
                cases.saturating_sub(1)
            }
            "binary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| txt(o, src))
                    .unwrap_or_default();
                if matches!(op.as_str(), "&&" | "||" | "and" | "or") {
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
// The defines/refs/writes scan (spec §3.6), PHP form
// ---------------------------------------------------------------------------

/// Compute one callable's (defines, refs, writes): the
/// parameter names, the body's own scan, the parameter types'
/// names, minus the defines from the refs.
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
            if !matches!(
                p.kind(),
                "simple_parameter" | "property_promotion_parameter" | "variadic_parameter"
            ) {
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
    match node.kind() {
        "name" | "primitive_type" => {
            refs.insert(txt(node, src));
        }
        "qualified_name" | "relative_name" => {
            if let Some(t) = qualified_segments(node, src).last().cloned() {
                refs.insert(t);
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
        MemberObj(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::MemberObj(m) => {
                // Only the object of a member access is a read.
                if let Some(o) = m.child_by_field_name("object") {
                    stack.push(S::N(o));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested declarations are separate symbols.
            "method_declaration" | "function_definition" | "class_declaration"
            | "interface_declaration" | "trait_declaration" | "enum_declaration" => {}
            "assignment_expression" => {
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(S::N(r));
                }
                if let Some(l) = node.child_by_field_name("left") {
                    match l.kind() {
                        "variable_name" => {
                            let name = txt(l, src);
                            if name != "$this" {
                                defines.insert(name.clone());
                                writes.insert(name);
                            }
                        }
                        "member_access_expression" => {
                            if let Some(n) = l.child_by_field_name("name") {
                                writes.insert(txt(n, src));
                            }
                            stack.push(S::MemberObj(l));
                        }
                        _ => stack.push(S::N(l)),
                    }
                }
            }
            "augmented_assignment_expression" => {
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(S::N(r));
                }
                if let Some(l) = node.child_by_field_name("left") {
                    match l.kind() {
                        "variable_name" => {
                            let name = txt(l, src);
                            if name != "$this" {
                                refs.insert(name);
                            }
                        }
                        "member_access_expression" => stack.push(S::MemberObj(l)),
                        _ => stack.push(S::N(l)),
                    }
                }
            }
            "member_access_expression" => stack.push(S::MemberObj(node)),
            "member_call_expression" | "nullsafe_member_call_expression" => {
                // The invoked name is a ref (the Java rule);
                // the object and arguments are reads.
                if let Some(n) = node.child_by_field_name("name") {
                    refs.insert(txt(n, src));
                }
                stack.push(S::MemberObj(node));
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "scoped_call_expression" | "scoped_property_access_expression" => {
                if let Some(s) = node.child_by_field_name("scope") {
                    scan_type_names(s, src, refs);
                }
                if let Some(n) = node.child_by_field_name("name") {
                    if n.kind() == "name" {
                        refs.insert(txt(n, src));
                    }
                }
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "function_call_expression" => {
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "name" => {
                            refs.insert(txt(f, src));
                        }
                        "qualified_name" | "relative_name" => {
                            if let Some(t) = qualified_segments(f, src).last().cloned() {
                                refs.insert(t);
                            }
                        }
                        _ => stack.push(S::N(f)),
                    }
                }
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "object_creation_expression" => {
                let kids = children(node);
                if let Some(first) = kids.first() {
                    scan_type_names(*first, src, refs);
                }
                for c in kids.iter().skip(1) {
                    stack.push(S::N(*c));
                }
            }
            "foreach_statement" => {
                // The iterated expression is a read; the
                // variables after it are defines.
                let kids = children(node);
                let mut skipped_expr = false;
                for c in kids {
                    if !skipped_expr
                        && !matches!(c.kind(), "pair" | "variable_name" | "list_literal")
                    {
                        stack.push(S::N(c));
                        skipped_expr = true;
                        continue;
                    }
                    match c.kind() {
                        "variable_name" => {
                            defines.insert(txt(c, src));
                        }
                        "pair" => {
                            for part in children(c) {
                                if part.kind() == "variable_name" {
                                    defines.insert(txt(part, src));
                                } else {
                                    stack.push(S::N(part));
                                }
                            }
                        }
                        _ => stack.push(S::N(c)),
                    }
                }
            }
            "variable_name" => {
                let name = txt(node, src);
                if name != "$this" {
                    refs.insert(name);
                }
            }
            "name" => {
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
        let f = one("Core/Engine.php", "<?php\nclass Engine {}\n");
        assert_eq!(f.module_id, "quarry::Core::Engine");
    }

    #[test]
    fn heritage_extends_and_implements() {
        let f = one(
            "C.php",
            "<?php\nclass Circle extends Shape implements Drawable, Poolable {}\n",
        );
        let rows: Vec<(&str, EdgeKind)> = f
            .heritage
            .iter()
            .map(|h| (h.target_name.as_str(), h.kind))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Shape", EdgeKind::Inherits),
                ("Drawable", EdgeKind::Implements),
                ("Poolable", EdgeKind::Implements),
            ]
        );
    }

    #[test]
    fn scan_this_is_never_a_ref() {
        let f = one(
            "E.php",
            "<?php\nclass E {\n    private float $radius;\n    public function __construct(float $radius) {\n        $this->radius = $radius;\n    }\n}\n",
        );
        let ctor = f.symbols.iter().find(|s| s.label == "__construct").unwrap();
        assert_eq!(ctor.defines, vec!["$radius".to_string()]);
        assert_eq!(ctor.writes, vec!["radius".to_string()]);
        assert_eq!(ctor.refs, vec!["float".to_string()]);
    }

    #[test]
    fn trait_use_is_heritage_not_an_import() {
        let f = one(
            "T.php",
            "<?php\nclass Engine {\n    use LogsActivity;\n}\n",
        );
        assert!(f.imports.is_empty());
        assert_eq!(f.heritage.len(), 1);
        assert_eq!(f.heritage[0].type_name, "Engine");
        assert_eq!(f.heritage[0].target_name, "LogsActivity");
        assert_eq!(f.heritage[0].kind, EdgeKind::Implements);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        // 10,000 segments, not the family's 50,000: the PHP
        // grammar's own parser overflows its stack on a
        // 50,000-term chain before extraction begins (verified
        // against the pinned crate) — the walker's heap stack
        // handles everything the parser produces.
        let mut src = String::from("<?php\nfunction f($x) {\n    return ");
        for _ in 0..10_000 {
            src.push_str("$x + ");
        }
        src.push_str("1;\n}\n");
        let f = one("deep.php", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
