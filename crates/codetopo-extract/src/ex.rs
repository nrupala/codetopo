// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Elixir extractor (`.ex` / `.exs`) — beside Rust,
//! TypeScript/JavaScript, Python, Kotlin, C, Java, Go,
//! Shell, C++, Swift, Ruby, R, Julia, C#, PHP, Scala,
//! and Haskell.
//!
//! Node kinds below were verified against
//! `tree-sitter-elixir-0.3.5/src/node-types.json` in the
//! crates.io registry cache and against parse trees of the
//! fixture (the pinned crate, never guessed). This grammar
//! is macro-shaped: nearly every Elixir form is a `call`
//! (field `target`; an `arguments` child; an optional
//! `do_block` child) — `source`, `call` (target an
//! `identifier` for a local call or a `dot` — fields `left`,
//! `operator`, `right` — for a remote call), `arguments`,
//! `do_block`, `alias` (one node, the whole dotted name),
//! `unary_operator` (fields `operator`, `operand` — a module
//! attribute is `@` over an `identifier` read or a `call`
//! set), `binary_operator` (fields `left`, `operator`,
//! `right`), `map` (a `struct` child — an `alias` or the
//! `__MODULE__` identifier — plus `map_content`),
//! `keywords` / `pair` (fields `key` — a `keyword` — and
//! `value`), `stab_clause` (fields `left` — an `arguments`
//! pattern — `operator`, `right` — a `body`),
//! `anonymous_function`, `tuple`, `list`, `identifier`,
//! `atom`.
//!
//! The mapping follows the Ruby extractor's spec shape:
//! module ids are path-derived (the shared rule). Symbols
//! (§3.2 — a `defmodule` is a class-kind symbol, as a Ruby
//! module is: a named container of functions, and the
//! schema has no namespace kind; its dotted name is a
//! symbol-path segment list, the PHP namespace rule. A
//! `defprotocol` is an interface; a `defimpl` is a class
//! whose segments are the protocol's followed by the
//! `for:` type's — the implementation module's real name.
//! `def` / `defp` / `defmacro` / `defmacrop` inside a
//! module are methods, outside are functions.
//! `defstruct` / `defexception` define the enclosing
//! module's struct fields — fields, not symbols (the Ruby
//! class-assignment precedent; the struct takes the
//! module's own name). A `@callback` and the other
//! doc/spec attributes declare contracts, not functions:
//! no symbol, no call. Top-level assignments are
//! variables). Imports (the `alias` / `import` / `require`
//! / `use` calls: a dotted name whose first segment matches
//! the package name case-insensitively is internal — the
//! segment is read as the package name, the PHP rule —
//! anything else is `external::{first segment}`; `alias`
//! binds its last segment or its `as:` name, `require`
//! binds its last segment, `import` binds its `only:`
//! function names, `use` binds nothing). Calls (§3.4 — the
//! called function's name; for a remote call the receiver
//! is not part of it, the Python attribute rule; the
//! declaration calls and the control forms — `if`,
//! `unless`, `case`, `cond`, `for`, `with`, `try`,
//! `receive` — are not calls). Heritage (a module's
//! `@behaviour M` is Implements; a `defimpl P, for: T` is
//! T Implements P; Elixir has no inheritance). The
//! defines/refs/writes scan (§3.6's marimo discipline:
//! nested declarations pruned, reads that are also defines
//! dropped, a match target is a define and a write, only
//! the receiver of a remote call is a read — a bare local
//! call's name is a ref, as a plain-name call is in Python;
//! a module attribute read is a ref under its full written
//! name, `@` included; `__MODULE__` is never a ref, the
//! `$this` rule); decisions (§3.2's enumeration, Elixir
//! form: `if` / `unless` +1, `case` / `cond` contribute
//! clauses − 1, each `&&` / `||` / `and` / `or` +1). The
//! walk runs on an explicit heap stack (the E-1 lesson).

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

/// The declaration calls: handled as declarations, never
/// recorded as calls, and pruned by the scan.
const DECL_CALLS: &[&str] = &[
    "defmodule",
    "defprotocol",
    "defimpl",
    "def",
    "defp",
    "defmacro",
    "defmacrop",
    "defstruct",
    "defexception",
    "alias",
    "import",
    "require",
    "use",
];

/// The control forms: calls in the grammar, control flow
/// in the language — not calls, and their names are not
/// refs.
const CONTROL_CALLS: &[&str] = &[
    "if", "unless", "case", "cond", "for", "with", "try", "receive",
];

/// Attributes that declare documentation or contracts:
/// pruned whole (their contents are types and prose, not
/// code — a `@callback`'s `area(...)` is not a call).
const PRUNED_ATTRS: &[&str] = &[
    "moduledoc", "doc", "spec", "callback", "type", "typep", "opaque",
];

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_elixir::LANGUAGE.into())
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
            "call" => self.visit_call(node, stack),
            "unary_operator" => {
                if node
                    .child_by_field_name("operator")
                    .map(|o| self.text(o))
                    .as_deref()
                    == Some("@")
                {
                    self.visit_attribute(node, stack);
                } else {
                    self.push_children(stack, node);
                }
            }
            "binary_operator"
                if self.enclosing.is_empty()
                    && self.path.is_empty()
                    && operator_of(node, self.src).as_deref() == Some("=") =>
            {
                // A top-level assignment to a plain name is a
                // variable.
                let left = node.child_by_field_name("left");
                let right = node.child_by_field_name("right");
                match (left, right) {
                    (Some(l), Some(r)) if l.kind() == "identifier" => {
                        let name = self.text(l);
                        let mut refs: Vec<String> = body_refs(r, self.src);
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
                            signature_of(node, Some(r), self.src),
                            None,
                        );
                        self.pop_symbol();
                        stack.push(Step::Enter(r));
                    }
                    _ => self.push_children(stack, node),
                }
            }
            _ => self.push_children(stack, node),
        }
    }

    fn visit_call<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(target) = node.child_by_field_name("target") else {
            self.push_children(stack, node);
            return;
        };
        match target.kind() {
            "dot" => {
                // A remote call: `to_name` is the function's
                // name (the receiver is not part of it).
                if let Some(f) = target.child_by_field_name("right") {
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
            "identifier" => {
                let name = self.text(target);
                match name.as_str() {
                    "defmodule" => self.schedule_module(node, NodeKind::Class, stack),
                    "defprotocol" => self.schedule_module(node, NodeKind::Interface, stack),
                    "defimpl" => self.schedule_impl(node, stack),
                    "def" | "defp" | "defmacro" | "defmacrop" => {
                        self.schedule_function(node, stack)
                    }
                    "defstruct" | "defexception" => {
                        // Struct fields are not symbols; the
                        // arguments (field names and literal
                        // defaults) hold no calls, but walk
                        // them so nothing nested is missed.
                        if let Some(args) = child_kind(node, "arguments") {
                            self.push_children(stack, args);
                        }
                    }
                    "alias" => self.handle_alias(node),
                    "import" => self.handle_import(node, ImportMode::Import),
                    "require" => self.handle_import(node, ImportMode::Require),
                    "use" => self.handle_import(node, ImportMode::Use),
                    _ if CONTROL_CALLS.contains(&name.as_str()) => {
                        self.push_children(stack, node);
                    }
                    _ => {
                        if !name.is_empty() {
                            self.calls.push(RawCall {
                                from_id: self.current_scope(),
                                to_name: name,
                                line: line_of(node),
                            });
                        }
                        self.push_children(stack, node);
                    }
                }
            }
            _ => self.push_children(stack, node),
        }
    }

    /// A module attribute. `@behaviour M` is heritage for
    /// the enclosing module; the doc/spec attributes are
    /// pruned; any other set (`@pi 3.14`) names no symbol —
    /// its value is walked for calls; a read (`@pi`) holds
    /// nothing to walk.
    fn visit_attribute<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(operand) = node.child_by_field_name("operand") else {
            return;
        };
        if operand.kind() != "call" {
            return;
        }
        let Some(t) = operand.child_by_field_name("target") else {
            return;
        };
        if t.kind() != "identifier" {
            return;
        }
        let name = self.text(t);
        if name == "behaviour" {
            if let (Some(type_name), Some(args)) =
                (self.path.last().cloned(), child_kind(operand, "arguments"))
            {
                if let Some(a) = named_children_of(args).into_iter().next() {
                    if a.kind() == "alias" {
                        if let Some(target_name) = alias_segments(&self.text(a)).last().cloned() {
                            self.heritage.push(RawHeritage {
                                type_name,
                                target_name,
                                kind: EdgeKind::Implements,
                                line: line_of(node),
                            });
                        }
                    }
                }
            }
            return;
        }
        if PRUNED_ATTRS.contains(&name.as_str()) {
            return;
        }
        if let Some(args) = child_kind(operand, "arguments") {
            self.push_children(stack, args);
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

    /// Schedule a `defmodule` / `defprotocol`: a class /
    /// interface symbol over the dotted name's segments;
    /// the body is the `do_block` (or a `do:` value).
    fn schedule_module<'t>(&mut self, node: Node<'t>, kind: NodeKind, stack: &mut Vec<Step<'t>>) {
        let Some(args) = child_kind(node, "arguments") else {
            self.push_children(stack, node);
            return;
        };
        let Some(first) = named_children_of(args).into_iter().next() else {
            self.push_children(stack, node);
            return;
        };
        if first.kind() != "alias" {
            self.push_children(stack, node);
            return;
        }
        let names = alias_segments(&self.text(first));
        if names.is_empty() {
            self.push_children(stack, node);
            return;
        }
        let (body_roots, cut) = body_roots_of(node, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        for b in body_roots.iter().rev() {
            stack.push(Step::Enter(*b));
        }
        stack.push(Step::OpenSymbol {
            kind,
            names,
            node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature: sig_of(node, cut, self.src),
            decisions: None,
            in_class: true,
        });
    }

    /// Schedule a `defimpl`: a class symbol whose segments
    /// are the protocol's followed by the `for:` type's,
    /// plus an Implements heritage row.
    fn schedule_impl<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(args) = child_kind(node, "arguments") else {
            self.push_children(stack, node);
            return;
        };
        let arg_nodes = named_children_of(args);
        let Some(proto_node) = arg_nodes.iter().find(|c| c.kind() == "alias") else {
            self.push_children(stack, node);
            return;
        };
        let proto = alias_segments(&self.text(*proto_node));
        let for_segs = keyword_value(args, "for", self.src)
            .filter(|v| v.kind() == "alias")
            .map(|v| alias_segments(&self.text(v)))
            .unwrap_or_default();
        if proto.is_empty() || for_segs.is_empty() {
            self.push_children(stack, node);
            return;
        }
        if let (Some(t), Some(p)) = (for_segs.last(), proto.last()) {
            self.heritage.push(RawHeritage {
                type_name: t.clone(),
                target_name: p.clone(),
                kind: EdgeKind::Implements,
                line: line_of(node),
            });
        }
        let mut names = proto;
        names.extend(for_segs);
        let (body_roots, cut) = body_roots_of(node, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        for b in body_roots.iter().rev() {
            stack.push(Step::Enter(*b));
        }
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Class,
            names,
            node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature: sig_of(node, cut, self.src),
            decisions: None,
            in_class: true,
        });
    }

    /// Schedule a `def` / `defp` / `defmacro` / `defmacrop`:
    /// the head (a call, a bare identifier, or a `when`
    /// binary over a call) names it and binds its
    /// parameters; the body is the `do_block` or a `do:`
    /// value; a head with neither is a body-less
    /// declaration (a protocol function).
    fn schedule_function<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(args) = child_kind(node, "arguments") else {
            self.push_children(stack, node);
            return;
        };
        let Some(head) = named_children_of(args)
            .into_iter()
            .find(|c| c.kind() != "keywords")
        else {
            self.push_children(stack, node);
            return;
        };
        let (head_call, guard) = if head.kind() == "binary_operator"
            && operator_of(head, self.src).as_deref() == Some("when")
        {
            (head.child_by_field_name("left"), head.child_by_field_name("right"))
        } else {
            (Some(head), None)
        };
        let Some(head_call) = head_call else {
            self.push_children(stack, node);
            return;
        };
        let (name, params): (String, Vec<Node>) = match head_call.kind() {
            "call" => {
                let Some(t) = head_call.child_by_field_name("target") else {
                    self.push_children(stack, node);
                    return;
                };
                if t.kind() != "identifier" {
                    self.push_children(stack, node);
                    return;
                }
                let params = child_kind(head_call, "arguments")
                    .map(|a| named_children_of(a))
                    .unwrap_or_default();
                (self.text(t), params)
            }
            "identifier" => (self.text(head_call), Vec::new()),
            _ => {
                self.push_children(stack, node);
                return;
            }
        };
        if name.is_empty() {
            self.push_children(stack, node);
            return;
        }
        let (body_roots, cut) = body_roots_of(node, self.src);
        let has_body = child_kind(node, "do_block").is_some()
            || keyword_value(args, "do", self.src).is_some();
        let (defines, refs, writes) = fn_body_info(self.src, &params, guard, &body_roots);
        let kind = if self.in_class {
            NodeKind::Method
        } else {
            NodeKind::Function
        };
        let decisions = if has_body {
            Some(count_decisions_in(&body_roots, self.src))
        } else {
            None
        };
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        for b in body_roots.iter().rev() {
            stack.push(Step::Enter(*b));
        }
        stack.push(Step::OpenSymbol {
            kind,
            names: vec![name],
            node,
            defines,
            refs,
            writes,
            end_line: Some(end_line_of(node)),
            signature: sig_of(node, cut, self.src),
            decisions,
            in_class: false,
        });
    }

    /// An `alias` call → one RawImport per module it names
    /// (the brace form `alias A.{B, C}` names one per entry).
    fn handle_alias(&mut self, node: Node) {
        let Some(args) = child_kind(node, "arguments") else {
            return;
        };
        let arg_nodes = named_children_of(args);
        let Some(first) = arg_nodes.first() else {
            return;
        };
        match first.kind() {
            "alias" => {
                let segs = alias_segments(&self.text(*first));
                if segs.is_empty() {
                    return;
                }
                let binding = keyword_value(args, "as", self.src)
                    .filter(|v| v.kind() == "alias")
                    .and_then(|v| alias_segments(&self.text(v)).last().cloned())
                    .unwrap_or_else(|| segs[segs.len() - 1].clone());
                self.imports.push(RawImport {
                    from_module_id: self.module_id.clone(),
                    to_module_id: self.resolve_module(&segs),
                    bindings: vec![binding],
                    line: line_of(node),
                });
            }
            "dot" => {
                let Some(prefix) = first.child_by_field_name("left") else {
                    return;
                };
                if prefix.kind() != "alias" {
                    return;
                }
                let prefix_segs = alias_segments(&self.text(prefix));
                if let Some(tuple) = first.child_by_field_name("right") {
                    for entry in named_children_of(tuple) {
                        if entry.kind() != "alias" {
                            continue;
                        }
                        let mut segs = prefix_segs.clone();
                        segs.extend(alias_segments(&self.text(entry)));
                        if segs.is_empty() {
                            continue;
                        }
                        let binding = segs[segs.len() - 1].clone();
                        self.imports.push(RawImport {
                            from_module_id: self.module_id.clone(),
                            to_module_id: self.resolve_module(&segs),
                            bindings: vec![binding],
                            line: line_of(node),
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// An `import` / `require` / `use` call → one RawImport.
    fn handle_import(&mut self, node: Node, mode: ImportMode) {
        let Some(args) = child_kind(node, "arguments") else {
            return;
        };
        let Some(first) = named_children_of(args).into_iter().next() else {
            return;
        };
        if first.kind() != "alias" {
            return;
        }
        let segs = alias_segments(&self.text(first));
        if segs.is_empty() {
            return;
        }
        let bindings = match mode {
            ImportMode::Import => keyword_value(args, "only", self.src)
                .map(|v| keyword_list_names(v, self.src))
                .unwrap_or_default(),
            ImportMode::Require => vec![
                keyword_value(args, "as", self.src)
                    .filter(|v| v.kind() == "alias")
                    .and_then(|v| alias_segments(&self.text(v)).last().cloned())
                    .unwrap_or_else(|| segs[segs.len() - 1].clone()),
            ],
            ImportMode::Use => Vec::new(),
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: self.resolve_module(&segs),
            bindings,
            line: line_of(node),
        });
    }

    /// Resolve a dotted module name: internal when its
    /// first segment matches the package name
    /// case-insensitively (the segment is read as the
    /// package name); `external::{first segment}` otherwise.
    fn resolve_module(&self, segs: &[String]) -> String {
        if segs
            .first()
            .map(|f| f.eq_ignore_ascii_case(&self.pkg))
            .unwrap_or(false)
        {
            if segs.len() > 1 {
                format!("{}::{}", self.pkg, segs[1..].join("::"))
            } else {
                self.pkg.clone()
            }
        } else {
            format!("external::{}", segs.first().cloned().unwrap_or_default())
        }
    }
}

#[derive(Clone, Copy)]
enum ImportMode {
    Import,
    Require,
    Use,
}

/// The first direct child of `node` with the given kind.
fn child_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    children(node).into_iter().find(|c| c.kind() == kind)
}

/// Named children of a node (the `arguments` node's named
/// children are its argument expressions; the punctuation
/// is anonymous and never appears).
fn named_children_of(node: Node) -> Vec<Node> {
    children(node)
}

/// The segments of a dotted module name (`Quarry.Shapes.
/// Circle` — written without the space — is one `alias`
/// node's text).
fn alias_segments(text: &str) -> Vec<String> {
    text.split('.')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The operator text of a binary/unary operator node.
fn operator_of(node: Node, src: &[u8]) -> Option<String> {
    node.child_by_field_name("operator")
        .map(|o| txt(o, src))
}

/// The key of a `pair`: its `keyword` field's text with the
/// trailing `:` and whitespace stripped (`"area: "` →
/// `area`).
fn pair_key(pair: Node, src: &[u8]) -> Option<String> {
    let key = pair.child_by_field_name("key")?;
    if key.kind() != "keyword" {
        return None;
    }
    let raw = txt(key, src);
    let k = raw.trim().trim_end_matches(':').trim().to_string();
    if k.is_empty() {
        None
    } else {
        Some(k)
    }
}

/// The value of keyword `name` among a call's argument
/// keywords (the `keywords` children of its `arguments`).
fn keyword_value<'t>(args: Node<'t>, name: &str, src: &[u8]) -> Option<Node<'t>> {
    for c in children(args) {
        if c.kind() != "keywords" {
            continue;
        }
        for pair in children(c) {
            if pair.kind() == "pair" && pair_key(pair, src).as_deref() == Some(name) {
                return pair.child_by_field_name("value");
            }
        }
    }
    None
}

/// The function names an `only:` / `except:` list names:
/// the keys of its keyword pairs (`[area: 1]` → `[area]`).
fn keyword_list_names(list: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![list];
    while let Some(n) = stack.pop() {
        if n.kind() == "pair" {
            if let Some(k) = pair_key(n, src) {
                out.push(k);
            }
            continue;
        }
        for c in children(n) {
            stack.push(c);
        }
    }
    out.reverse();
    out
}

/// The body roots of a declaration call and the node the
/// signature is cut at: the `do_block`'s statement children,
/// or the single `do:` keyword value.
fn body_roots_of<'t>(node: Node<'t>, src: &[u8]) -> (Vec<Node<'t>>, Option<Node<'t>>) {
    if let Some(block) = child_kind(node, "do_block") {
        return (children(block), Some(block));
    }
    if let Some(args) = child_kind(node, "arguments") {
        if let Some(v) = keyword_value(args, "do", src) {
            // Cut at the keywords node that carries `do:`.
            let cut = children(args).into_iter().find(|c| c.kind() == "keywords");
            return (vec![v], cut);
        }
    }
    (Vec::new(), None)
}

/// A declaration's signature: the shared rule, with the
/// trailing `do` the cut leaves behind stripped.
fn sig_of(node: Node, cut: Option<Node>, src: &[u8]) -> Option<String> {
    let mut sig = signature_of(node, cut, src)?;
    while let Some(stripped) = sig.strip_suffix(" do") {
        sig = stripped.trim_end().to_string();
    }
    if sig.is_empty() {
        None
    } else {
        Some(sig)
    }
}

/// Decision points in a body (the Elixir form of §3.2's
/// enumeration). Iterative (E-1); nested declarations pruned.
fn count_decisions_in(roots: &[Node], src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack: Vec<Node> = roots.to_vec();
    while let Some(node) = stack.pop() {
        match node.kind() {
            "call" => {
                let target = node.child_by_field_name("target");
                let name = target
                    .filter(|t| t.kind() == "identifier")
                    .map(|t| txt(t, src))
                    .unwrap_or_default();
                if DECL_CALLS.contains(&name.as_str()) {
                    continue;
                }
                n += match name.as_str() {
                    "if" | "unless" => 1,
                    "case" | "cond" => {
                        let clauses = child_kind(node, "do_block")
                            .map(|b| {
                                children(b)
                                    .iter()
                                    .filter(|c| c.kind() == "stab_clause")
                                    .count() as u32
                            })
                            .unwrap_or(0);
                        clauses.saturating_sub(1)
                    }
                    _ => 0,
                };
            }
            "binary_operator" => {
                let op = operator_of(node, src).unwrap_or_default();
                if matches!(op.as_str(), "&&" | "||" | "and" | "or") {
                    n += 1;
                }
            }
            _ => {}
        }
        for c in children(node) {
            stack.push(c);
        }
    }
    n
}

// ---------------------------------------------------------------------------
// The defines/refs/writes scan (spec §3.6), Elixir form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): the
/// parameter patterns' bound names, the guard's reads, the
/// body's own scan, minus the defines from the refs.
fn fn_body_info(
    src: &[u8],
    params: &[Node],
    guard: Option<Node>,
    body_roots: &[Node],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
    for p in params {
        scan_pattern(*p, src, &mut defines, &mut refs, false, &mut writes);
    }
    if let Some(g) = guard {
        scan_expr(g, src, &mut defines, &mut refs, &mut writes);
    }
    for b in body_roots {
        scan_expr(*b, src, &mut defines, &mut refs, &mut writes);
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
    scan_expr(root, src, &mut defines, &mut refs, &mut writes);
    refs.into_iter().collect()
}

/// The last segment of an alias node's dotted name.
fn alias_last(node: Node, src: &[u8]) -> Option<String> {
    alias_segments(&txt(node, src)).last().cloned()
}

/// One walk computing defines/refs/writes for a body.
/// Iterative.
fn scan_expr(
    root: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
    writes: &mut HashSet<String>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "call" => {
                let Some(target) = node.child_by_field_name("target") else {
                    continue;
                };
                match target.kind() {
                    "dot" => {
                        // Only the receiver is a read.
                        if let Some(recv) = target.child_by_field_name("left") {
                            stack.push(recv);
                        }
                        if let Some(args) = child_kind(node, "arguments") {
                            for c in named_children_of(args) {
                                stack.push(c);
                            }
                        }
                        if let Some(b) = child_kind(node, "do_block") {
                            for c in children(b) {
                                stack.push(c);
                            }
                        }
                    }
                    "identifier" => {
                        let name = txt(target, src);
                        if DECL_CALLS.contains(&name.as_str()) {
                            // A nested declaration is a
                            // separate symbol (or an import,
                            // which reads nothing here).
                            continue;
                        }
                        if !CONTROL_CALLS.contains(&name.as_str()) {
                            // A bare local call's name is a
                            // ref (the Python rule).
                            refs.insert(name);
                        }
                        if let Some(args) = child_kind(node, "arguments") {
                            for c in named_children_of(args) {
                                stack.push(c);
                            }
                        }
                        if let Some(b) = child_kind(node, "do_block") {
                            for c in children(b) {
                                stack.push(c);
                            }
                        }
                    }
                    _ => {
                        for c in children(node) {
                            stack.push(c);
                        }
                    }
                }
            }
            "stab_clause" => {
                // The left is a pattern; the right (a `body`
                // node) is expressions.
                if let Some(l) = node.child_by_field_name("left") {
                    for c in named_children_of(l) {
                        scan_pattern(c, src, defines, refs, true, writes);
                    }
                }
                if let Some(r) = node.child_by_field_name("right") {
                    for c in children(r) {
                        stack.push(c);
                    }
                }
            }
            "binary_operator" => {
                let op = operator_of(node, src).unwrap_or_default();
                let left = node.child_by_field_name("left");
                let right = node.child_by_field_name("right");
                match op.as_str() {
                    "=" | "<-" => {
                        if let Some(l) = left {
                            scan_pattern(l, src, defines, refs, true, writes);
                        }
                        if let Some(r) = right {
                            stack.push(r);
                        }
                    }
                    _ => {
                        if let Some(r) = right {
                            stack.push(r);
                        }
                        if let Some(l) = left {
                            stack.push(l);
                        }
                    }
                }
            }
            "unary_operator" => {
                let op = operator_of(node, src).unwrap_or_default();
                if let Some(operand) = node.child_by_field_name("operand") {
                    if op == "@" {
                        match operand.kind() {
                            "identifier" => {
                                refs.insert(format!("@{}", txt(operand, src)));
                            }
                            "call" => {
                                if let Some(t) = operand.child_by_field_name("target") {
                                    if t.kind() == "identifier" {
                                        refs.insert(format!("@{}", txt(t, src)));
                                    }
                                }
                                if let Some(args) = child_kind(operand, "arguments") {
                                    for c in named_children_of(args) {
                                        stack.push(c);
                                    }
                                }
                            }
                            _ => stack.push(operand),
                        }
                    } else {
                        stack.push(operand);
                    }
                }
            }
            "map" => {
                for c in children(node) {
                    if c.kind() == "struct" {
                        for s in children(c) {
                            match s.kind() {
                                "alias" => {
                                    if let Some(l) = alias_last(s, src) {
                                        refs.insert(l);
                                    }
                                }
                                "identifier" if txt(s, src) != "__MODULE__" => {
                                    refs.insert(txt(s, src));
                                }
                                _ => {}
                            }
                        }
                    } else {
                        stack.push(c);
                    }
                }
            }
            "pair" => {
                // A keyword key is not a read; a computed key
                // is. The value is an expression.
                if let Some(k) = node.child_by_field_name("key") {
                    if k.kind() != "keyword" {
                        stack.push(k);
                    }
                }
                if let Some(v) = node.child_by_field_name("value") {
                    stack.push(v);
                }
            }
            "identifier" => {
                let name = txt(node, src);
                if name != "_" && name != "__MODULE__" {
                    refs.insert(name);
                }
            }
            "alias" => {
                if let Some(l) = alias_last(node, src) {
                    refs.insert(l);
                }
            }
            _ => {
                for c in children(node) {
                    stack.push(c);
                }
            }
        }
    }
}

/// One pattern walk: identifiers bind (defines — and
/// writes when `mark_writes`, a match/generator target);
/// struct module names and defaults are reads.
fn scan_pattern(
    root: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
    mark_writes: bool,
    writes: &mut HashSet<String>,
) {
    enum P<'t> {
        Pat(Node<'t>),
        Expr(Node<'t>),
    }
    let mut stack = vec![P::Pat(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            P::Expr(e) => {
                scan_expr(e, src, defines, refs, writes);
                continue;
            }
            P::Pat(n) => n,
        };
        match node.kind() {
            "identifier" => {
                let name = txt(node, src);
                if name != "_" && name != "__MODULE__" {
                    defines.insert(name.clone());
                    if mark_writes {
                        writes.insert(name);
                    }
                }
            }
            "alias" => {
                if let Some(l) = alias_last(node, src) {
                    refs.insert(l);
                }
            }
            "binary_operator" => {
                let op = operator_of(node, src).unwrap_or_default();
                let left = node.child_by_field_name("left");
                let right = node.child_by_field_name("right");
                match op.as_str() {
                    // A default: the right is an expression.
                    "\\\\" => {
                        if let Some(r) = right {
                            stack.push(P::Expr(r));
                        }
                        if let Some(l) = left {
                            stack.push(P::Pat(l));
                        }
                    }
                    // A guard on a clause pattern: the right
                    // is an expression.
                    "when" => {
                        if let Some(r) = right {
                            stack.push(P::Expr(r));
                        }
                        if let Some(l) = left {
                            stack.push(P::Pat(l));
                        }
                    }
                    _ => {
                        if let Some(r) = right {
                            stack.push(P::Pat(r));
                        }
                        if let Some(l) = left {
                            stack.push(P::Pat(l));
                        }
                    }
                }
            }
            "unary_operator" => {
                // The pin operator reads its operand; any
                // other unary in a pattern wraps a pattern.
                if let Some(operand) = node.child_by_field_name("operand") {
                    if operator_of(node, src).as_deref() == Some("^") {
                        stack.push(P::Expr(operand));
                    } else {
                        stack.push(P::Pat(operand));
                    }
                }
            }
            "map" => {
                for c in children(node) {
                    if c.kind() == "struct" {
                        for s in children(c) {
                            match s.kind() {
                                "alias" => {
                                    if let Some(l) = alias_last(s, src) {
                                        refs.insert(l);
                                    }
                                }
                                "identifier" if txt(s, src) != "__MODULE__" => {
                                    refs.insert(txt(s, src));
                                }
                                _ => {}
                            }
                        }
                    } else {
                        stack.push(P::Pat(c));
                    }
                }
            }
            "pair" => {
                if let Some(k) = node.child_by_field_name("key") {
                    if k.kind() != "keyword" {
                        stack.push(P::Expr(k));
                    }
                }
                if let Some(v) = node.child_by_field_name("value") {
                    stack.push(P::Pat(v));
                }
            }
            // Calls do not occur in patterns (guards are
            // handled at the `when` arm); anything else
            // (tuples, lists, keywords, literals) recurses
            // structurally.
            "call" => {}
            _ => {
                for c in children(node) {
                    stack.push(P::Pat(c));
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
        let f = one("Core/Engine.ex", "defmodule Engine do\nend\n");
        assert_eq!(f.module_id, "quarry::Core::Engine");
        let s = one("main.exs", "x = 1\n");
        assert_eq!(s.module_id, "quarry::main");
    }

    #[test]
    fn alias_import_require_use_are_imports() {
        let f = one(
            "Core/Engine.ex",
            "defmodule E do\n  alias Quarry.Shapes.Circle\n  alias Quarry.Core.Engine, as: Eng\n  import Quarry.Shapes.Circle, only: [area: 1]\n  require Logger\n  use GenServer\nend\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("quarry::Shapes::Circle", vec!["Circle".to_string()]),
                ("quarry::Core::Engine", vec!["Eng".to_string()]),
                ("quarry::Shapes::Circle", vec!["area".to_string()]),
                ("external::Logger", vec!["Logger".to_string()]),
                ("external::GenServer", vec![]),
            ]
        );
        assert!(f.calls.is_empty());
    }

    #[test]
    fn brace_alias_names_one_import_per_entry() {
        let f = one(
            "a.ex",
            "defmodule A do\n  alias Quarry.Shapes.{Circle, Square}\nend\n",
        );
        let rows: Vec<&str> = f.imports.iter().map(|i| i.to_module_id.as_str()).collect();
        assert_eq!(rows, vec!["quarry::Shapes::Circle", "quarry::Shapes::Square"]);
    }

    #[test]
    fn behaviour_and_defimpl_are_heritage() {
        let f = one(
            "Shapes/Circle.ex",
            "defmodule Quarry.Shapes.Circle do\n  @behaviour Quarry.Shapes.Shape\nend\n",
        );
        assert_eq!(f.heritage.len(), 1);
        assert_eq!(f.heritage[0].type_name, "Circle");
        assert_eq!(f.heritage[0].target_name, "Shape");
        assert_eq!(f.heritage[0].kind, EdgeKind::Implements);
        let g = one(
            "Shapes/Drawable.ex",
            "defimpl Quarry.Shapes.Drawable, for: Quarry.Shapes.Circle do\n  def draw(c), do: c\nend\n",
        );
        assert_eq!(g.heritage.len(), 1);
        assert_eq!(g.heritage[0].type_name, "Circle");
        assert_eq!(g.heritage[0].target_name, "Drawable");
        let draw = g.symbols.iter().find(|s| s.label == "draw").unwrap();
        assert_eq!(draw.kind, NodeKind::Method);
        assert_eq!(draw.defines, vec!["c".to_string()]);
    }

    #[test]
    fn defstruct_is_not_a_symbol_and_attributes_scan() {
        let f = one(
            "Shapes/Circle.ex",
            "defmodule Quarry.Shapes.Circle do\n  defstruct radius: 0.0\n  @pi 3.14\n  def area(%__MODULE__{radius: radius}) do\n    @pi * radius * radius\n  end\nend\n",
        );
        let labels: Vec<&str> = f.symbols.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["Circle", "area"]);
        let area = f.symbols.iter().find(|s| s.label == "area").unwrap();
        assert_eq!(area.defines, vec!["radius".to_string()]);
        assert_eq!(area.refs, vec!["@pi".to_string()]);
        assert!(area.writes.is_empty());
        assert!(f.calls.is_empty());
    }

    #[test]
    fn case_decisions_and_clause_binds() {
        let f = one(
            "a.ex",
            "defmodule A do\n  def classify(shape) do\n    case shape do\n      %Circle{radius: r} -> r\n      _ -> 0\n    end\n  end\nend\n",
        );
        let classify = f.symbols.iter().find(|s| s.label == "classify").unwrap();
        assert_eq!(classify.decisions, Some(1));
        assert_eq!(classify.defines, sv2(&["r", "shape"]));
        assert_eq!(classify.refs, sv2(&["Circle"]));
    }

    fn sv2(names: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("defmodule Deep do\n  def f(x) do\n    ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1\n  end\nend\n");
        let f = one("deep.ex", &src);
        assert_eq!(f.symbols.len(), 2);
    }
}
