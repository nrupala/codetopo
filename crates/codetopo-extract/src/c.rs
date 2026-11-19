// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! C extractor (`.c`, `.h`) — beside Rust, TypeScript/JavaScript,
//! Python, and Kotlin.
//!
//! Node kinds below were verified against
//! `tree-sitter-c-0.24.2/src/node-types.json` in the crates.io
//! registry cache (the pinned crate, never guessed):
//! `translation_unit`, `function_definition` (fields `body`,
//! `declarator`, `type`), `function_declarator` (fields
//! `declarator`, `parameters`), `declaration` (fields `declarator`,
//! `type`), `init_declarator` (fields `declarator`, `value`),
//! `struct_specifier` / `union_specifier` / `enum_specifier`
//! (fields `name`, `body`), `enumerator` (field `name`),
//! `type_definition` (fields `declarator`, `type`),
//! `preproc_include` (field `path`: `string_literal` or
//! `system_lib_string`), `preproc_def` (fields `name`, `value`),
//! `preproc_function_def` (fields `name`, `parameters`, `value`),
//! `call_expression` (field `function`), `assignment_expression`
//! (fields `left`, `operator`, `right`), `field_expression`
//! (fields `argument`, `field`), `parameter_declaration` (fields
//! `declarator`, `type`).
//!
//! The mapping follows the Python extractor's spec shape: module
//! ids are path-derived (§3.1's rule — a header and its
//! implementation share a stem, hence a module); symbols (§3.2 —
//! function definitions AND declarations (prototypes) are
//! functions, struct/union/enum definitions and typedef names are
//! classes, enumerators are variables scoped under their enum,
//! file-scope variables are variables, struct fields are not
//! symbols, function-like macros are functions, object-like macros
//! with a value are variables — a value-less `#define` is an
//! include-guard-style flag and emits nothing); imports
//! (`#include` — quoted paths resolve on module-id segments
//! relative to the including file, system includes are
//! `external::{first segment}`; C includes bind no names, so
//! bindings are always empty); calls (§3.4 — the callee text as
//! written); heritage (none — C has no inheritance syntax; the
//! section is empty by construction, not by omission); the
//! defines/refs/writes scan (§3.6's marimo discipline: nested
//! declarations pruned, reads that are also defines dropped,
//! augmented-assignment targets read not written, only the
//! argument of a field expression is a read); decisions (§3.2's
//! enumeration, C form: `if` +1, `for`/`while`/`do` +1, `switch`
//! contributes cases − 1, ternary +1, each `&&`/`||` +1). The
//! walk runs on an explicit heap stack (the E-1 lesson).

use std::collections::HashSet;
use std::path::Path;

use codetopo_core::emit::{FileSymbols, RawCall, RawImport, RawSymbol};
use codetopo_core::schema::NodeKind;

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
        .set_language(&tree_sitter_c::LANGUAGE.into())
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
        enclosing: Vec::new(),
        in_class: false,
        symbols: Vec::new(),
        imports: Vec::new(),
        calls: Vec::new(),
    };
    w.walk(tree.root_node());
    Ok(FileSymbols {
        package: package.to_string(),
        file: display_path(rel_path),
        module_id,
        symbols: w.symbols,
        imports: w.imports,
        calls: w.calls,
        heritage: Vec::new(),
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
    enclosing: Vec<String>,
    in_class: bool,
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
    calls: Vec<RawCall>,
}

/// One step of the walker's explicit work stack (the E-1 lesson:
/// recursion over the syntax tree overflowed on generated
/// deep-chain files; the walk replays the recursive order here).
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
            "function_definition" => self.schedule_function_def(node, stack),
            // A bare specifier at file scope (`enum Color { … };`)
            // is a translation-unit child in its own right, not
            // wrapped in a declaration node.
            "struct_specifier" | "union_specifier" | "enum_specifier"
                if self.enclosing.is_empty()
                    && node.child_by_field_name("body").is_some() =>
            {
                self.schedule_specifier_class(node, node, stack)
            }
            "declaration" if self.enclosing.is_empty() => {
                self.handle_file_declaration(node, stack)
            }
            "type_definition" if self.enclosing.is_empty() => {
                self.handle_typedef(node, stack)
            }
            "enumerator" if self.in_class => {
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
            "preproc_include" => self.handle_include(node),
            "preproc_def" if self.enclosing.is_empty() => {
                // An object-like macro with a value is a variable;
                // a value-less define (include guards) emits nothing.
                if node.child_by_field_name("value").is_some() {
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
                }
            }
            "preproc_function_def" if self.enclosing.is_empty() => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    self.push_symbol(
                        NodeKind::Function,
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

    /// Schedule a function definition: scan computed now; the
    /// stack replays — the symbol opens, the body walks, the scope
    /// closes. (C has no parameter defaults, so nothing walks in
    /// the enclosing scope.)
    fn schedule_function_def<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(decl) = node.child_by_field_name("declarator") else {
            self.push_children(stack, node);
            return;
        };
        let Some(name) = declarator_name(decl, self.src) else {
            self.push_children(stack, node);
            return;
        };
        let params = find_params(decl);
        let body = node.child_by_field_name("body");
        let (defines, mut refs, writes) = fn_body_info(self.src, params, body);
        // The return type's names are refs (the D5 rule).
        if let Some(t) = node.child_by_field_name("type") {
            let mut set: HashSet<String> = refs.iter().cloned().collect();
            scan_type_names(t, self.src, &mut set);
            refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
            refs.sort();
        }
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

    /// A file-scope `declaration`: any combination of a
    /// struct/union/enum definition (a class), function prototypes
    /// (functions), and variables. Each declarator is classified
    /// by its shape.
    fn handle_file_declaration<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let ty = node.child_by_field_name("type");
        for decl in declarator_children(node) {
            match decl.kind() {
                "function_declarator" => {
                    if let Some(name) = declarator_name(decl, self.src) {
                        let (defines, mut refs, writes) =
                            fn_body_info(self.src, find_params(decl), None);
                        if let Some(t) = ty {
                            let mut set: HashSet<String> = refs.iter().cloned().collect();
                            scan_type_names(t, self.src, &mut set);
                            refs = set
                                .into_iter()
                                .filter(|r| !defines.contains(r))
                                .collect();
                            refs.sort();
                        }
                        self.push_symbol(
                            NodeKind::Function,
                            &name,
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
                _ => {
                    if let Some(name) = declarator_name(decl, self.src) {
                        // The initializer's reads are the variable's
                        // refs; its calls belong to the module and
                        // are walked below.
                        let init = init_value(decl);
                        let mut refs: Vec<String> =
                            init.map(|v| body_refs(v, self.src)).unwrap_or_default();
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
                            signature_of(node, init, self.src),
                            None,
                        );
                        self.pop_symbol();
                        if let Some(v) = init {
                            stack.push(Step::Enter(v));
                        }
                    }
                }
            }
        }
        // A specifier with a body defines a type — schedule it as
        // a class so its enumerators nest under it. Scheduled last
        // so the declaration's own symbols stay in source order
        // ahead of it in the emission list.
        if let Some(spec) = specifier_with_body(node) {
            self.schedule_specifier_class(spec, node, stack);
        }
    }

    /// `typedef … Name;` — the alias name is a class (the Python
    /// PEP 695 precedent: a type alias is a class-kind symbol).
    /// When the underlying specifier has a body, the class opens
    /// so enumerators nest; a specifier whose own name equals the
    /// alias emits once, under the alias.
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
            // Schedule the specifier as this typedef's class: emit
            // the class here (under the typedef name) and walk the
            // body with the scope open.
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
            return;
        }
        self.push_symbol(
            NodeKind::Class,
            &name,
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

    /// Schedule a struct/union/enum definition found in a
    /// declaration as a class symbol, opening its scope so enum
    /// members nest under it.
    fn schedule_specifier_class<'t>(
        &mut self,
        spec: Node<'t>,
        decl_node: Node<'t>,
        stack: &mut Vec<Step<'t>>,
    ) {
        let Some(name_node) = spec.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
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
            name,
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

    /// `#include` → one RawImport. Quoted paths resolve on
    /// module-id segments relative to the including file's
    /// directory (`..` ascends; ascending past the root is
    /// `external::unresolved`). System includes are
    /// `external::{first segment, extension stripped}`. Bindings
    /// are always empty — a C include binds no names.
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

    /// Resolve a quoted include path against the including file's
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

/// The declarator children of a declaration: the `declarator`
/// field may repeat (`int a, b;`), and an init_declarator wraps
/// its own declarator.
fn declarator_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    let mut cursor = node.walk();
    for c in node.children_by_field_name("declarator", &mut cursor) {
        out.push(c);
    }
    out
}

/// The name a declarator binds, unwrapping pointer / array /
/// function / parenthesized / init declarators to the identifier
/// (or type_identifier, for typedef names) at the core.
fn declarator_name(node: Node, src: &[u8]) -> Option<String> {
    let mut cur = node;
    loop {
        match cur.kind() {
            "identifier" | "type_identifier" | "field_identifier" => {
                return Some(txt(cur, src));
            }
            "init_declarator" | "pointer_declarator" | "array_declarator"
            | "function_declarator" | "parenthesized_declarator" | "attributed_declarator" => {
                cur = cur.child_by_field_name("declarator")?;
            }
            _ => return None,
        }
    }
}

/// The parameter list of a function declarator (unwrapping
/// pointer declarators to reach it).
fn find_params(node: Node) -> Option<Node> {
    let mut cur = node;
    loop {
        if cur.kind() == "function_declarator" {
            return cur.child_by_field_name("parameters");
        }
        cur = cur.child_by_field_name("declarator")?;
    }
}

/// The initializer value of a declarator, if it is an init_declarator.
fn init_value(node: Node) -> Option<Node> {
    if node.kind() == "init_declarator" {
        node.child_by_field_name("value")
    } else {
        None
    }
}

/// The struct/union/enum specifier with a body among a
/// declaration's type field (or nested in a sized/parenthesized
/// type), if any.
fn specifier_with_body<'t>(node: Node<'t>) -> Option<Node<'t>> {
    let ty = node.child_by_field_name("type")?;
    let mut stack = vec![ty];
    while let Some(n) = stack.pop() {
        if matches!(n.kind(), "struct_specifier" | "union_specifier" | "enum_specifier")
            && n.child_by_field_name("body").is_some()
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

/// Decision points in a body (the C form of §3.2's enumeration).
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
            | "conditional_expression" => 1,
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
// The defines/refs/writes scan (spec §3.6), C form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): the parameter
/// names, the body's own scan, the parameter types' names, minus
/// the defines from the refs. A prototype (no body) scans its
/// parameters only.
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
            // A specifier's body holds field declarations, not
            // reads; its name (collected above) is the read.
            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                if let Some(name) = n.child_by_field_name("name") {
                    refs.insert(txt(name, src));
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
                // Only the argument of a field expression is a read.
                if let Some(a) = f.child_by_field_name("argument") {
                    stack.push(S::N(a));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested function definitions are separate symbols.
            "function_definition" => {}
            // A type specifier's names are reads; its body is not.
            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                scan_type_names(node, src, refs);
            }
            "declaration" => {
                if let Some(t) = node.child_by_field_name("type") {
                    scan_type_names(t, src, refs);
                }
                for decl in declarator_children(node) {
                    if decl.kind() == "function_declarator" {
                        // A local prototype declares the name.
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
                                // Augmented: read, not written.
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
                        // Pointer/subscript targets are reads of
                        // the base, never defines.
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

    fn symbols(
        files: Vec<(&str, &str)>,
    ) -> Vec<codetopo_core::emit::FileSymbols> {
        let dir = std::env::temp_dir().join(format!(
            "codetopo-c-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for (rel, src) in &files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, src).unwrap();
        }
        let out: Vec<FileSymbols> = files
            .iter()
            .map(|(rel, src)| extract("t", &PathBuf::from(rel), src).unwrap())
            .collect();
        std::fs::remove_dir_all(&dir).ok();
        out
    }

    fn one(rel: &str, src: &str) -> FileSymbols {
        symbols(vec![(rel, src)]).into_iter().next().unwrap()
    }

    #[test]
    fn module_ids_follow_the_path_rule() {
        let f = one("core/engine.c", "int x;\n");
        assert_eq!(f.module_id, "t::core::engine");
        // A header and its implementation share a module.
        let h = one("core/engine.h", "int x;\n");
        assert_eq!(h.module_id, "t::core::engine");
        let root = one("main.c", "int x;\n");
        assert_eq!(root.module_id, "t::main");
    }

    #[test]
    fn struct_fields_are_not_symbols_but_enumerators_are() {
        let f = one(
            "s.c",
            "enum Color { RED, GREEN = 4 };\nstruct Pair { int left; int right; };\n",
        );
        let names: Vec<(&str, &str)> = f
            .symbols
            .iter()
            .map(|s| (s.id.rsplit("::").next().unwrap(), match s.kind {
                NodeKind::Class => "class",
                NodeKind::Variable => "var",
                _ => "?",
            }))
            .collect();
        assert!(names.contains(&("Color", "class")), "{names:?}");
        assert!(names.contains(&("RED", "var")), "{names:?}");
        assert!(names.contains(&("GREEN", "var")), "{names:?}");
        assert!(names.contains(&("Pair", "class")), "{names:?}");
        assert!(!names.iter().any(|(n, _)| *n == "left"), "{names:?}");
        let green = f.symbols.iter().find(|s| s.label == "GREEN").unwrap();
        assert_eq!(green.id, "t::s::Color::GREEN");
    }

    #[test]
    fn typedef_of_struct_is_one_class_under_the_alias() {
        let f = one(
            "s.c",
            "typedef struct { double x; } Point;\ntypedef int Count;\n",
        );
        let classes: Vec<&str> = f
            .symbols
            .iter()
            .filter(|s| s.kind == NodeKind::Class)
            .map(|s| s.label.as_str())
            .collect();
        assert_eq!(classes, vec!["Point", "Count"]);
    }

    #[test]
    fn includes_resolve_relative_and_system() {
        let f = one(
            "core/engine.c",
            "#include \"engine.h\"\n#include \"../shapes/circle.h\"\n#include <sys/types.h>\n#include <math.h>\nint x;\n",
        );
        let tos: Vec<&str> = f.imports.iter().map(|i| i.to_module_id.as_str()).collect();
        assert_eq!(
            tos,
            vec![
                "t::core::engine",
                "t::shapes::circle",
                "external::sys",
                "external::math"
            ]
        );
        assert!(f.imports.iter().all(|i| i.bindings.is_empty()));
    }

    #[test]
    fn macros_functions_and_valued_defines_only() {
        let f = one(
            "m.c",
            "#define LIMIT 10\n#define GUARD\n#define SQ(x) ((x) * (x))\nint y;\n",
        );
        let names: Vec<(&str, NodeKind)> =
            f.symbols.iter().map(|s| (s.label.as_str(), s.kind)).collect();
        assert!(names.contains(&("LIMIT", NodeKind::Variable)), "{names:?}");
        assert!(names.contains(&("SQ", NodeKind::Function)), "{names:?}");
        assert!(!names.iter().any(|(n, _)| *n == "GUARD"), "{names:?}");
    }

    #[test]
    fn prototypes_and_definitions_are_functions() {
        let f = one(
            "p.c",
            "int add(int a, int b);\nint add(int a, int b) { return a + b; }\n",
        );
        let fns: Vec<&RawSymbol> =
            f.symbols.iter().filter(|s| s.kind == NodeKind::Function).collect();
        assert_eq!(fns.len(), 2);
        let def = fns[1];
        assert_eq!(def.defines, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(def.refs, vec!["int".to_string()]);
    }

    #[test]
    fn scan_assignment_and_field_rules() {
        let f = one(
            "f.c",
            "int g;\nint run(struct E *e) { int local = 1; g = local; e->size = helper(g); g += 2; return local; }\n",
        );
        let run = f.symbols.iter().find(|s| s.label == "run").unwrap();
        assert_eq!(run.defines, vec!["e".to_string(), "g".to_string(), "local".to_string()]);
        assert_eq!(run.writes, vec!["g".to_string(), "local".to_string(), "size".to_string()]);
        assert_eq!(run.refs, vec!["E".to_string(), "helper".to_string(), "int".to_string()]);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("int f(int x) { return ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1; }\n");
        let f = one("deep.c", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
