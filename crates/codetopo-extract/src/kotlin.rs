// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Kotlin extractor (`.kt`, `.kts`) — beside Rust, TypeScript/
//! JavaScript, and Python.
//!
//! Node kinds below were verified against
//! `tree-sitter-kotlin-ng-1.1.0/src/node-types.json` in the crates.io
//! registry cache (the pinned crate, never guessed) and against
//! parse-tree dumps of the fixture: `source_file`, `import`
//! (children: `qualified_identifier`, optional alias `identifier`),
//! `class_declaration` (field `name`; the leading keyword token is
//! `class` / `interface`, children `primary_constructor`,
//! `delegation_specifiers`, `class_body`, `enum_class_body`),
//! `object_declaration`, `companion_object`,
//! `function_declaration` (field `name`; children
//! `function_value_parameters`, return `type`, `function_body`),
//! `property_declaration` (children `variable_declaration`,
//! initializer `expression`, `getter`, `setter`),
//! `variable_declaration` (children `identifier`, `type`),
//! `call_expression` (first named child is the callee expression),
//! `navigation_expression` (children: object expression, member
//! `identifier`), `assignment` (fields `left`, `operator`, `right`).
//! The upstream `tree-sitter-kotlin` crate (fwcd) pins
//! tree-sitter <0.23, whose `links` value cannot coexist with this
//! workspace's tree-sitter 0.25; the maintained `-ng` fork of the
//! same grammar uses the `tree-sitter-language` binding the other
//! pinned grammars use, so it is the dependency.
//!
//! The mapping follows the Python extractor's spec shape: module
//! ids are path-derived (§3.1's rule, unchanged); symbols (§3.2 —
//! top-level properties are variables, class-level properties are
//! not symbols, constructor `val`/`var` parameters are not symbols,
//! `typealias` emits no symbol — the code schema has no alias
//! mapping for it, the same silence the TS walker keeps for
//! re-export aliases); imports (§3.3's dotted rule; the binding is
//! the alias or the last segment — what a Kotlin import actually
//! binds); calls (§3.4 — the callee text as written); heritage
//! (delegation specifiers, Inherits only); the defines/refs/writes
//! scan (§3.6's marimo discipline: nested declarations pruned,
//! reads that are also defines dropped, augmented-assignment
//! targets read not written, only the root of a navigation chain
//! is a read); decisions (§3.2's enumeration, Kotlin form: `if`
//! +1, `when` entries − 1, `for`/`while`/`do-while` +1, each
//! `catch` +1, each `&&`/`||`/`?:` +1). The walk runs on an
//! explicit heap stack (the E-1 lesson).

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
        .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
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

/// Path-derived module id — the shared rule (Python §3.1): package,
/// then the parent directories, then the file stem.
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
    path: Vec<String>,
    enclosing: Vec<String>,
    in_class: bool,
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
    calls: Vec<RawCall>,
    heritage: Vec<RawHeritage>,
}

/// One step of the walker's explicit work stack (see the Python
/// walker's note: recursion over the syntax tree overflowed on
/// generated deep-chain files; the walk replays the recursive
/// order on this heap stack instead).
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
            "function_declaration" => self.schedule_function(node, stack),
            "class_declaration" => self.schedule_class(node, stack),
            "object_declaration" | "companion_object" => self.schedule_object(node, stack),
            "property_declaration" if self.enclosing.is_empty() => {
                self.handle_module_property(node, stack)
            }
            "import" => self.handle_import(node),
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

    /// One `call_expression` → one RawCall; the callee is the first
    /// named child (an identifier, a navigation expression, …) and
    /// `to_name` is its text as written, trimmed (§3.4).
    fn record_call(&mut self, node: Node) {
        if let Some(f) = node.named_child(0) {
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

    /// Schedule a function declaration: the symbol's scan is
    /// computed now; the stack then replays the recursive order —
    /// parameter defaults walk in the enclosing scope, the symbol
    /// opens, the body walks, the scope closes.
    fn schedule_function<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            // An anonymous function is not a named symbol; walk it
            // in the current scope.
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let kind = if self.in_class {
            NodeKind::Method
        } else {
            NodeKind::Function
        };
        let params = find_child(node, "function_value_parameters");
        let body = find_child(node, "function_body");
        let (defines, mut refs, writes) = fn_body_info(self.src, params, body);
        // The return type's names are refs of the function (the
        // Python D5 rule). The return type is the type child that
        // follows the parameters (a type child before them is an
        // extension receiver, not a return type).
        if let Some(t) = return_type_of(node) {
            let mut set: HashSet<String> = refs.iter().cloned().collect();
            scan_type_refs(t, self.src, &mut set);
            refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
            refs.sort();
        }
        let (end_line, signature) = measured(node, body, self.src);
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
            end_line,
            signature,
            decisions,
            in_class: false,
        });
        if let Some(p) = params {
            self.push_children(stack, p);
        }
    }

    /// Schedule a class declaration. Heritage comes from the
    /// delegation specifiers; a leading `interface` keyword token
    /// flips the kind to interface.
    fn schedule_class<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let kind = if has_token(node, "interface") {
            NodeKind::Interface
        } else {
            NodeKind::Class
        };
        self.push_heritage(node, &name);
        let body = find_child(node, "class_body").or_else(|| find_child(node, "enum_class_body"));
        let refs = body.map(|b| body_refs(b, self.src)).unwrap_or_default();
        let (end_line, signature) = measured(node, body, self.src);
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
            refs,
            writes: Vec::new(),
            end_line,
            signature,
            decisions: None,
            in_class: true,
        });
        // Supertype constructor calls in the delegation specifiers
        // are evaluated in the enclosing scope — pushed last so
        // they run before the symbol opens.
        if let Some(d) = find_child(node, "delegation_specifiers") {
            self.push_children(stack, d);
        }
        if let Some(p) = find_child(node, "primary_constructor") {
            self.push_children(stack, p);
        }
    }

    /// `object Foo { … }` and companion objects are class-kind
    /// symbols; an unnamed companion is `Companion` (the name the
    /// language itself gives it).
    fn schedule_object<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n))
            .unwrap_or_else(|| "Companion".to_string());
        self.push_heritage(node, &name);
        let body = find_child(node, "class_body");
        let refs = body.map(|b| body_refs(b, self.src)).unwrap_or_default();
        let (end_line, signature) = measured(node, body, self.src);
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
            refs,
            writes: Vec::new(),
            end_line,
            signature,
            decisions: None,
            in_class: true,
        });
        if let Some(d) = find_child(node, "delegation_specifiers") {
            self.push_children(stack, d);
        }
    }

    /// Heritage rows from a declaration's delegation specifiers:
    /// each specifier contributes the last segment of its type.
    fn push_heritage(&mut self, node: Node, type_name: &str) {
        let Some(specs) = find_child(node, "delegation_specifiers") else {
            return;
        };
        for i in 0..specs.named_child_count() {
            let Some(spec) = specs.named_child(i) else {
                continue;
            };
            if let Some(t) = delegation_target(spec, self.src) {
                self.heritage.push(RawHeritage {
                    type_name: type_name.to_string(),
                    target_name: t,
                    kind: EdgeKind::Inherits,
                    line: line_of(node),
                });
            }
        }
    }

    /// Module-level properties are `variable` symbols (§3.2). A
    /// destructuring declaration yields one symbol per name.
    fn handle_module_property<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let mut names: Vec<String> = Vec::new();
        for i in 0..node.named_child_count() {
            let Some(c) = node.named_child(i) else { continue };
            match c.kind() {
                "variable_declaration" => {
                    if let Some(id) = find_child(c, "identifier") {
                        names.push(self.text(id));
                    }
                }
                "multi_variable_declaration" => {
                    for j in 0..c.named_child_count() {
                        if let Some(vd) = c.named_child(j) {
                            if let Some(id) = find_child(vd, "identifier") {
                                names.push(self.text(id));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // The initializer is the child that is none of the
        // declaration's own parts; its reads are the variable's
        // refs, and its calls belong to the module (§3.4), walked
        // below.
        let init = children(node).into_iter().find(|c| {
            !matches!(
                c.kind(),
                "variable_declaration"
                    | "multi_variable_declaration"
                    | "modifiers"
                    | "getter"
                    | "setter"
                    | "property_delegate"
            )
        });
        let mut refs: Vec<String> = init.map(|e| body_refs(e, self.src)).unwrap_or_default();
        refs.sort();
        refs.dedup();
        let signature = signature_of(node, init, self.src);
        for name in &names {
            self.push_symbol(
                NodeKind::Variable,
                name,
                node,
                Vec::new(),
                refs.clone(),
                Vec::new(),
                Some(end_line_of(node)),
                signature.clone(),
                None,
            );
            self.pop_symbol();
        }
        if let Some(e) = init {
            stack.push(Step::Enter(e));
        }
    }

    /// `import a.b.C` / `import a.b.C as D` / `import a.b.*` — one
    /// RawImport; the binding is the alias or the last segment; a
    /// wildcard import binds nothing (the Python glob rule).
    fn handle_import(&mut self, node: Node) {
        let line = line_of(node);
        let mut dotted: Option<String> = None;
        let mut alias: Option<String> = None;
        for i in 0..node.named_child_count() {
            let Some(c) = node.named_child(i) else { continue };
            match c.kind() {
                "qualified_identifier" => dotted = Some(self.text(c)),
                "identifier" => alias = Some(self.text(c)),
                _ => {}
            }
        }
        let Some(dotted) = dotted else { return };
        let wildcard = self.text(node).contains(".*");
        let bindings = if wildcard {
            Vec::new()
        } else if let Some(a) = alias {
            vec![a]
        } else {
            vec![dotted.rsplit('.').next().unwrap_or("").to_string()]
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: self.resolve_dotted(&dotted),
            bindings,
            line,
        });
    }

    /// The shared dotted rule (§3.3): the first segment decides —
    /// the workspace package's own name means internal, anything
    /// else is `external::{first}`.
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
}

/// First direct named child of `node` with the given kind.
fn find_child<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    (0..node.named_child_count())
        .filter_map(|i| node.named_child(i))
        .find(|c| c.kind() == kind)
}

/// True when `node` has a direct child (named or not) of `kind` —
/// how the grammar marks `interface` class declarations.
fn has_token(node: Node, kind: &str) -> bool {
    (0..node.child_count()).any(|i| node.child(i).map(|c| c.kind() == kind).unwrap_or(false))
}

/// The return-type node of a function declaration: the type child
/// that follows the parameters (a type child before the parameters
/// is an extension receiver, not a return type).
fn return_type_of<'t>(node: Node<'t>) -> Option<Node<'t>> {
    let mut seen_params = false;
    for i in 0..node.named_child_count() {
        let Some(c) = node.named_child(i) else { continue };
        if c.kind() == "function_value_parameters" {
            seen_params = true;
            continue;
        }
        if seen_params
            && matches!(
                c.kind(),
                "user_type"
                    | "nullable_type"
                    | "non_nullable_type"
                    | "function_type"
                    | "parenthesized_type"
            )
        {
            return Some(c);
        }
    }
    None
}

/// The heritage target of one delegation specifier: the last
/// segment of its type (a constructor invocation's type, an
/// explicit delegation's type, or a bare type).
fn delegation_target(spec: Node, src: &[u8]) -> Option<String> {
    for i in 0..spec.named_child_count() {
        let Some(c) = spec.named_child(i) else { continue };
        match c.kind() {
            "constructor_invocation" | "explicit_delegation" => {
                if let Some(t) = find_child(c, "user_type") {
                    return Some(last_type_segment(t, src));
                }
            }
            "user_type" => return Some(last_type_segment(c, src)),
            _ => {}
        }
    }
    None
}

/// Last identifier segment of a type node (`a.b.C<T>` → `C`),
/// walking in source order. Iterative.
fn last_type_segment(node: Node, src: &[u8]) -> String {
    let mut last = String::new();
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" {
            last = txt(n, src);
        }
        for i in (0..n.named_child_count()).rev() {
            if let Some(c) = n.named_child(i) {
                stack.push(c);
            }
        }
    }
    last
}

/// (end_line, signature) for a declaration: the shared signature
/// rule with the cut at the body node (Kotlin bodies are children,
/// not fields, so the shared `measured_fields` field lookup does
/// not find them).
fn measured(node: Node, body: Option<Node>, src: &[u8]) -> (Option<u32>, Option<String>) {
    (Some(end_line_of(node)), signature_of(node, body, src))
}

/// Decision points in a body (the Kotlin form of §3.2's
/// enumeration). Iterative (E-1): nested declarations are pruned —
/// their decisions belong to their own symbols.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "function_declaration" | "class_declaration" | "object_declaration"
            | "companion_object" => continue,
            _ => {}
        }
        n += match node.kind() {
            "if_expression" | "for_statement" | "while_statement" | "do_while_statement"
            | "catch_block" => 1,
            "when_expression" => {
                let entries = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .filter(|c| c.kind() == "when_entry")
                    .count() as u32;
                entries.saturating_sub(1)
            }
            "binary_expression" => {
                let op = node
                    .child_by_field_name("operator")
                    .map(|o| txt(o, src))
                    .unwrap_or_default();
                if matches!(op.as_str(), "&&" | "||" | "?:") {
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

/// Accumulators for the defines/refs/writes scan.
#[derive(Default)]
struct Acc {
    defines: HashSet<String>,
    refs: HashSet<String>,
    writes: HashSet<String>,
}

/// Every identifier inside a type expression is a ref (D5).
/// Iterative, like the Python scan's.
fn scan_type_refs(node: Node, src: &[u8], out: &mut HashSet<String>) {
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if node.kind() == "identifier" {
            out.insert(txt(node, src));
            continue;
        }
        for i in 0..node.named_child_count() {
            if let Some(c) = node.named_child(i) {
                stack.push(c);
            }
        }
    }
}

/// The binding names of a parameter list (`parameter` children of
/// `function_value_parameters`, `class_parameter` children of
/// `class_parameters`, lambda `variable_declaration`s). Each
/// contributes its `identifier` child; a parameter's type
/// contributes refs.
fn collect_params(node: Node, src: &[u8], acc: &mut Acc) {
    for i in 0..node.named_child_count() {
        let Some(c) = node.named_child(i) else { continue };
        match c.kind() {
            "parameter" | "class_parameter" => {
                if let Some(id) = find_child(c, "identifier") {
                    acc.defines.insert(txt(id, src));
                }
                for j in 0..c.named_child_count() {
                    if let Some(t) = c.named_child(j) {
                        if matches!(
                            t.kind(),
                            "user_type" | "nullable_type" | "non_nullable_type" | "function_type"
                        ) {
                            scan_type_refs(t, src, &mut acc.refs);
                        }
                    }
                }
            }
            "variable_declaration" => {
                if let Some(id) = find_child(c, "identifier") {
                    acc.defines.insert(txt(id, src));
                }
            }
            _ => {}
        }
    }
}

/// Record an assignment target. A plain identifier defines AND
/// writes for `=`, and is only read for augmented operators (the
/// Python rule). A navigation target writes its member name; its
/// object is not a read for `=` (the Python write-target rule) and
/// is read for augmented operators.
fn record_target(node: Node, src: &[u8], acc: &mut Acc, augmented: bool) {
    match node.kind() {
        "identifier" => {
            let name = txt(node, src);
            if augmented {
                acc.refs.insert(name);
            } else {
                acc.defines.insert(name.clone());
                acc.writes.insert(name);
            }
        }
        "navigation_expression" => {
            if augmented {
                if let Some(obj) = node.named_child(0) {
                    scan_w(obj, src, acc);
                }
            } else if let Some(id) = node.named_child(1) {
                if id.kind() == "identifier" {
                    acc.writes.insert(txt(id, src));
                }
            }
        }
        "multi_variable_declaration" => {
            for i in 0..node.named_child_count() {
                if let Some(vd) = node.named_child(i) {
                    if let Some(id) = find_child(vd, "identifier") {
                        let name = txt(id, src);
                        if augmented {
                            acc.refs.insert(name);
                        } else {
                            acc.defines.insert(name.clone());
                            acc.writes.insert(name);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

/// The marimo scan over a body (§3.6): defines / refs / writes with
/// nested declarations pruned. Iterative, on an explicit stack —
/// every accumulator is a set, so visit order cannot change the
/// result.
fn scan_w(root: Node, src: &[u8], acc: &mut Acc) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            // Nested declarations are their own symbols.
            "function_declaration" | "class_declaration" | "object_declaration"
            | "companion_object" | "import" => {}
            "identifier" => {
                acc.refs.insert(txt(node, src));
            }
            // Only the root of a navigation chain is a read.
            "navigation_expression" => {
                if let Some(obj) = node.named_child(0) {
                    stack.push(obj);
                }
            }
            "assignment" => {
                let augmented = node
                    .child_by_field_name("operator")
                    .map(|o| txt(o, src) != "=")
                    .unwrap_or(false);
                if let Some(l) = node.child_by_field_name("left") {
                    record_target(l, src, acc, augmented);
                }
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(r);
                }
            }
            "property_declaration" => {
                // A local property binds its names (define + write,
                // the assignment precedent); its type's names are
                // refs; its initializer is read.
                for i in 0..node.named_child_count() {
                    let Some(c) = node.named_child(i) else { continue };
                    match c.kind() {
                        "variable_declaration" => {
                            if let Some(id) = find_child(c, "identifier") {
                                let name = txt(id, src);
                                acc.defines.insert(name.clone());
                                acc.writes.insert(name);
                            }
                            for j in 0..c.named_child_count() {
                                if let Some(t) = c.named_child(j) {
                                    if t.kind() != "identifier" {
                                        scan_type_refs(t, src, &mut acc.refs);
                                    }
                                }
                            }
                        }
                        "multi_variable_declaration" => record_target(c, src, acc, false),
                        "getter" | "setter" | "modifiers" => {}
                        _ => stack.push(c),
                    }
                }
            }
            "for_statement" => {
                for i in 0..node.named_child_count() {
                    let Some(c) = node.named_child(i) else { continue };
                    match c.kind() {
                        "variable_declaration" => {
                            if let Some(id) = find_child(c, "identifier") {
                                acc.defines.insert(txt(id, src));
                            }
                        }
                        "multi_variable_declaration" => {
                            for j in 0..c.named_child_count() {
                                if let Some(vd) = c.named_child(j) {
                                    if let Some(id) = find_child(vd, "identifier") {
                                        acc.defines.insert(txt(id, src));
                                    }
                                }
                            }
                        }
                        _ => stack.push(c),
                    }
                }
            }
            "catch_block" => {
                // Children: the parameter identifier, its type, the
                // block. The identifier binds; the type is refs.
                for i in 0..node.named_child_count() {
                    let Some(c) = node.named_child(i) else { continue };
                    match c.kind() {
                        "identifier" => {
                            acc.defines.insert(txt(c, src));
                        }
                        "user_type" | "nullable_type" | "non_nullable_type" => {
                            scan_type_refs(c, src, &mut acc.refs);
                        }
                        _ => stack.push(c),
                    }
                }
            }
            "lambda_literal" => {
                for i in 0..node.named_child_count() {
                    let Some(c) = node.named_child(i) else { continue };
                    if c.kind() == "lambda_parameters" {
                        collect_params(c, src, acc);
                    } else {
                        stack.push(c);
                    }
                }
            }
            "anonymous_function" => {
                for i in 0..node.named_child_count() {
                    let Some(c) = node.named_child(i) else { continue };
                    match c.kind() {
                        "function_value_parameters" => collect_params(c, src, acc),
                        "user_type" | "nullable_type" | "non_nullable_type" => {
                            scan_type_refs(c, src, &mut acc.refs);
                        }
                        _ => stack.push(c),
                    }
                }
            }
            _ => {
                for i in 0..node.named_child_count() {
                    if let Some(c) = node.named_child(i) {
                        stack.push(c);
                    }
                }
            }
        }
    }
}

/// (defines, refs, writes) of a function: parameters bind, the
/// body scans, reads that are also defines are dropped (the
/// finalize step shared with the other walkers).
fn fn_body_info(
    src: &[u8],
    params: Option<Node>,
    body: Option<Node>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut acc = Acc::default();
    if let Some(p) = params {
        collect_params(p, src, &mut acc);
    }
    if let Some(b) = body {
        scan_w(b, src, &mut acc);
    }
    finalize(acc)
}

/// Refs read in an initializer or class body, with locally bound
/// names filtered out.
fn body_refs(node: Node, src: &[u8]) -> Vec<String> {
    let mut acc = Acc::default();
    scan_w(node, src, &mut acc);
    let (_, refs, _) = finalize(acc);
    refs
}

fn finalize(acc: Acc) -> (Vec<String>, Vec<String>, Vec<String>) {
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
    fn kt_deep_expression_chain_does_not_overflow() {
        // Regression shape (E-1): a generated module whose value is
        // one expression nested tens of thousands of levels deep.
        let chain = vec!["\"AAAA\""; 50_000].join(" + ");
        let src = format!("val BLOB = {chain}\n");
        let fs = run("blob.kt", &src);
        let ids: Vec<&str> = fs.symbols.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["engine::blob::BLOB"]);
        assert_eq!(fs.symbols[0].kind, NodeKind::Variable);
    }

    #[test]
    fn kt_module_id_rules() {
        assert_eq!(module_id_for("engine", Path::new("Main.kt")), "engine::Main");
        assert_eq!(
            module_id_for("engine", Path::new("core/Engine.kt")),
            "engine::core::Engine"
        );
        assert_eq!(
            module_id_for("engine", Path::new("a/b/Util.kt")),
            "engine::a::b::Util"
        );
    }

    const SYMBOLS_KT: &str = r#"package engine.core

import engine.shapes.Circle
import engine.shapes.Shape as S
import kotlin.math.*

const val LIMIT = 10

var counter = 0

fun topLevel(x: Int): Int {
    fun nested(y: Int): Int {
        return y + x
    }
    return nested(x)
}

interface Drawable {
    fun draw()
}

class Engine(val shape: S) : Drawable {
    var size: Int = LIMIT

    override fun draw() {
    }

    fun render(): Double {
        counter += 1
        return shape.area()
    }
}

object Registry {
    fun lookup(): Int {
        return 1
    }
}
"#;

    #[test]
    fn kt_symbols_and_kinds() {
        let fs = run("core/Engine.kt", SYMBOLS_KT);
        assert_eq!(fs.module_id, "engine::core::Engine");
        let m = by_id(&fs);
        let want: BTreeMap<&str, NodeKind> = [
            ("engine::core::Engine::LIMIT", NodeKind::Variable),
            ("engine::core::Engine::counter", NodeKind::Variable),
            ("engine::core::Engine::topLevel", NodeKind::Function),
            ("engine::core::Engine::topLevel::nested", NodeKind::Function),
            ("engine::core::Engine::Drawable", NodeKind::Interface),
            ("engine::core::Engine::Drawable::draw", NodeKind::Method),
            ("engine::core::Engine::Engine", NodeKind::Class),
            ("engine::core::Engine::Engine::draw", NodeKind::Method),
            ("engine::core::Engine::Engine::render", NodeKind::Method),
            ("engine::core::Engine::Registry", NodeKind::Class),
            ("engine::core::Engine::Registry::lookup", NodeKind::Method),
        ]
        .into_iter()
        .collect();
        let got: BTreeMap<&str, NodeKind> = m.iter().map(|(k, v)| (*k, v.kind)).collect();
        assert_eq!(got, want);
        // Class-level properties and constructor val parameters are
        // NOT symbols.
        assert!(!m.contains_key("engine::core::Engine::Engine::size"));
        assert!(!m.contains_key("engine::core::Engine::Engine::shape"));
    }

    #[test]
    fn kt_imports_and_heritage() {
        let fs = run("core/Engine.kt", SYMBOLS_KT);
        let got: Vec<(&str, Vec<String>)> = fs
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        let want: Vec<(&str, Vec<String>)> = vec![
            ("engine::shapes::Circle", sv(&["Circle"])),
            ("engine::shapes::Shape", sv(&["S"])),
            ("external::kotlin", sv(&[])),
        ];
        assert_eq!(got, want);
        let heritage: Vec<(&str, &str)> = fs
            .heritage
            .iter()
            .map(|h| (h.type_name.as_str(), h.target_name.as_str()))
            .collect();
        assert_eq!(heritage, vec![("Engine", "Drawable")]);
    }

    #[test]
    fn kt_scan_hand_derived() {
        let fs = run("core/Engine.kt", SYMBOLS_KT);
        let m = by_id(&fs);
        // render: `counter += 1` reads counter (augmented rule);
        // `shape.area()` reads the root `shape`; the return type
        // Double is a ref (D5). No defines, no writes.
        let render = m["engine::core::Engine::Engine::render"];
        assert_eq!(render.defines, sv(&[]));
        assert_eq!(render.refs, sv(&["Double", "counter", "shape"]));
        assert_eq!(render.writes, sv(&[]));
        // topLevel: parameter x defines; the nested function's
        // body is pruned; `nested(x)` reads the callee name; the
        // return type Int is a ref.
        let top = m["engine::core::Engine::topLevel"];
        assert_eq!(top.defines, sv(&["x"]));
        assert_eq!(top.refs, sv(&["Int", "nested"]));
        assert_eq!(top.writes, sv(&[]));
    }
}
