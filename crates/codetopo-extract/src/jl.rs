// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Julia extractor (`.jl`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, Java, Go, Shell, C++, Swift, Ruby, and R.
//!
//! Node kinds below were verified against
//! `tree-sitter-julia-0.23.1/src/node-types.json` in the
//! crates.io registry cache and against parse trees of the
//! fixture (the pinned crate, never guessed). This grammar is
//! positional — most nodes carry no fields: `module_definition`
//! (field `name`, then statement children),
//! `struct_definition` / `abstract_definition` /
//! `primitive_definition` (a `type_head` child, then field
//! declarations), `function_definition` (a `signature` child,
//! then body statements), `signature` (a `call_expression`
//! head), `assignment` (children: lhs, `operator`, rhs — a
//! short-form function definition is an assignment whose lhs
//! is a call), `call_expression` (children: callee,
//! `argument_list`), `field_expression` (field `value`, then
//! the field's identifier), `compound_assignment_expression`,
//! `using_statement` / `import_statement` (`import_path`,
//! `selected_import`, `import_alias` children), `if_statement`,
//! `for_statement`, `while_statement`, `binary_expression`
//! (an `operator` child between its operands).
//!
//! The mapping follows the Python extractor's spec shape:
//! module ids are path-derived (the shared rule). Symbols
//! (§3.2 — a `module` block is a class-kind symbol, as in
//! Ruby; structs and primitive types are classes; abstract
//! types are interfaces, Julia's conformance analog; long-form
//! and short-form function definitions are functions, and a
//! definition on a qualified name (`Base.show(…)`) is a method
//! nested under the qualifier; top-level assignments are
//! variables; struct fields are not symbols). Imports
//! (`using` / `import`: a dotted path resolves on module-id
//! segments — a leading `.` starts from the file's directory
//! module and each extra dot ascends one segment; an absolute
//! path whose first segment is the package name is internal,
//! anything else is `external::{first segment}`; a selected
//! import binds the selected names). Calls (§3.4 — the callee
//! text as written). Heritage (a type head's `<:` supertype
//! is Inherits). The defines/refs/writes scan (§3.6's marimo
//! discipline: nested function definitions pruned, reads that
//! are also defines dropped, only the value of a field
//! expression is a read, a compound assignment's target is
//! read not written); decisions (§3.2's enumeration, Julia
//! form: `if` +1, each `elseif` +1, `for`/`while` +1, ternary
//! +1, each `&&`/`||` +1). The walk runs on an explicit heap
//! stack (the E-1 lesson).

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
        .set_language(&tree_sitter_julia::LANGUAGE.into())
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

/// A file's directory segments (its directory module).
fn dir_segments(rel_path: &Path) -> Vec<String> {
    let mut segs = Vec::new();
    if let Some(parent) = rel_path.parent() {
        for comp in parent.components() {
            if let std::path::Component::Normal(s) = comp {
                segs.push(s.to_string_lossy().into_owned());
            }
        }
    }
    segs
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
            "module_definition" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    // The signature is the header: cut at the
                    // first statement child.
                    let cut = children(node)
                        .into_iter()
                        .find(|c| c.id() != n.id());
                    self.schedule_scope_symbol(node, NodeKind::Class, vec![name], cut, stack);
                } else {
                    self.push_children(stack, node);
                }
            }
            "struct_definition" | "primitive_definition" => {
                self.schedule_type_def(node, NodeKind::Class, stack)
            }
            "abstract_definition" => {
                self.schedule_type_def(node, NodeKind::Interface, stack)
            }
            "function_definition" => self.schedule_long_function(node, stack),
            "assignment" => self.handle_assignment(node, stack),
            "using_statement" | "import_statement" => self.handle_import_stmt(node),
            "call_expression" => {
                self.record_call(node);
                self.push_children(stack, node);
            }
            "macrocall_expression" => {
                if let Some(first) = children(node).into_iter().next() {
                    if first.kind() == "macro_identifier" {
                        let t = self.text(first);
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

    /// Schedule a symbol whose body is the node's remaining
    /// children (a module block).
    fn schedule_scope_symbol<'t>(
        &mut self,
        node: Node<'t>,
        kind: NodeKind,
        names: Vec<String>,
        cut: Option<Node<'t>>,
        stack: &mut Vec<Step<'t>>,
    ) {
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        self.push_children(stack, node);
        stack.push(Step::OpenSymbol {
            kind,
            names,
            node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature: signature_of(node, cut, self.src),
            decisions: None,
            in_class: true,
        });
    }

    /// Schedule a struct / abstract / primitive definition: the
    /// name and optional supertype live in the `type_head`
    /// child; the remaining children are field declarations
    /// (not symbols) and method definitions.
    fn schedule_type_def<'t>(&mut self, node: Node<'t>, kind: NodeKind, stack: &mut Vec<Step<'t>>) {
        let head = children(node)
            .into_iter()
            .find(|c| c.kind() == "type_head");
        let Some(head) = head else {
            self.push_children(stack, node);
            return;
        };
        let (name, supertype) = type_head_parts(head, self.src);
        let Some(name) = name else {
            self.push_children(stack, node);
            return;
        };
        if let Some(super_name) = supertype {
            self.heritage.push(RawHeritage {
                type_name: name.clone(),
                target_name: super_name,
                kind: EdgeKind::Inherits,
                line: line_of(node),
            });
        }
        let signature = signature_of(node, Some(head), self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        self.push_children(stack, node);
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

    /// Schedule a long-form `function … end` definition.
    fn schedule_long_function<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some((names, params)) = signature_parts(node, self.src) else {
            self.push_children(stack, node);
            return;
        };
        let body_nodes: Vec<Node> = children(node)
            .into_iter()
            .filter(|c| c.kind() != "signature")
            .collect();
        let (defines, refs, writes) = fn_body_info(self.src, params, &body_nodes);
        let kind = if names.len() > 1 {
            NodeKind::Method
        } else {
            NodeKind::Function
        };
        let first_body = body_nodes.first().copied();
        let signature = signature_of(node, first_body, self.src);
        let decisions = Some(count_decisions_in(&body_nodes, self.src));
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        for b in body_nodes.iter().rev() {
            stack.push(Step::Enter(*b));
        }
        stack.push(Step::OpenSymbol {
            kind,
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

    /// An assignment: a call on the left is a short-form
    /// function definition (a symbol at any level); a plain
    /// identifier on the left is a Variable at the top level.
    fn handle_assignment<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let parts: Vec<Node> = children(node)
            .into_iter()
            .filter(|c| c.kind() != "operator")
            .collect();
        if parts.len() != 2 {
            self.push_children(stack, node);
            return;
        }
        let (lhs, rhs) = (parts[0], parts[1]);
        if lhs.kind() == "call_expression" {
            // Short-form definition.
            let Some((names, params)) = call_head_parts(lhs, self.src) else {
                self.push_children(stack, node);
                return;
            };
            let (defines, refs, writes) = fn_body_info(self.src, params, &[rhs]);
            let kind = if names.len() > 1 {
                NodeKind::Method
            } else {
                NodeKind::Function
            };
            let signature = signature_of(node, Some(rhs), self.src);
            let decisions = Some(count_decisions(rhs, self.src));
            stack.push(Step::LeaveSymbol {
                in_class: self.in_class,
            });
            stack.push(Step::Enter(rhs));
            stack.push(Step::OpenSymbol {
                kind,
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
            return;
        }
        if self.enclosing.is_empty() && self.path.is_empty() && lhs.kind() == "identifier" {
            let name = self.text(lhs);
            let mut refs: Vec<String> = body_refs(rhs, self.src);
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
                signature_of(node, Some(rhs), self.src),
                None,
            );
            self.pop_symbol();
            stack.push(Step::Enter(rhs));
            return;
        }
        self.push_children(stack, node);
    }

    /// A using/import statement → one RawImport per path it
    /// names.
    fn handle_import_stmt(&mut self, node: Node) {
        for c in children(node) {
            match c.kind() {
                "import_path" => {
                    let (to, _) = self.resolve_import_path(&self.text(c));
                    self.imports.push(RawImport {
                        from_module_id: self.module_id.clone(),
                        to_module_id: to,
                        bindings: Vec::new(),
                        line: line_of(node),
                    });
                }
                "selected_import" => {
                    let kids = children(c);
                    if let Some(path_node) = kids.first() {
                        let (to, _) = self.resolve_import_path(&self.text(*path_node));
                        let bindings: Vec<String> = kids
                            .iter()
                            .skip(1)
                            .filter(|k| k.kind() == "identifier")
                            .map(|k| self.text(*k))
                            .collect();
                        self.imports.push(RawImport {
                            from_module_id: self.module_id.clone(),
                            to_module_id: to,
                            bindings,
                            line: line_of(node),
                        });
                    }
                }
                "import_alias" => {
                    let kids = children(c);
                    if kids.len() >= 2 {
                        let (to, _) = self.resolve_import_path(&self.text(kids[0]));
                        self.imports.push(RawImport {
                            from_module_id: self.module_id.clone(),
                            to_module_id: to,
                            bindings: vec![self.text(kids[kids.len() - 1])],
                            line: line_of(node),
                        });
                    }
                }
                "identifier" | "scoped_identifier" => {
                    let (to, _) = self.resolve_import_path(&self.text(c));
                    self.imports.push(RawImport {
                        from_module_id: self.module_id.clone(),
                        to_module_id: to,
                        bindings: Vec::new(),
                        line: line_of(node),
                    });
                }
                _ => {}
            }
        }
    }

    /// Resolve a dotted import path: leading dots are relative
    /// to the file's directory module (one dot = the directory
    /// itself, each extra dot ascends); an absolute path whose
    /// first segment is the package name is internal, anything
    /// else is external.
    fn resolve_import_path(&self, raw: &str) -> (String, Vec<String>) {
        let raw = raw.trim();
        let dots = raw.chars().take_while(|c| *c == '.').count();
        let rest = raw[dots..].trim_start_matches('.');
        let segs: Vec<String> = rest
            .split('.')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        if dots == 0 {
            if segs.first().map(|f| f == &self.pkg).unwrap_or(false) {
                return (segs.join("::"), segs);
            }
            return (
                format!("external::{}", segs.first().cloned().unwrap_or_default()),
                segs,
            );
        }
        let mut base = dir_segments(&self.rel_path);
        for _ in 1..dots {
            if base.pop().is_none() {
                return ("external::unresolved".to_string(), segs);
            }
        }
        base.extend(segs.iter().cloned());
        if base.is_empty() {
            ("external::unresolved".to_string(), segs)
        } else {
            (format!("{}::{}", self.pkg, base.join("::")), segs)
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
}

/// The (name, supertype) of a type_head: identifiers in
/// reading order — the first is the type's name; if the head
/// contains a `<:` the last is the supertype.
fn type_head_parts(head: Node, src: &[u8]) -> (Option<String>, Option<String>) {
    let mut idents: Vec<String> = Vec::new();
    let mut has_subtype_op = false;
    let mut stack = vec![head];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "identifier" => idents.push(txt(n, src)),
            "operator" if txt(n, src) == "<:" => {
                has_subtype_op = true;
            }
            _ => {}
        }
        for c in children(n) {
            stack.push(c);
        }
    }
    idents.reverse();
    let name = idents.first().cloned();
    let supertype = if has_subtype_op && idents.len() > 1 {
        idents.last().cloned()
    } else {
        None
    };
    (name, supertype)
}

/// The (names, argument_list) of a function_definition's
/// signature.
fn signature_parts<'t>(node: Node<'t>, src: &[u8]) -> Option<(Vec<String>, Option<Node<'t>>)> {
    let sig = children(node)
        .into_iter()
        .find(|c| c.kind() == "signature")?;
    let call = find_first_call(sig)?;
    call_head_parts(call, src)
}

/// The first call_expression in a subtree (pre-order).
fn find_first_call(node: Node) -> Option<Node> {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "call_expression" {
            return Some(n);
        }
        for c in children(n).into_iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// The (names, argument_list) of a call head: a plain
/// identifier is one name; a field expression contributes its
/// qualifier segments plus the field name.
fn call_head_parts<'t>(call: Node<'t>, src: &[u8]) -> Option<(Vec<String>, Option<Node<'t>>)> {
    let kids = children(call);
    let head = kids.first()?;
    let args = kids
        .iter()
        .find(|c| c.kind() == "argument_list")
        .copied();
    let names = match head.kind() {
        "identifier" => vec![txt(*head, src)],
        "field_expression" => {
            let mut out = Vec::new();
            if let Some(v) = head.child_by_field_name("value") {
                out.push(txt(v, src));
            }
            for c in children(*head) {
                if c.kind() == "identifier" {
                    out.push(txt(c, src));
                }
            }
            out
        }
        _ => return None,
    };
    if names.is_empty() {
        None
    } else {
        Some((names, args))
    }
}

/// Decision points in a body (the Julia form of §3.2's
/// enumeration). Iterative (E-1); nested function definitions
/// are pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    count_decisions_in(&[body], src)
}

fn count_decisions_in(roots: &[Node], src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack: Vec<Node> = roots.to_vec();
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "function_definition") {
            continue;
        }
        // A nested short-form definition is a separate symbol.
        if node.kind() == "assignment" {
            let parts: Vec<Node> = children(node)
                .into_iter()
                .filter(|c| c.kind() != "operator")
                .collect();
            if parts.len() == 2 && parts[0].kind() == "call_expression" {
                continue;
            }
        }
        n += match node.kind() {
            "if_statement" | "elseif_clause" | "for_statement" | "while_statement"
            | "ternary_expression" => 1,
            "binary_expression" => {
                let op = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .find(|c| c.kind() == "operator")
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
// The defines/refs/writes scan (spec §3.6), Julia form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): the
/// parameter names and their types' names, the body's own scan
/// over the given body roots, minus the defines from the refs.
fn fn_body_info(
    src: &[u8],
    params_list: Option<Node>,
    body_roots: &[Node],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
    if let Some(pl) = params_list {
        for p in children(pl) {
            collect_param(p, src, &mut defines, &mut refs, &mut writes);
        }
    }
    for b in body_roots {
        scan_w(*b, src, &mut defines, &mut refs, &mut writes);
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

/// One parameter: a plain identifier binds; a typed parameter
/// binds its first identifier and reads its type; a keyword
/// parameter binds its name and reads its default; a splat
/// binds its inner name.
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
        "typed_expression" => {
            let kids = children(p);
            if let Some(first) = kids.first() {
                if first.kind() == "identifier" {
                    defines.insert(txt(*first, src));
                }
            }
            for t in kids.iter().skip(1) {
                collect_type_refs(*t, src, refs);
            }
        }
        "keyword_argument" => {
            let kids = children(p);
            if let Some(first) = kids.first() {
                if first.kind() == "identifier" {
                    defines.insert(txt(*first, src));
                }
            }
            for v in kids.iter().skip(1) {
                if v.kind() != "operator" {
                    scan_w(*v, src, defines, refs, writes);
                }
            }
        }
        "splat_expression" => {
            for c in children(p) {
                if c.kind() == "identifier" {
                    defines.insert(txt(c, src));
                }
            }
        }
        _ => {}
    }
}

/// Collect the names a type expression mentions as refs.
fn collect_type_refs(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind() == "identifier" {
            refs.insert(txt(n, src));
            continue;
        }
        if n.kind() == "field_expression" {
            // A qualified type (A.B): the last segment is the
            // type's name.
            let kids = children(n);
            if let Some(last) = kids.last() {
                if last.kind() == "identifier" {
                    refs.insert(txt(*last, src));
                }
            }
            continue;
        }
        for c in children(n) {
            stack.push(c);
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
            // Nested function definitions are their own symbols.
            "function_definition" => {}
            "assignment" => {
                let parts: Vec<Node> = children(node)
                    .into_iter()
                    .filter(|c| c.kind() != "operator")
                    .collect();
                if parts.len() == 2 {
                    let (lhs, rhs) = (parts[0], parts[1]);
                    if lhs.kind() == "call_expression" {
                        // A short-form definition: its own
                        // symbol, pruned here.
                        continue;
                    }
                    stack.push(S::N(rhs));
                    match lhs.kind() {
                        "identifier" => {
                            let name = txt(lhs, src);
                            defines.insert(name.clone());
                            writes.insert(name);
                        }
                        "open_tuple" | "tuple_expression" => {
                            for e in children(lhs) {
                                if e.kind() == "identifier" {
                                    let name = txt(e, src);
                                    defines.insert(name.clone());
                                    writes.insert(name);
                                } else {
                                    stack.push(S::N(e));
                                }
                            }
                        }
                        _ => stack.push(S::N(lhs)),
                    }
                } else {
                    for c in children(node) {
                        stack.push(S::N(c));
                    }
                }
            }
            "compound_assignment_expression" => {
                // The target is read, not written.
                for c in children(node) {
                    if c.kind() != "operator" {
                        stack.push(S::N(c));
                    }
                }
            }
            "field_expression" => stack.push(S::FieldValue(node)),
            "call_expression" => {
                let kids = children(node);
                if let Some(callee) = kids.first() {
                    if callee.kind() == "field_expression" {
                        stack.push(S::FieldValue(*callee));
                    } else {
                        stack.push(S::N(*callee));
                    }
                }
                for c in kids.iter().skip(1) {
                    stack.push(S::N(*c));
                }
            }
            "for_statement" => {
                for c in children(node) {
                    if c.kind() == "for_binding" {
                        let kids = children(c);
                        if let Some(first) = kids.first() {
                            if first.kind() == "identifier" {
                                defines.insert(txt(*first, src));
                            }
                        }
                        for v in kids.iter().skip(1) {
                            if v.kind() != "operator" {
                                stack.push(S::N(*v));
                            }
                        }
                    } else {
                        stack.push(S::N(c));
                    }
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
        let f = one("Core/Engine.jl", "module Engine\nend\n");
        assert_eq!(f.module_id, "quarry::Core::Engine");
    }

    #[test]
    fn struct_supertype_is_heritage() {
        let f = one(
            "S.jl",
            "abstract type Shape end\nstruct Circle <: Shape\n    radius::Float64\nend\n",
        );
        assert_eq!(f.heritage.len(), 1);
        assert_eq!(f.heritage[0].type_name, "Circle");
        assert_eq!(f.heritage[0].target_name, "Shape");
        let kinds: Vec<(&str, NodeKind)> =
            f.symbols.iter().map(|s| (s.label.as_str(), s.kind)).collect();
        assert!(kinds.contains(&("Shape", NodeKind::Interface)));
        assert!(kinds.contains(&("Circle", NodeKind::Class)));
    }

    #[test]
    fn using_selected_and_relative() {
        let f = one(
            "Core/Engine.jl",
            "using ..Shapes: Circle\nusing LinearAlgebra\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("quarry::Shapes", vec!["Circle".to_string()]),
                ("external::LinearAlgebra", vec![]),
            ]
        );
    }

    #[test]
    fn short_form_definition_scans() {
        let f = one(
            "a.jl",
            "area(c::Circle) = 3.14 * c.radius * c.radius\n",
        );
        let area = f.symbols.iter().find(|s| s.label == "area").unwrap();
        assert_eq!(area.kind, NodeKind::Function);
        assert_eq!(area.defines, vec!["c".to_string()]);
        assert_eq!(area.refs, vec!["Circle".to_string()]);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("f(x) = ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1\n");
        let f = one("deep.jl", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
