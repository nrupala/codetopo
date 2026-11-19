// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Ruby extractor (`.rb`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, Java, Go, Shell, C++, and Swift.
//!
//! Node kinds below were verified against
//! `tree-sitter-ruby-0.23.1/src/node-types.json` in the
//! crates.io registry cache and against parse trees of the
//! fixture (the pinned crate, never guessed): `program`,
//! `module` / `class` (fields `name`, `body`; `class` also
//! `superclass`), `method` (fields `name`, `parameters`,
//! `body`), `singleton_method` (fields `object`, `name`,
//! `parameters`, `body`), `singleton_class` (field `body`),
//! `assignment` (fields `left`, `right`),
//! `operator_assignment` (fields `left`, `operator`, `right`),
//! `call` (fields `receiver`, `method`, `arguments`, `block`),
//! `scope_resolution` (fields `scope`, `name`), `constant`,
//! `identifier`, `instance_variable`, `class_variable`,
//! `global_variable`, `if`, `elsif`, `case`, `when`, `while`,
//! `until`, `for`, `binary`, `method_parameters`,
//! `block_parameters`.
//!
//! The mapping follows the Python extractor's spec shape:
//! module ids are path-derived (the shared rule). Symbols
//! (§3.2 — modules and classes are both class-kind symbols: a
//! Ruby module is a named container of methods and constants,
//! and the schema has no namespace kind; their names are
//! symbol-path segments. Methods inside a type are methods,
//! outside are functions; a singleton method (`def self.x`,
//! `def Foo.x`) is a method, nested under its object when the
//! object is named. Top-level assignments are variables;
//! class-level assignments are fields, not symbols — the
//! Python precedent). Imports (Ruby's import mechanism is the
//! `require` / `require_relative` call: a call by either name
//! whose first argument is a string literal is an import,
//! recorded as an import, not a call; `require_relative`
//! resolves relative to the requiring file like a C include,
//! `require` resolves the same way when its first segment is
//! the package name and is `external::{first segment}`
//! otherwise; bindings are empty). Calls (§3.4 — the called
//! method's name; the receiver is not part of it, the Python
//! attribute rule). Heritage (a class's superclass is
//! Inherits). The defines/refs/writes scan (§3.6's marimo
//! discipline: nested declarations pruned, reads that are also
//! defines dropped, augmented-assignment targets read not
//! written, only a call's receiver is a read — a bare call's
//! name is a ref, as a plain-name call is in Python; constants
//! and instance/class/global variables are refs and writes
//! under their full written names, sigils included); decisions
//! (§3.2's enumeration, Ruby form: `if` +1, each `elsif` +1,
//! `while`/`until`/`for` +1, `case` contributes whens − 1,
//! each `&&`/`||`/`and`/`or` +1). The walk runs on an explicit
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
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
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
        /// more for a scope-resolution name (`Foo::Bar`) or a
        /// singleton method on a named object.
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
    /// Open a scope that is not a symbol (`class << self`):
    /// sets the class-scope flag, pushes no path segment.
    OpenClassScope,
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
                Step::OpenClassScope => {
                    self.in_class = true;
                }
                Step::LeaveScope { in_class } => {
                    self.in_class = in_class;
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
            "module" | "class" => self.schedule_type(node, stack),
            "method" => self.schedule_method(node, stack),
            "singleton_method" => self.schedule_singleton_method(node, stack),
            "singleton_class" => {
                stack.push(Step::LeaveScope {
                    in_class: self.in_class,
                });
                if let Some(b) = node.child_by_field_name("body") {
                    self.push_children(stack, b);
                }
                stack.push(Step::OpenClassScope);
            }
            "assignment" if self.enclosing.is_empty() && self.path.is_empty() => {
                // A top-level assignment to a plain name is a
                // variable.
                if let Some(left) = node.child_by_field_name("left") {
                    if matches!(left.kind(), "identifier" | "constant") {
                        let name = self.text(left);
                        let value = node.child_by_field_name("right");
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
                    } else {
                        self.push_children(stack, node);
                    }
                }
            }
            "call" => {
                if !self.handle_possible_require(node) {
                    self.record_call(node);
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

    /// One `call` → one RawCall; `to_name` is the method's name
    /// (§3.4 — the receiver is not part of it).
    fn record_call(&mut self, node: Node) {
        if let Some(m) = node.child_by_field_name("method") {
            let t = self.text(m).trim().to_string();
            if !t.is_empty() {
                self.calls.push(RawCall {
                    from_id: self.current_scope(),
                    to_name: t,
                    line: line_of(node),
                });
            }
        }
    }

    /// If `node` is a `require` / `require_relative` call with a
    /// string-literal first argument, record the import and
    /// return true.
    fn handle_possible_require(&mut self, node: Node) -> bool {
        let Some(m) = node.child_by_field_name("method") else {
            return false;
        };
        let name = self.text(m);
        if name != "require" && name != "require_relative" {
            return false;
        }
        let Some(args) = node.child_by_field_name("arguments") else {
            return false;
        };
        let Some(first) = children(args).into_iter().next() else {
            return false;
        };
        if first.kind() != "string" {
            return false;
        }
        let mut lit = String::new();
        for c in children(first) {
            if c.kind() != "string_content" {
                return false;
            }
            lit.push_str(&self.text(c));
        }
        let to = if name == "require_relative" {
            self.resolve_relative(&lit)
        } else {
            let segs: Vec<&str> = lit.split('/').collect();
            if segs.first().map(|f| *f == self.pkg).unwrap_or(false) {
                self.module_path_of(&lit)
            } else {
                format!("external::{}", segs.first().copied().unwrap_or(""))
            }
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings: Vec::new(),
            line: line_of(node),
        });
        true
    }

    /// Resolve a require_relative path against the requiring
    /// file's directory segments.
    fn resolve_relative(&self, include: &str) -> String {
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

    /// A require path as a module id under the package.
    fn module_path_of(&self, path: &str) -> String {
        let mut segs: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty() && *s != ".")
            .map(|s| s.to_string())
            .collect();
        if let Some(last) = segs.last_mut() {
            if let Some((stem, _)) = last.rsplit_once('.') {
                *last = stem.to_string();
            }
        }
        segs.join("::")
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

    /// Schedule a module or class: a Class symbol; a class's
    /// superclass is an Inherits row.
    fn schedule_type<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let segments = constant_segments(name_node, self.src);
        if segments.is_empty() {
            self.push_children(stack, node);
            return;
        }
        let label = segments.last().cloned().unwrap_or_default();
        if node.kind() == "class" {
            if let Some(sc) = node.child_by_field_name("superclass") {
                for c in children(sc) {
                    let target = constant_segments(c, self.src);
                    if let Some(t) = target.last() {
                        self.heritage.push(RawHeritage {
                            type_name: label.clone(),
                            target_name: t.clone(),
                            kind: EdgeKind::Inherits,
                            line: line_of(sc),
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
            kind: NodeKind::Class,
            names: segments,
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

    /// Schedule a method definition.
    fn schedule_method<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(
            self.src,
            node.child_by_field_name("parameters"),
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

    /// Schedule a singleton method: `def self.x` nests under the
    /// current scope; `def Foo.x` nests under Foo's segments.
    fn schedule_singleton_method<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let mut names = vec![];
        if let Some(obj) = node.child_by_field_name("object") {
            if obj.kind() != "self" {
                names.extend(constant_segments(obj, self.src));
            }
        }
        names.push(name);
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(
            self.src,
            node.child_by_field_name("parameters"),
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
}

/// The segments a constant reference denotes: a plain constant
/// is one segment; a scope resolution contributes its scope
/// segments plus its name.
fn constant_segments(node: Node, src: &[u8]) -> Vec<String> {
    match node.kind() {
        "constant" | "identifier" => vec![txt(node, src)],
        "scope_resolution" => {
            let mut out = Vec::new();
            if let Some(scope) = node.child_by_field_name("scope") {
                out.extend(constant_segments(scope, src));
            }
            if let Some(name) = node.child_by_field_name("name") {
                out.push(txt(name, src));
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Decision points in a body (the Ruby form of §3.2's
/// enumeration). Iterative (E-1); nested declarations pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "method" | "singleton_method" | "class" | "module") {
            continue;
        }
        n += match node.kind() {
            "if" | "elsif" | "while" | "until" | "for" => 1,
            "case" => {
                let whens = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .filter(|c| c.kind() == "when")
                    .count() as u32;
                whens.saturating_sub(1)
            }
            "binary" => {
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

// ---------------------------------------------------------------------------
// The defines/refs/writes scan (spec §3.6), Ruby form
// ---------------------------------------------------------------------------

/// Compute one method's (defines, refs, writes): the parameter
/// names, the defaults' reads, the body's own scan, minus the
/// defines from the refs.
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
            collect_param(p, src, &mut defines, &mut refs, &mut writes);
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

/// One parameter: its bound names are defines; a default value
/// is scanned for reads.
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
        "keyword_parameter" | "optional_parameter" | "splat_parameter"
        | "hash_splat_parameter" | "block_parameter" => {
            let kids = children(p);
            if let Some(first) = kids.first() {
                if first.kind() == "identifier" {
                    defines.insert(txt(*first, src));
                }
            }
            for v in kids.iter().skip(1) {
                scan_w(*v, src, defines, refs, writes);
            }
        }
        "destructured_parameter" => {
            for c in children(p) {
                collect_param(c, src, defines, refs, writes);
            }
        }
        _ => {}
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
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            // Nested declarations are separate symbols.
            "method" | "singleton_method" | "class" | "module" | "singleton_class" => {}
            "assignment" => {
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(r);
                }
                if let Some(l) = node.child_by_field_name("left") {
                    scan_target(l, src, defines, writes, &mut stack);
                }
            }
            "operator_assignment" => {
                if let Some(r) = node.child_by_field_name("right") {
                    stack.push(r);
                }
                if let Some(l) = node.child_by_field_name("left") {
                    // An augmented target is read, not written.
                    match l.kind() {
                        "identifier" | "instance_variable" | "class_variable"
                        | "global_variable" | "constant" => {
                            refs.insert(txt(l, src));
                        }
                        "call" => {
                            if let Some(recv) = l.child_by_field_name("receiver") {
                                stack.push(recv);
                            }
                        }
                        _ => stack.push(l),
                    }
                }
            }
            "call" => {
                // Only the receiver is a read; a bare call's
                // name is a ref (the Python plain-name rule).
                if let Some(recv) = node.child_by_field_name("receiver") {
                    stack.push(recv);
                } else if let Some(m) = node.child_by_field_name("method") {
                    if m.kind() == "identifier" {
                        refs.insert(txt(m, src));
                    }
                }
                for c in children(node) {
                    if matches!(c.kind(), "argument_list" | "block" | "do_block") {
                        stack.push(c);
                    }
                }
            }
            "scope_resolution" => {
                // Only the scope is a read (the navigation rule).
                if let Some(s) = node.child_by_field_name("scope") {
                    stack.push(s);
                }
            }
            "block_parameters" => {
                for c in children(node) {
                    collect_param(c, src, defines, refs, writes);
                }
            }
            "for" => {
                if let Some(p) = node.child_by_field_name("pattern") {
                    scan_target(p, src, defines, writes, &mut stack);
                }
                for c in children(node) {
                    stack.push(c);
                }
            }
            "identifier" | "instance_variable" | "class_variable" | "global_variable"
            | "constant" => {
                refs.insert(txt(node, src));
            }
            _ => {
                for c in children(node) {
                    stack.push(c);
                }
            }
        }
    }
}

/// An assignment target in the scan: a plain name or variable
/// defines and writes; a constant writes; an attribute target
/// writes the attribute name; anything else is scanned as an
/// expression.
fn scan_target<'t>(
    l: Node<'t>,
    src: &[u8],
    defines: &mut HashSet<String>,
    writes: &mut HashSet<String>,
    stack: &mut Vec<Node<'t>>,
) {
    match l.kind() {
        "identifier" => {
            let name = txt(l, src);
            defines.insert(name.clone());
            writes.insert(name);
        }
        "instance_variable" | "class_variable" | "global_variable" | "constant" => {
            writes.insert(txt(l, src));
        }
        "call" => {
            // obj.attr = …: writes the attribute name (a
            // trailing `=` of the writer method is not part of
            // the attribute's name); the receiver is a read.
            if let Some(m) = l.child_by_field_name("method") {
                let name = txt(m, src);
                writes.insert(name.trim_end_matches('=').to_string());
            }
            if let Some(recv) = l.child_by_field_name("receiver") {
                stack.push(recv);
            }
        }
        "left_assignment_list" => {
            for c in children(l) {
                scan_target(c, src, defines, writes, stack);
            }
        }
        _ => stack.push(l),
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
        let f = one("lib/circle.rb", "module Quarry\nend\n");
        assert_eq!(f.module_id, "quarry::lib::circle");
    }

    #[test]
    fn require_relative_is_an_import_not_a_call() {
        let f = one(
            "core/engine.rb",
            "require_relative \"../lib/circle\"\nrequire \"json\"\n",
        );
        let rows: Vec<&str> = f.imports.iter().map(|i| i.to_module_id.as_str()).collect();
        assert_eq!(rows, vec!["quarry::lib::circle", "external::json"]);
        assert!(f.calls.is_empty());
    }

    #[test]
    fn superclass_is_heritage() {
        let f = one("c.rb", "class Circle < Shape\nend\n");
        assert_eq!(f.heritage.len(), 1);
        assert_eq!(f.heritage[0].type_name, "Circle");
        assert_eq!(f.heritage[0].target_name, "Shape");
    }

    #[test]
    fn scan_instance_variables_and_params() {
        let f = one(
            "e.rb",
            "class E\n  def render(wheel)\n    @renders += 1\n    wheel.area\n  end\nend\n",
        );
        let render = f.symbols.iter().find(|s| s.label == "render").unwrap();
        assert_eq!(render.defines, vec!["wheel".to_string()]);
        assert_eq!(render.refs, vec!["@renders".to_string()]);
        assert!(render.writes.is_empty());
    }

    #[test]
    fn singleton_methods_nest() {
        let f = one(
            "s.rb",
            "class S\n  def self.build\n    new\n  end\nend\ndef Quarry.make\nend\n",
        );
        let ids: Vec<&str> = f.symbols.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"quarry::s::S::build"), "{ids:?}");
        assert!(ids.contains(&"quarry::s::Quarry::make"), "{ids:?}");
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("def f(x)\n  ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1\nend\n");
        let f = one("deep.rb", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
