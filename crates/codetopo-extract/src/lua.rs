// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Lua extractor (`.lua`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, Java, Go, Shell, C++, Swift, Ruby, R,
//! Julia, C#, PHP, Scala, Haskell, and Elixir.
//!
//! Node kinds below were verified against
//! `tree-sitter-lua-0.5.0/src/node-types.json` in the crates.io
//! registry cache and against parse trees of the fixture (the
//! pinned crate, never guessed): `chunk`,
//! `function_declaration` (fields `name`, `parameters`,
//! `body`; the name is an `identifier`, a
//! `dot_index_expression`, or a `method_index_expression`, and
//! a `local function` is the same node with a `local` token),
//! `function_definition` (fields `parameters`, `body` — the
//! anonymous form), `variable_declaration` (a `local`/`global`
//! wrapper holding either an `assignment_statement` or a bare
//! `variable_list`), `assignment_statement` (children
//! `variable_list`, `expression_list`), `variable_list` (field
//! `name`, repeated), `expression_list` (field `value`,
//! repeated), `function_call` (fields `name`, `arguments`),
//! `dot_index_expression` (fields `table`, `field`),
//! `method_index_expression` (fields `table`, `method`),
//! `bracket_index_expression` (fields `table`, `field`),
//! `table_constructor` (`field` children: fields `name`,
//! `value`), `parameters` (field `name`, repeated),
//! `if_statement` / `elseif_statement`, `for_statement`
//! (`for_numeric_clause` / `for_generic_clause`),
//! `while_statement`, `repeat_statement`,
//! `binary_expression` (fields `left`, `operator`, `right`).
//!
//! The mapping follows the Julia extractor's spec shape:
//! module ids are path-derived (the shared rule). Symbols
//! (§3.2 — Lua has no class syntax, so a table is the type
//! construction: a top-level name bound directly to a table
//! constructor is a class; a definition on a dotted or
//! method name (`function T.name`, `function T:name`, and
//! the assignment forms `T.name = function …`) is a method
//! nested under its table chain; a plain function definition
//! is a function at any nesting level; any other top-level
//! plain-name binding is a variable; table fields and
//! top-level field assignments are not symbols; a top-level
//! name bound to a `setmetatable(…)` call is a variable, not
//! a class — only a direct table literal is a class).
//! Imports (`require("a.b")` with a string-literal argument
//! is an import, not a call — the R rule; Lua resolves it
//! against `package.path` from the package root, so the
//! dotted path is package-root-relative under the package
//! name; the import binds the name the call is assigned to;
//! a `require` with a non-literal argument is an ordinary
//! call). Calls (§3.4 — the callee text as written: a method
//! call keeps its colon form, a dotted call its dots).
//! Heritage (Lua has no inheritance syntax; exactly one
//! metatable form is counted — `setmetatable(T, { __index =
//! U })` with plain names `T`/`U`, plus the same call bound
//! to a name (`local Sub = setmetatable({}, { __index =
//! U })`) — is Inherits; `T.__index = T` and instance
//! construction `setmetatable({}, T)` are NOT heritage).
//! The defines/refs/writes scan (§3.6's marimo discipline:
//! nested function definitions pruned, reads that are also
//! defines dropped, only the table root of an index
//! expression is a read, a field assignment writes the
//! field name); decisions (§3.2's enumeration, Lua form:
//! `if` +1, each `elseif` +1, `for`/`while`/`repeat` +1,
//! each `and`/`or` +1). Anonymous function values are not
//! symbols: the walk descends into them in the enclosing
//! scope, the scan prunes them. The walk runs on an
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
        .set_language(&tree_sitter_lua::LANGUAGE.into())
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
    },
    LeaveSymbol,
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
                } => {
                    self.push_symbol(
                        kind, &names, node, defines, refs, writes, end_line, signature, decisions,
                    );
                }
                Step::LeaveSymbol => {
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
            "function_declaration" => self.schedule_function_declaration(node, stack),
            "variable_declaration" => self.handle_declaration(node, node, stack),
            "assignment_statement" => self.handle_declaration(node, node, stack),
            "function_call" => {
                self.handle_call(node, stack);
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

    /// One `function_call` → one RawCall (or one RawImport for
    /// a literal `require`); `to_name` is the callee's text as
    /// written (§3.4). A counted `setmetatable` heritage form
    /// is recorded alongside the call.
    fn handle_call<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        if let Some(target) = require_target(node, self.src) {
            self.imports.push(RawImport {
                from_module_id: self.module_id.clone(),
                to_module_id: self.resolve_require(&target),
                bindings: Vec::new(),
                line: line_of(node),
            });
            return;
        }
        if let Some(name_node) = node.child_by_field_name("name") {
            let t = self.text(name_node).trim().to_string();
            if !t.is_empty() {
                self.calls.push(RawCall {
                    from_id: self.current_scope(),
                    to_name: t,
                    line: line_of(node),
                });
            }
            // Heritage form H1: setmetatable(T, {__index = U}).
            if name_node.kind() == "identifier" && self.text(name_node) == "setmetatable" {
                if let Some((t_name, u_name)) = setmetatable_heritage(node, None, self.src) {
                    self.heritage.push(RawHeritage {
                        type_name: t_name,
                        target_name: u_name,
                        kind: EdgeKind::Inherits,
                        line: line_of(node),
                    });
                }
            }
        }
        self.push_children(stack, node);
    }

    /// Resolve a `require` path: package-root-relative (Lua's
    /// `package.path` semantics) — the segments join under the
    /// package name; a path already starting with the package
    /// name is used as written.
    fn resolve_require(&self, raw: &str) -> String {
        let segs: Vec<&str> = raw
            .split(['.', '/'])
            .filter(|s| !s.is_empty())
            .collect();
        match segs.first() {
            Some(first) if *first == self.pkg => segs.join("::"),
            Some(_) => format!("{}::{}", self.pkg, segs.join("::")),
            None => "external::unresolved".to_string(),
        }
    }

    /// Schedule a `function_declaration`: plain names are
    /// functions, dotted/method names are methods nested under
    /// their table chain.
    fn schedule_function_declaration<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let Some(names) = name_chain(name_node, self.src) else {
            self.push_children(stack, node);
            return;
        };
        self.schedule_function(node, names, node, stack);
    }

    /// Shared scheduling for every function-shaped symbol:
    /// the symbol's scan and measured fields are computed now;
    /// the stack then replays the recursive order — the symbol
    /// opens, the body walks, the scope closes.
    fn schedule_function<'t>(
        &mut self,
        sym_node: Node<'t>,
        names: Vec<String>,
        fn_node: Node<'t>,
        stack: &mut Vec<Step<'t>>,
    ) {
        let kind = if names.len() > 1 {
            NodeKind::Method
        } else {
            NodeKind::Function
        };
        let params = fn_node.child_by_field_name("parameters");
        let body = fn_node.child_by_field_name("body");
        let body_roots: Vec<Node> = body.into_iter().collect();
        let (defines, refs, writes) = fn_body_info(self.src, params, &body_roots);
        let decisions = Some(count_decisions_in(&body_roots, self.src));
        let signature = signature_of(sym_node, body, self.src);
        stack.push(Step::LeaveSymbol);
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind,
            names,
            node: sym_node,
            defines,
            refs,
            writes,
            end_line: Some(end_line_of(sym_node)),
            signature,
            decisions,
        });
    }

    /// A `variable_declaration` or a standalone
    /// `assignment_statement`: the binding forms that carry
    /// symbols, imports, and heritage. `stmt` is the statement
    /// node symbols are attributed to (for a declaration, the
    /// wrapper; its inner assignment is unwrapped here and
    /// never visited separately).
    fn handle_declaration<'t>(
        &mut self,
        stmt: Node<'t>,
        node: Node<'t>,
        stack: &mut Vec<Step<'t>>,
    ) {
        let inner = if node.kind() == "variable_declaration" {
            children(node)
                .into_iter()
                .find(|c| c.kind() == "assignment_statement")
        } else {
            None
        };
        let holder = inner.unwrap_or(node);
        let var_list = children(holder)
            .into_iter()
            .find(|c| c.kind() == "variable_list");
        let expr_list = children(holder)
            .into_iter()
            .find(|c| c.kind() == "expression_list");
        let Some(var_list) = var_list else {
            self.push_children(stack, node);
            return;
        };
        let names: Vec<Node> = children(var_list);
        let values: Vec<Node> = expr_list.map(|e| children(e)).unwrap_or_default();

        // Single binding of an anonymous function: a function
        // or method symbol at any nesting level.
        if names.len() == 1 && values.len() == 1 && values[0].kind() == "function_definition" {
            if let Some(chain) = target_chain(names[0], self.src) {
                self.schedule_function(stmt, chain, values[0], stack);
                return;
            }
        }
        // Single binding of a literal require: an import that
        // binds the assigned name. The binding is still a
        // symbol (below); the call itself is consumed here so
        // the walk does not record it a second time.
        let mut consumed_value = false;
        if names.len() == 1 && values.len() == 1 && values[0].kind() == "function_call" {
            if let Some(target) = require_target(values[0], self.src) {
                let binding = target_chain(names[0], self.src)
                    .and_then(|c| c.last().cloned())
                    .unwrap_or_default();
                self.imports.push(RawImport {
                    from_module_id: self.module_id.clone(),
                    to_module_id: self.resolve_require(&target),
                    bindings: if binding.is_empty() {
                        Vec::new()
                    } else {
                        vec![binding]
                    },
                    line: line_of(stmt),
                });
                consumed_value = true;
            }
        }
        // Heritage form H2: a name bound to
        // setmetatable(…, {__index = U}).
        if names.len() == 1 && values.len() == 1 && values[0].kind() == "function_call" {
            if let Some(chain) = target_chain(names[0], self.src) {
                if chain.len() == 1 {
                    if let Some((_, u_name)) =
                        setmetatable_heritage(values[0], Some(&chain[0]), self.src)
                    {
                        self.heritage.push(RawHeritage {
                            type_name: chain[0].clone(),
                            target_name: u_name,
                            kind: EdgeKind::Inherits,
                            line: line_of(values[0]),
                        });
                    }
                }
            }
        }

        // Top-level plain-name bindings are symbols: a direct
        // table constructor is a class, anything else a
        // variable. Nested bindings are not symbols.
        if self.enclosing.is_empty() {
            for (i, name_node) in names.iter().enumerate() {
                if name_node.kind() != "identifier" {
                    continue;
                }
                let name = self.text(*name_node);
                let value = values.get(i).copied();
                let kind = match value {
                    Some(v) if v.kind() == "table_constructor" => NodeKind::Class,
                    _ => NodeKind::Variable,
                };
                let refs = value.map(|v| body_refs(v, self.src)).unwrap_or_default();
                let cut = expr_list;
                self.push_symbol(
                    kind,
                    &[name],
                    stmt,
                    Vec::new(),
                    refs,
                    Vec::new(),
                    Some(end_line_of(stmt)),
                    signature_of(stmt, cut, self.src),
                    None,
                );
                self.pop_symbol();
            }
        }
        // Initializer expressions walk in the enclosing scope:
        // their calls and bare requires belong to it. A consumed
        // require call is not walked (it is the import above).
        if !consumed_value {
            for v in values.into_iter().rev() {
                stack.push(Step::Enter(v));
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
}

/// The name chain of a definition name or assignment target:
/// a plain identifier is one segment; a dot/method index
/// contributes its table chain plus its field/method name.
/// A bracket index in the chain yields `None` (no symbol).
fn name_chain(node: Node, src: &[u8]) -> Option<Vec<String>> {
    match node.kind() {
        "identifier" => Some(vec![txt(node, src)]),
        "dot_index_expression" => {
            let mut segs = name_chain(node.child_by_field_name("table")?, src)?;
            segs.push(txt(node.child_by_field_name("field")?, src));
            Some(segs)
        }
        "method_index_expression" => {
            let mut segs = name_chain(node.child_by_field_name("table")?, src)?;
            segs.push(txt(node.child_by_field_name("method")?, src));
            Some(segs)
        }
        _ => None,
    }
}

/// The chain of an assignment target (same shapes as a
/// definition name).
fn target_chain(node: Node, src: &[u8]) -> Option<Vec<String>> {
    name_chain(node, src)
}

/// The string-literal target of a `require(…)` call, when
/// the call is exactly that: callee the plain identifier
/// `require`, first argument a string literal.
fn require_target(call: Node, src: &[u8]) -> Option<String> {
    let name = call.child_by_field_name("name")?;
    if name.kind() != "identifier" || txt(name, src) != "require" {
        return None;
    }
    let args = call.child_by_field_name("arguments")?;
    let first = children(args).into_iter().next()?;
    if first.kind() != "string" {
        return None;
    }
    string_content(first, src)
}

/// A string node's literal content (its `string_content`
/// child), when it has one.
fn string_content(node: Node, src: &[u8]) -> Option<String> {
    children(node)
        .into_iter()
        .find(|c| c.kind() == "string_content")
        .map(|c| txt(c, src))
}

/// The heritage a `setmetatable` call records, if any:
/// `(type_name, target_name)`. With `bound = None` (form
/// H1) the first argument must be the plain type name; with
/// `bound = Some(name)` (form H2, the call is bound to
/// `name`) the first argument is the fresh table and the
/// bound name is the type. In both forms the second
/// argument must be a table constructor whose `__index`
/// field is a plain name different from the type name.
fn setmetatable_heritage(
    call: Node,
    bound: Option<&str>,
    src: &[u8],
) -> Option<(String, String)> {
    let name = call.child_by_field_name("name")?;
    if name.kind() != "identifier" || txt(name, src) != "setmetatable" {
        return None;
    }
    let args = call.child_by_field_name("arguments")?;
    let argv: Vec<Node> = children(args);
    if argv.len() < 2 {
        return None;
    }
    let type_name = match bound {
        Some(b) => b.to_string(),
        None => {
            if argv[0].kind() != "identifier" {
                return None;
            }
            txt(argv[0], src)
        }
    };
    let meta = argv[1];
    if meta.kind() != "table_constructor" {
        return None;
    }
    for f in children(meta) {
        if f.kind() != "field" {
            continue;
        }
        let Some(fname) = f.child_by_field_name("name") else {
            continue;
        };
        let is_index = match fname.kind() {
            "identifier" => txt(fname, src) == "__index",
            "string" => string_content(fname, src).as_deref() == Some("__index"),
            _ => false,
        };
        if !is_index {
            continue;
        }
        let value = f.child_by_field_name("value")?;
        if value.kind() != "identifier" {
            return None;
        }
        let target = txt(value, src);
        if target == type_name {
            return None;
        }
        return Some((type_name, target));
    }
    None
}

/// Decision points in a body (the Lua form of §3.2's
/// enumeration). Iterative (E-1); nested function definitions
/// are pruned.
fn count_decisions_in(roots: &[Node], src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack: Vec<Node> = roots.to_vec();
    while let Some(node) = stack.pop() {
        if matches!(node.kind(), "function_declaration" | "function_definition") {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "elseif_statement" | "for_statement" | "while_statement"
            | "repeat_statement" => 1,
            "binary_expression" => node
                .child_by_field_name("operator")
                .map(|o| txt(o, src))
                .filter(|o| matches!(o.as_str(), "and" | "or"))
                .map(|_| 1)
                .unwrap_or(0),
            _ => 0,
        };
        for c in children(node) {
            stack.push(c);
        }
    }
    n
}

// ---------------------------------------------------------------------------
// The defines/refs/writes scan (spec §3.6), Lua form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): the
/// parameter names, the body's own scan over the given body
/// roots, minus the defines from the refs.
fn fn_body_info(
    src: &[u8],
    params: Option<Node>,
    body_roots: &[Node],
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
    if let Some(pl) = params {
        for p in children(pl) {
            if p.kind() == "identifier" {
                defines.insert(txt(p, src));
            }
        }
    }
    for b in body_roots {
        scan_w(*b, src, &mut defines, &mut refs, &mut writes);
    }
    finalize(defines, refs, writes)
}

/// The reads of one expression (a top-level variable's
/// initializer refs), with locally bound names filtered out.
fn body_refs(root: Node, src: &[u8]) -> Vec<String> {
    let mut defines = HashSet::new();
    let mut refs = HashSet::new();
    let mut writes = HashSet::new();
    scan_w(root, src, &mut defines, &mut refs, &mut writes);
    let (_, refs, _) = finalize(defines, refs, writes);
    refs
}

fn finalize(
    defines: HashSet<String>,
    refs: HashSet<String>,
    writes: HashSet<String>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut d: Vec<String> = defines.iter().cloned().collect();
    d.sort();
    let mut r: Vec<String> = refs
        .into_iter()
        .filter(|x| !defines.contains(x))
        .collect();
    r.sort();
    let mut w: Vec<String> = writes.into_iter().collect();
    w.sort();
    (d, r, w)
}

/// One walk computing defines/refs/writes for a body.
/// Iterative (E-1); every accumulator is a set.
fn scan_w(
    root: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
    writes: &mut HashSet<String>,
) {
    enum S<'t> {
        N(Node<'t>),
        /// Only the table root of an index expression is a read.
        IndexRoot(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::IndexRoot(ix) => {
                if let Some(t) = ix.child_by_field_name("table") {
                    stack.push(S::N(t));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested function definitions are their own symbols.
            "function_declaration" | "function_definition" => {}
            "identifier" => {
                refs.insert(txt(node, src));
            }
            "dot_index_expression" | "method_index_expression"
            | "bracket_index_expression" => stack.push(S::IndexRoot(node)),
            "variable_declaration" | "assignment_statement" => {
                // Plain names define and write; index targets
                // write their field name and read their table
                // root; values are scanned.
                let holder = if node.kind() == "variable_declaration" {
                    children(node)
                        .into_iter()
                        .find(|c| c.kind() == "assignment_statement")
                        .unwrap_or(node)
                } else {
                    node
                };
                for part in children(holder) {
                    match part.kind() {
                        "variable_list" => {
                            for target in children(part) {
                                match target.kind() {
                                    "identifier" => {
                                        let n = txt(target, src);
                                        defines.insert(n.clone());
                                        writes.insert(n);
                                    }
                                    "dot_index_expression" => {
                                        if let Some(f) =
                                            target.child_by_field_name("field")
                                        {
                                            writes.insert(txt(f, src));
                                        }
                                        stack.push(S::IndexRoot(target));
                                    }
                                    "bracket_index_expression" => {
                                        if let Some(f) =
                                            target.child_by_field_name("field")
                                        {
                                            if f.kind() == "string" {
                                                if let Some(c) =
                                                    string_content(f, src)
                                                {
                                                    writes.insert(c);
                                                }
                                            } else {
                                                stack.push(S::N(f));
                                            }
                                        }
                                        stack.push(S::IndexRoot(target));
                                    }
                                    _ => stack.push(S::IndexRoot(target)),
                                }
                            }
                        }
                        "expression_list" => {
                            for v in children(part) {
                                stack.push(S::N(v));
                            }
                        }
                        _ => {}
                    }
                }
            }
            "function_call" => {
                if let Some(name) = node.child_by_field_name("name") {
                    match name.kind() {
                        "dot_index_expression" | "method_index_expression"
                        | "bracket_index_expression" => stack.push(S::IndexRoot(name)),
                        _ => stack.push(S::N(name)),
                    }
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    for c in children(args) {
                        stack.push(S::N(c));
                    }
                }
            }
            "for_statement" => {
                for c in children(node) {
                    match c.kind() {
                        "for_numeric_clause" => {
                            if let Some(n) = c.child_by_field_name("name") {
                                if n.kind() == "identifier" {
                                    defines.insert(txt(n, src));
                                }
                            }
                            for part in ["start", "end", "step"] {
                                if let Some(v) = c.child_by_field_name(part) {
                                    stack.push(S::N(v));
                                }
                            }
                        }
                        "for_generic_clause" => {
                            for part in children(c) {
                                if part.kind() == "variable_list" {
                                    for vn in children(part) {
                                        if vn.kind() == "identifier" {
                                            defines.insert(txt(vn, src));
                                        }
                                    }
                                } else {
                                    stack.push(S::N(part));
                                }
                            }
                        }
                        _ => stack.push(S::N(c)),
                    }
                }
            }
            "table_constructor" => {
                // Field names are not reads; field values are.
                for f in children(node) {
                    if f.kind() == "field" {
                        if let Some(v) = f.child_by_field_name("value") {
                            stack.push(S::N(v));
                        }
                    } else {
                        stack.push(S::N(f));
                    }
                }
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
        let f = one("core/engine.lua", "local x = 1\n");
        assert_eq!(f.module_id, "quarry::core::engine");
    }

    #[test]
    fn setmetatable_index_is_heritage() {
        let f = one(
            "shapes.lua",
            "local Shape = {}\nlocal Circle = {}\nsetmetatable(Circle, { __index = Shape })\n",
        );
        assert_eq!(f.heritage.len(), 1);
        assert_eq!(f.heritage[0].type_name, "Circle");
        assert_eq!(f.heritage[0].target_name, "Shape");
        let kinds: Vec<(&str, NodeKind)> =
            f.symbols.iter().map(|s| (s.label.as_str(), s.kind)).collect();
        assert!(kinds.contains(&("Shape", NodeKind::Class)));
        assert!(kinds.contains(&("Circle", NodeKind::Class)));
    }

    #[test]
    fn require_binds_and_resolves_from_the_root() {
        let f = one(
            "core/engine.lua",
            "local shapes = require(\"shapes\")\nrequire(\"socket\")\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("quarry::shapes", vec!["shapes".to_string()]),
                ("quarry::socket", vec![]),
            ]
        );
        assert!(f.calls.is_empty());
    }

    #[test]
    fn method_definition_scans() {
        let f = one(
            "a.lua",
            "local T = {}\nfunction T:area()\n  return self.radius * 2\nend\n",
        );
        let area = f.symbols.iter().find(|s| s.label == "area").unwrap();
        assert_eq!(area.kind, NodeKind::Method);
        assert_eq!(area.id, "quarry::a::T::area");
        assert_eq!(area.refs, vec!["self".to_string()]);
        assert_eq!(area.decisions, Some(0));
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let chain = vec!["\"a\""; 50_000].join(" .. ");
        let src = format!("local s = {chain}\n");
        let f = one("deep.lua", &src);
        assert_eq!(f.symbols.len(), 1);
        assert_eq!(f.symbols[0].label, "s");
    }
}
