// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Scala extractor (`.scala`, `.sc`) — beside Rust,
//! TypeScript/JavaScript, Python, Kotlin, C, Java, Go, Shell,
//! C++, Swift, Ruby, R, Julia, C#, and PHP.
//!
//! Node kinds below were verified against
//! `tree-sitter-scala-0.26.13/src/node-types.json` in the
//! crates.io registry cache and against parse trees of the
//! fixture (the pinned crate, never guessed):
//! `compilation_unit`, `package_clause` (field `name`, a
//! `package_identifier` of segment identifiers),
//! `import_declaration` (repeated field `path`, one segment
//! identifier each; a trailing `namespace_selectors`,
//! `namespace_wildcard`, or `as_renamed_identifier` child),
//! `class_definition` / `trait_definition` /
//! `object_definition` / `enum_definition` (fields `name`,
//! `extend` — an `extends_clause` with repeated field
//! `type` — and `body`, a `template_body` or `enum_body`),
//! `simple_enum_case` (field `name`), `function_definition`
//! (fields `name`, `parameters`, `return_type`, `body`),
//! `function_declaration` (the same minus `body` — a trait's
//! abstract `def`), `type_definition` (fields `name`,
//! `type`), `val_definition` / `var_definition` (fields
//! `pattern`, `type`, `value`), `call_expression` (fields
//! `function`, `arguments`), `generic_function` (field
//! `function`), `field_expression` (fields `value`,
//! `field`), `instance_expression` (a type child, then field
//! `arguments` — a `new X(…)`), `infix_expression` (fields
//! `left`, `operator`, `right`), `assignment_expression`
//! (fields `left`, `right`), `parameter` (fields `name`,
//! `type`, `default_value`), `generic_type` (fields `type`,
//! `type_arguments`), `if_expression`, `for_expression`
//! (`enumerators` of `enumerator`), `while_expression`,
//! `do_while_expression`, `match_expression` (field `body`,
//! a `case_block` of `case_clause`), `try_expression`
//! (`catch_clause` children), `lambda_expression` (field
//! `parameters`).
//!
//! The mapping follows the C# extractor's spec shape (the
//! languages are close kin), with the field-expression scan
//! rule from Julia: module ids are path-derived (the shared
//! rule — the `package` clause does not enter the module id;
//! it anchors the import rule). Symbols (§3.2 — classes and
//! objects are classes; traits are interfaces; enums are
//! classes and their cases are variables scoped under the
//! enum; type definitions are classes, the Rust/TS alias
//! rule; `def`s are methods inside a type and functions
//! outside; top-level `val`/`var` definitions are variables;
//! class-level `val`/`var` definitions and constructor
//! parameters are fields, not symbols). Imports (an import
//! is internal when its first segment equals the file's
//! package root — the `package` clause's first segment, or
//! the package name: then its target is the imported name's
//! module-shaped path, the package plus the remaining
//! segments; anything else is `external::{first segment}`;
//! the imported name binds, a rename binds the alias, a
//! wildcard binds nothing; a selector list emits one import
//! per selector). Calls (§3.4 — the callee text as written;
//! `new X(…)` is a call on the type name; a parameterless
//! method access without parentheses is a field access, not
//! a call). Heritage (an `extends_clause`: for a class,
//! object, or enum the first type is Inherits and the rest
//! are Implements; for a trait all are Inherits; a
//! `derives` clause is typeclass derivation, not heritage,
//! and is not emitted). The defines/refs/writes scan
//! (§3.6's marimo discipline, Julia form: nested definitions
//! pruned, reads that are also defines dropped, only the
//! value of a field expression is a read, an infix compound
//! assignment's target is read not written, a plain
//! identifier callee is a ref while a field callee's name is
//! not, the type of a `new` expression is not a ref);
//! decisions (§3.2's enumeration, Scala form: `if` +1 — each
//! `else if` is a nested `if` — `for`/`while`/`do-while`
//! +1, `match` contributes case clauses − 1, each `catch`
//! clause +1, each `&&`/`||` infix operator +1). The walk
//! runs on an explicit heap stack (the E-1 lesson).
//!
//! Known partials, by design: `given` definitions and
//! extension methods are walked (their `def`s become
//! symbols) but are not themselves symbols; enum-case
//! `extends` clauses are not heritage (a case's parent is
//! its enum by construction); pattern bindings inside
//! `match` cases and `for` enumerators beyond the first
//! identifier are not tracked as defines; calls in
//! constructor arguments of an `extends` clause and in
//! parameter default values are not recorded (the C# rule:
//! only bodies are walked for calls).

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

/// All named children of `node` carrying the given field name
/// (tree-sitter fields repeat: an import's segments, an
/// extends clause's types, a curried def's parameter lists).
fn children_by_field<'a>(node: Node<'a>, field: &str) -> Vec<Node<'a>> {
    (0..node.named_child_count())
        .filter_map(|i| {
            node.named_child(i)
                .filter(|_| node.field_name_for_named_child(i as u32) == Some(field))
        })
        .collect()
}

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_scala::LANGUAGE.into())
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;
    let module_id = module_id_for(package, rel_path);
    let root = tree.root_node();
    // The file's package root anchors the import rule: the
    // first segment of the first `package` clause.
    let mut pkg_root: Option<String> = None;
    for c in children(root) {
        if c.kind() == "package_clause" {
            if let Some(n) = c.child_by_field_name("name") {
                pkg_root = children(n)
                    .into_iter()
                    .find(|s| s.kind() == "identifier")
                    .map(|s| txt(s, source.as_bytes()));
            }
            break;
        }
    }
    let mut w = Walker {
        src: source.as_bytes(),
        pkg: package.to_string(),
        pkg_root,
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
    /// The file's own package root segment (from its first
    /// `package` clause), anchoring the import rule.
    pkg_root: Option<String>,
    module_id: String,
    path: Vec<String>,
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
            "class_definition" | "object_definition" | "enum_definition" => {
                self.schedule_type(node, NodeKind::Class, stack)
            }
            "trait_definition" => self.schedule_type(node, NodeKind::Interface, stack),
            "simple_enum_case" | "parameterized_enum_case" if self.in_class => {
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
            "function_definition" | "function_declaration" => {
                self.schedule_function(node, stack)
            }
            "type_definition" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    let ty = node.child_by_field_name("type");
                    let mut refs: Vec<String> = ty
                        .map(|t| {
                            let mut set = HashSet::new();
                            scan_type_names(t, self.src, &mut set);
                            set.into_iter().collect::<Vec<String>>()
                        })
                        .unwrap_or_default();
                    refs.sort();
                    refs.dedup();
                    self.push_symbol(
                        NodeKind::Class,
                        &[name],
                        node,
                        Vec::new(),
                        refs,
                        Vec::new(),
                        Some(end_line_of(node)),
                        signature_of(node, ty, self.src),
                        None,
                    );
                    self.pop_symbol();
                } else {
                    self.push_children(stack, node);
                }
            }
            "val_definition" | "var_definition" if self.enclosing.is_empty() => {
                // A top-level statement variable.
                let pattern = node.child_by_field_name("pattern");
                let value = node.child_by_field_name("value");
                let names: Vec<String> = pattern
                    .map(|p| pattern_names(p, self.src))
                    .unwrap_or_default();
                let mut refs: Vec<String> =
                    value.map(|v| body_refs(v, self.src)).unwrap_or_default();
                if let Some(t) = node.child_by_field_name("type") {
                    let mut set: HashSet<String> = refs.iter().cloned().collect();
                    scan_type_names(t, self.src, &mut set);
                    refs = set.into_iter().collect();
                }
                refs.sort();
                refs.dedup();
                for name in names {
                    self.push_symbol(
                        NodeKind::Variable,
                        &[name],
                        node,
                        Vec::new(),
                        refs.clone(),
                        Vec::new(),
                        Some(end_line_of(node)),
                        signature_of(node, value, self.src),
                        None,
                    );
                    self.pop_symbol();
                }
                if let Some(v) = value {
                    stack.push(Step::Enter(v));
                }
            }
            "import_declaration" => self.handle_import(node),
            "call_expression" => {
                self.record_call(node);
                self.push_children(stack, node);
            }
            "instance_expression" => {
                // A `new X(…)` is a call on the type name.
                if let Some(t) = children(node)
                    .into_iter()
                    .find(|c| !matches!(c.kind(), "arguments" | "template_body"))
                {
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

    /// One `call_expression` → one RawCall; `to_name` is the
    /// callee's text as written (§3.4). A generic callee
    /// (`foo[Int](…)`) names its function part.
    fn record_call(&mut self, node: Node) {
        if let Some(f) = node.child_by_field_name("function") {
            let target = if f.kind() == "generic_function" {
                f.child_by_field_name("function").unwrap_or(f)
            } else {
                f
            };
            let t = self.text(target).trim().to_string();
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
    /// extends clause now; the body walks under the opened
    /// symbol.
    fn schedule_type<'t>(&mut self, node: Node<'t>, kind: NodeKind, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        // Heritage (§3.3): for a class, object, or enum the
        // first extended type is Inherits and the rest are
        // Implements; for a trait (Interface kind) all are
        // Inherits.
        if let Some(ext) = node.child_by_field_name("extend") {
            for (i, t) in children_by_field(ext, "type").into_iter().enumerate() {
                if let Some(target) = type_name_of(t, self.src) {
                    let edge = if kind == NodeKind::Interface || i == 0 {
                        EdgeKind::Inherits
                    } else {
                        EdgeKind::Implements
                    };
                    self.heritage.push(RawHeritage {
                        type_name: name.clone(),
                        target_name: target,
                        kind: edge,
                        line: line_of(ext),
                    });
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

    /// Schedule a `def` — a `function_definition` (with body)
    /// or a `function_declaration` (a trait's abstract def,
    /// no body: decisions stay `None`).
    fn schedule_function<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let param_lists = children_by_field(node, "parameters");
        let return_type = node.child_by_field_name("return_type");
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(self.src, &param_lists, return_type, body);
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
            stack.push(Step::Enter(b));
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

    /// An `import` declaration → one RawImport per imported
    /// name (a selector list emits one per selector).
    fn handle_import(&mut self, node: Node) {
        let line = line_of(node);
        let mut segs: Vec<String> = Vec::new();
        for p in children_by_field(node, "path") {
            // A path child is normally one segment identifier;
            // a stable identifier's text splits on its dots.
            for s in self.text(p).split('.') {
                if !s.is_empty() {
                    segs.push(s.to_string());
                }
            }
        }
        let kids = children(node);
        let selectors = kids.iter().find(|c| c.kind() == "namespace_selectors");
        let has_wildcard = kids
            .iter()
            .any(|c| matches!(c.kind(), "namespace_wildcard" | "wildcard"));
        let renamed = kids.iter().find(|c| {
            matches!(c.kind(), "as_renamed_identifier" | "arrow_renamed_identifier")
        });
        if let Some(sel) = selectors {
            for c in children(*sel) {
                match c.kind() {
                    "identifier" | "type_identifier" => {
                        let orig = self.text(c);
                        let mut full = segs.clone();
                        full.push(orig.clone());
                        self.push_import(full, vec![orig], line);
                    }
                    "as_renamed_identifier" | "arrow_renamed_identifier" => {
                        if let (Some(n), Some(a)) = (
                            c.child_by_field_name("name"),
                            c.child_by_field_name("alias"),
                        ) {
                            let mut full = segs.clone();
                            full.push(self.text(n));
                            self.push_import(full, vec![self.text(a)], line);
                        }
                    }
                    "namespace_wildcard" | "wildcard" => {
                        self.push_import(segs.clone(), Vec::new(), line)
                    }
                    _ => {}
                }
            }
        } else if has_wildcard {
            self.push_import(segs, Vec::new(), line);
        } else if let Some(r) = renamed {
            if let (Some(n), Some(a)) = (
                r.child_by_field_name("name"),
                r.child_by_field_name("alias"),
            ) {
                let mut full = segs.clone();
                full.push(self.text(n));
                self.push_import(full, vec![self.text(a)], line);
            }
        } else if !segs.is_empty() {
            let binding = segs.last().cloned().unwrap_or_default();
            self.push_import(segs, vec![binding], line);
        }
    }

    /// Resolve one imported name to its target module id:
    /// internal when the first segment is the file's package
    /// root (or the package name) — the target is the package
    /// plus the remaining segments — else external.
    fn push_import(&mut self, segs: Vec<String>, bindings: Vec<String>, line: u32) {
        if segs.is_empty() {
            return;
        }
        let internal = self.pkg_root.as_deref() == Some(segs[0].as_str())
            || segs[0] == self.pkg;
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
            line,
        });
    }
}

/// The name a type node denotes: the last segment.
fn type_name_of(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" | "identifier" => Some(txt(node, src)),
        "generic_type" | "annotated_type" => node
            .child_by_field_name("type")
            .or_else(|| children(node).into_iter().next())
            .and_then(|c| type_name_of(c, src)),
        "stable_type_identifier" => {
            // The last identifier segment in reading order.
            let mut last: Option<String> = None;
            let mut stack = vec![node];
            while let Some(n) = stack.pop() {
                if n.kind() == "identifier" || n.kind() == "type_identifier" {
                    last = Some(txt(n, src));
                    continue;
                }
                for c in children(n).into_iter().rev() {
                    stack.push(c);
                }
            }
            last
        }
        "projected_type" => node
            .child_by_field_name("selector")
            .and_then(|c| type_name_of(c, src)),
        "compound_type" => node
            .child_by_field_name("base")
            .and_then(|c| type_name_of(c, src)),
        "singleton_type" => children(node)
            .into_iter()
            .next()
            .and_then(|c| type_name_of(c, src)),
        _ => None,
    }
}

/// The names a `val`/`var` pattern binds, in reading order:
/// a plain identifier, or every identifier in a pattern.
fn pattern_names(pattern: Node, src: &[u8]) -> Vec<String> {
    if pattern.kind() == "identifier" {
        return vec![txt(pattern, src)];
    }
    let mut out: Vec<String> = Vec::new();
    let mut stack = vec![pattern];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" {
            out.push(txt(n, src));
            continue;
        }
        for c in children(n).into_iter().rev() {
            stack.push(c);
        }
    }
    out.reverse();
    out
}

/// Decision points in a body (the Scala form of §3.2's
/// enumeration). Iterative (E-1); nested definitions pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "function_definition"
                | "function_declaration"
                | "class_definition"
                | "trait_definition"
                | "object_definition"
                | "enum_definition"
                | "type_definition"
                | "given_definition"
        ) {
            continue;
        }
        n += match node.kind() {
            "if_expression" | "for_expression" | "while_expression"
            | "do_while_expression" => 1,
            "match_expression" => {
                // Case clauses − 1 (the k-way branch rule).
                let cases = node
                    .child_by_field_name("body")
                    .map(|b| {
                        children(b)
                            .into_iter()
                            .filter(|c| c.kind() == "case_clause")
                            .count() as u32
                    })
                    .unwrap_or(0);
                cases.saturating_sub(1)
            }
            "catch_clause" => 1,
            "infix_expression" => {
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
// The defines/refs/writes scan (spec §3.6), Scala form
// ---------------------------------------------------------------------------

/// Compute one def's (defines, refs, writes): the parameter
/// names and their types' names, the return type's names,
/// the body's own scan, minus the defines from the refs.
fn fn_body_info(
    src: &[u8],
    param_lists: &[Node],
    return_type: Option<Node>,
    body: Option<Node>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
    for pl in param_lists {
        for p in children(*pl) {
            collect_param(p, src, &mut defines, &mut refs, &mut writes);
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

/// One parameter: a plain identifier binds; a `parameter`
/// binds its name, reads its type, and scans its default.
fn collect_param(
    p: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
    writes: &mut HashSet<String>,
) {
    match p.kind() {
        "identifier" => {
            defines.insert(txt(p, src));
        }
        "parameter" => {
            if let Some(n) = p.child_by_field_name("name") {
                for name in pattern_names(n, src) {
                    defines.insert(name);
                }
            }
            if let Some(t) = p.child_by_field_name("type") {
                scan_type_names(t, src, refs);
            }
            if let Some(d) = p.child_by_field_name("default_value") {
                scan_w(d, src, defines, refs, writes);
            }
        }
        _ => {}
    }
}

/// Collect the names a type expression mentions as refs:
/// every `type_identifier`, and the last segment of a stable
/// (qualified) type identifier.
fn scan_type_names(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "type_identifier" => {
                refs.insert(txt(n, src));
            }
            "stable_type_identifier" => {
                if let Some(name) = type_name_of(n, src) {
                    refs.insert(name);
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

/// The reads of one expression (a top-level variable's
/// initializer refs).
fn body_refs(root: Node, src: &[u8]) -> Vec<String> {
    let mut defines = HashSet::new();
    let mut refs = HashSet::new();
    let mut writes = HashSet::new();
    scan_w(root, src, &mut defines, &mut refs, &mut writes);
    refs.into_iter().collect()
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
        FieldValue(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::FieldValue(f) => {
                // Only the value of a field expression is a read.
                if let Some(v) = f.child_by_field_name("value") {
                    stack.push(S::N(v));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested definitions are their own symbols.
            "function_definition" | "function_declaration" | "class_definition"
            | "trait_definition" | "object_definition" | "enum_definition"
            | "type_definition" | "given_definition" => {}
            "val_definition" | "var_definition" => {
                if let Some(t) = node.child_by_field_name("type") {
                    scan_type_names(t, src, refs);
                }
                let mut names: Vec<String> = Vec::new();
                if let Some(p) = node.child_by_field_name("pattern") {
                    names = pattern_names(p, src);
                }
                for name in &names {
                    defines.insert(name.clone());
                }
                if let Some(v) = node.child_by_field_name("value") {
                    for name in &names {
                        writes.insert(name.clone());
                    }
                    stack.push(S::N(v));
                }
            }
            "assignment_expression" => {
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(S::N(r));
                }
                if let Some(l) = node.child_by_field_name("left") {
                    match l.kind() {
                        "identifier" => {
                            let name = txt(l, src);
                            defines.insert(name.clone());
                            writes.insert(name);
                        }
                        "field_expression" => stack.push(S::FieldValue(l)),
                        _ => stack.push(S::N(l)),
                    }
                }
            }
            "infix_expression" => {
                // A compound assignment's target (`x += 1`,
                // an infix expression in this grammar) is read,
                // not written — the generic descent below reads
                // both operands, which is exactly that.
                for c in children(node) {
                    if c.kind() != "operator_identifier" {
                        stack.push(S::N(c));
                    }
                }
            }
            "field_expression" => stack.push(S::FieldValue(node)),
            "call_expression" => {
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "field_expression" => stack.push(S::FieldValue(f)),
                        "generic_function" => {
                            if let Some(inner) = f.child_by_field_name("function") {
                                if inner.kind() == "field_expression" {
                                    stack.push(S::FieldValue(inner));
                                } else {
                                    stack.push(S::N(inner));
                                }
                            }
                        }
                        _ => stack.push(S::N(f)),
                    }
                }
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "instance_expression" => {
                // The constructed type is not a ref; its
                // arguments (and any template body) are reads.
                for c in children(node) {
                    if matches!(c.kind(), "arguments" | "template_body") {
                        stack.push(S::N(c));
                    }
                }
            }
            "lambda_expression" => {
                let kids = children(node);
                for (i, c) in kids.iter().enumerate() {
                    if i + 1 == kids.len() {
                        // The body is the last child.
                        stack.push(S::N(*c));
                    } else {
                        collect_param(*c, src, defines, refs, writes);
                        if c.kind() == "parameters" {
                            for p in children(*c) {
                                collect_param(p, src, defines, refs, writes);
                            }
                        }
                    }
                }
            }
            "enumerator" => {
                let kids = children(node);
                for (i, c) in kids.iter().enumerate() {
                    if i == 0 && c.kind() == "identifier" {
                        defines.insert(txt(*c, src));
                    } else {
                        stack.push(S::N(*c));
                    }
                }
            }
            "case_clause" => {
                // The pattern binds, it does not read (a typed
                // pattern's type is a ref); the guard and the
                // body are reads.
                if let Some(p) = node.child_by_field_name("pattern") {
                    if p.kind() == "typed_pattern" {
                        if let Some(t) = p.child_by_field_name("type") {
                            scan_type_names(t, src, refs);
                        }
                    }
                }
                for c in children(node) {
                    stack.push(S::N(c));
                }
                if let Some(b) = node.child_by_field_name("body") {
                    stack.push(S::N(b));
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
        let f = one("Core/Engine.scala", "package quarry.core\nclass Engine\n");
        assert_eq!(f.module_id, "quarry::Core::Engine");
    }

    #[test]
    fn trait_and_extends_heritage() {
        let f = one(
            "S.scala",
            "package quarry.shapes\ntrait Shape { def area: Double }\nclass Circle(val radius: Double) extends Shape { def area: Double = radius }\n",
        );
        assert_eq!(f.heritage.len(), 1);
        assert_eq!(f.heritage[0].type_name, "Circle");
        assert_eq!(f.heritage[0].target_name, "Shape");
        assert_eq!(f.heritage[0].kind, EdgeKind::Inherits);
        let kinds: Vec<(&str, NodeKind)> =
            f.symbols.iter().map(|s| (s.label.as_str(), s.kind)).collect();
        assert!(kinds.contains(&("Shape", NodeKind::Interface)));
        assert!(kinds.contains(&("Circle", NodeKind::Class)));
        let abs = f.symbols.iter().find(|s| s.id.ends_with("Shape::area")).unwrap();
        assert_eq!(abs.kind, NodeKind::Method);
        assert_eq!(abs.decisions, None);
        assert_eq!(abs.refs, vec!["Double".to_string()]);
    }

    #[test]
    fn imports_internal_external_selectors_wildcard() {
        let f = one(
            "M.scala",
            "package quarry\nimport quarry.shapes.Circle\nimport quarry.shapes.{Square, Triangle as Tri}\nimport quarry.util._\nimport scala.collection.mutable.ArrayBuffer\nobject M\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("quarry::shapes::Circle", vec!["Circle".to_string()]),
                ("quarry::shapes::Square", vec!["Square".to_string()]),
                ("quarry::shapes::Triangle", vec!["Tri".to_string()]),
                ("quarry::util", vec![]),
                ("external::scala", vec!["ArrayBuffer".to_string()]),
            ]
        );
    }

    #[test]
    fn scan_method_body() {
        let f = one(
            "E.scala",
            "package quarry.core\nclass Engine {\n  private var renders = 0\n  def render(wheel: Circle): Double = {\n    renders += 1\n    wheel.area()\n  }\n}\n",
        );
        let render = f.symbols.iter().find(|s| s.label == "render").unwrap();
        assert_eq!(render.defines, vec!["wheel".to_string()]);
        assert_eq!(
            render.refs,
            vec!["Circle".to_string(), "Double".to_string(), "renders".to_string()]
        );
        assert!(render.writes.is_empty());
        assert_eq!(render.decisions, Some(0));
    }

    #[test]
    fn object_and_new_calls() {
        let f = one(
            "Main.scala",
            "package quarry\nobject Main {\n  def main(args: Array[String]): Unit = {\n    val engine = new Engine()\n    println(engine.render(engine))\n  }\n}\n",
        );
        let main = f.symbols.iter().find(|s| s.label == "main").unwrap();
        assert_eq!(main.kind, NodeKind::Method);
        assert_eq!(main.defines, vec!["args".to_string(), "engine".to_string()]);
        assert_eq!(main.writes, vec!["engine".to_string()]);
        assert_eq!(
            main.refs,
            vec![
                "Array".to_string(),
                "String".to_string(),
                "Unit".to_string(),
                "println".to_string()
            ]
        );
        let calls: Vec<&str> = f.calls.iter().map(|c| c.to_name.as_str()).collect();
        assert_eq!(calls, vec!["Engine", "println", "engine.render"]);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        // The full 50,000 survives in-process (see the
        // deep-chain test in tests/known_answer_scala.rs).
        let mut src = String::from("package quarry\nobject Deep {\n  def chain: String = ");
        for _ in 0..50_000 {
            src.push_str("\"a\" + ");
        }
        src.push_str("\"z\"\n}\n");
        let f = one("deep.scala", &src);
        assert!(f.symbols.iter().any(|s| s.label == "chain"));
    }
}
