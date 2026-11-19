// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! R extractor (`.R`, `.r`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, Java, Go, Shell, C++, Swift, and Ruby.
//!
//! Node kinds below were verified against
//! `tree-sitter-r-1.3.0/src/node-types.json` in the crates.io
//! registry cache (the pinned crate, never guessed): `program`,
//! `function_definition` (fields `parameters`, `body`),
//! `parameter` (fields `name`, `default`), `binary_operator`
//! (fields `lhs`, `operator`, `rhs` — assignments are binary
//! operators: `<-`, `<<-`, `=`, `->`, `->>`), `call` (fields
//! `function`, `arguments`), `argument` (fields `name`,
//! `value`), `extract_operator` (fields `lhs`, `operator`,
//! `rhs`), `namespace_operator` (fields `lhs`, `operator`,
//! `rhs`), `if_statement`, `for_statement` (field `variable`),
//! `while_statement`, `repeat_statement`, `identifier`,
//! `string`.
//!
//! The mapping follows the Python extractor's spec shape:
//! module ids are path-derived (the shared rule). Symbols
//! (§3.2 — in R a function definition is an assignment whose
//! right side is a `function(…)` expression: an assignment
//! binding a function is a Function symbol at any nesting
//! level, as Python's nested `def` is; an assignment binding
//! anything else is a Variable at the top level and a scan
//! define inside a function. R has no class-definition syntax
//! in this grammar — S3/S4/R6 types are built by calls — so
//! there are no Class symbols). Imports (calls again:
//! `library(x)` / `require(x)` load a package —
//! `external::{name}` with the name as binding; `source("p")`
//! includes a file, resolved relative to the sourcing file
//! like a C include. A call by any of those names with a
//! literal first argument is an import, not a call). Calls
//! (§3.4 — the called name: for `pkg::fn` the function name,
//! for `obj$fn` the member name). Heritage (none — no
//! inheritance syntax; the section is empty by construction).
//! The defines/refs/writes scan (§3.6's marimo discipline:
//! nested function definitions pruned — they are their own
//! symbols — reads that are also defines dropped, only the
//! left side of `$`/`@` and `::` is a read, an argument's name
//! is not a read); decisions (§3.2's enumeration, R form:
//! `if` +1, `for`/`while`/`repeat` +1, each `&&`/`||` +1).
//! The walk runs on an explicit heap stack (the E-1 lesson).

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

const ASSIGN_OPS: [&str; 5] = ["<-", "<<-", "=", "->", "->>"];

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_r::LANGUAGE.into())
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
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
    calls: Vec<RawCall>,
}

/// One step of the walker's explicit work stack (the E-1 lesson).
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
                    name,
                    node,
                    defines,
                    refs,
                    writes,
                    end_line,
                    signature,
                    decisions,
                } => {
                    self.push_symbol(
                        kind, &name, node, defines, refs, writes, end_line, signature, decisions,
                    );
                }
                Step::LeaveSymbol => self.pop_symbol(),
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
            "binary_operator" if is_assignment(node, self.src) => {
                self.handle_assignment(node, stack)
            }
            "call" => {
                if !self.handle_possible_import_call(node) {
                    self.record_call(node);
                }
                self.push_children(stack, node);
            }
            _ => self.push_children(stack, node),
        }
    }

    /// An assignment: a function value makes a Function symbol
    /// (any level); any other value makes a Variable at the top
    /// level only.
    fn handle_assignment<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let (target, value) = assignment_parts(node, self.src);
        let Some(target) = target else {
            self.push_children(stack, node);
            return;
        };
        let Some(value) = value else {
            self.push_children(stack, node);
            return;
        };
        if value.kind() == "function_definition" {
            if target.kind() == "identifier" {
                let name = self.text(target);
                self.schedule_function(node, value, name, stack);
                return;
            }
            // A function assigned into a structure is not a
            // named symbol; walk on.
            self.push_children(stack, node);
            return;
        }
        if self.enclosing.is_empty() && self.path.is_empty() && target.kind() == "identifier" {
            let name = self.text(target);
            let mut refs: Vec<String> = body_refs(value, self.src);
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
                signature_of(node, Some(value), self.src),
                None,
            );
            self.pop_symbol();
            stack.push(Step::Enter(value));
            return;
        }
        self.push_children(stack, node);
    }

    /// Schedule a function assignment: the symbol's node is the
    /// assignment (its signature is the assignment up to the
    /// body); the function body walks under it.
    fn schedule_function<'t>(
        &mut self,
        assign_node: Node<'t>,
        fn_node: Node<'t>,
        name: String,
        stack: &mut Vec<Step<'t>>,
    ) {
        let body = fn_node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(
            self.src,
            fn_node.child_by_field_name("parameters"),
            body,
        );
        let signature = signature_of(assign_node, body, self.src);
        let decisions = body.map(|b| count_decisions(b, self.src));
        stack.push(Step::LeaveSymbol);
        if let Some(b) = body {
            self.push_children(stack, b);
        }
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Function,
            name,
            node: assign_node,
            defines,
            refs,
            writes,
            end_line: Some(end_line_of(assign_node)),
            signature,
            decisions,
        });
    }

    fn current_scope(&self) -> String {
        self.enclosing
            .last()
            .cloned()
            .unwrap_or_else(|| self.module_id.clone())
    }

    /// One `call` → one RawCall; `to_name` is the called name
    /// (§3.4): the identifier, the right side of `::`, or the
    /// member name of `$`.
    fn record_call(&mut self, node: Node) {
        let Some(f) = node.child_by_field_name("function") else {
            return;
        };
        let t = match f.kind() {
            "identifier" => self.text(f),
            "namespace_operator" | "extract_operator" => f
                .child_by_field_name("rhs")
                .map(|r| self.text(r).trim_matches('"').to_string())
                .unwrap_or_default(),
            _ => self.text(f).trim().to_string(),
        };
        if !t.is_empty() {
            self.calls.push(RawCall {
                from_id: self.current_scope(),
                to_name: t,
                line: line_of(node),
            });
        }
    }

    /// If `node` is a `library` / `require` / `source` call
    /// with a literal first argument, record the import and
    /// return true.
    fn handle_possible_import_call(&mut self, node: Node) -> bool {
        let Some(f) = node.child_by_field_name("function") else {
            return false;
        };
        if f.kind() != "identifier" {
            return false;
        }
        let name = self.text(f);
        if !matches!(name.as_str(), "library" | "require" | "source") {
            return false;
        }
        let Some(args) = node.child_by_field_name("arguments") else {
            return false;
        };
        let Some(first) = children(args).into_iter().next() else {
            return false;
        };
        let value = first.child_by_field_name("value").unwrap_or(first);
        if name == "source" {
            if value.kind() != "string" {
                return false;
            }
            let Some(lit) = string_content(value, self.src) else {
                return false;
            };
            let to = self.resolve_relative(&lit);
            self.imports.push(RawImport {
                from_module_id: self.module_id.clone(),
                to_module_id: to,
                bindings: Vec::new(),
                line: line_of(node),
            });
            return true;
        }
        // library / require: the argument is a package name, as
        // an identifier or a string.
        let pkg_name = match value.kind() {
            "identifier" => self.text(value),
            "string" => match string_content(value, self.src) {
                Some(s) => s,
                None => return false,
            },
            _ => return false,
        };
        if pkg_name.is_empty() {
            return false;
        }
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: format!("external::{pkg_name}"),
            bindings: vec![pkg_name],
            line: line_of(node),
        });
        true
    }

    /// Resolve a source() path against the sourcing file's
    /// directory segments.
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
}

/// Is this binary_operator an assignment?
fn is_assignment(node: Node, src: &[u8]) -> bool {
    node.kind() == "binary_operator"
        && node
            .child_by_field_name("operator")
            .map(|o| ASSIGN_OPS.contains(&txt(o, src).as_str()))
            .unwrap_or(false)
}

/// The (target, value) of an assignment, honoring the reversed
/// `->` / `->>` forms.
fn assignment_parts<'t>(node: Node<'t>, src: &[u8]) -> (Option<Node<'t>>, Option<Node<'t>>) {
    let lhs = node.child_by_field_name("lhs");
    let rhs = node.child_by_field_name("rhs");
    let op = node
        .child_by_field_name("operator")
        .map(|o| txt(o, src))
        .unwrap_or_default();
    if matches!(op.as_str(), "->" | "->>") {
        (rhs, lhs)
    } else {
        (lhs, rhs)
    }
}

/// The literal content of a string node, if it has no
/// interpolation. (R string nodes carry `string_open` /
/// `string_close` children beside the content.)
fn string_content(node: Node, src: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut any = false;
    for c in children(node) {
        match c.kind() {
            "string_content" => {
                any = true;
                out.push_str(&txt(c, src));
            }
            "string_open" | "string_close" | "escape_sequence" => {}
            _ => return None,
        }
    }
    if any {
        Some(out)
    } else {
        None
    }
}

/// Decision points in a body (the R form of §3.2's
/// enumeration). Iterative (E-1); nested function definitions
/// are pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if node.kind() == "function_definition" {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "for_statement" | "while_statement" | "repeat_statement" => 1,
            "binary_operator" => {
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
// The defines/refs/writes scan (spec §3.6), R form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes): the
/// parameter names, the defaults' reads, the body's own scan,
/// minus the defines from the refs.
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
            if let Some(d) = p.child_by_field_name("default") {
                scan_w(d, src, &mut defines, &mut refs, &mut writes);
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
        ExtractLhs(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::ExtractLhs(e) => {
                // Only the left side of $ / @ / :: is a read.
                if let Some(l) = e.child_by_field_name("lhs") {
                    stack.push(S::N(l));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // A nested function definition is its own symbol.
            "function_definition" => {}
            "binary_operator" if is_assignment(node, src) => {
                let (target, value) = assignment_parts(node, src);
                if let Some(v) = value {
                    stack.push(S::N(v));
                }
                if let Some(t) = target {
                    match t.kind() {
                        "identifier" => {
                            let name = txt(t, src);
                            defines.insert(name.clone());
                            writes.insert(name);
                        }
                        // x$y <- … / x[[i]] <- …: the base is
                        // a read, not a define.
                        _ => stack.push(S::N(t)),
                    }
                }
            }
            "call" => {
                if let Some(f) = node.child_by_field_name("function") {
                    match f.kind() {
                        "identifier" => {
                            refs.insert(txt(f, src));
                        }
                        "namespace_operator" | "extract_operator" => {
                            stack.push(S::ExtractLhs(f));
                        }
                        _ => stack.push(S::N(f)),
                    }
                }
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "argument" => {
                // An argument's name is not a read; its value is.
                if let Some(v) = node.child_by_field_name("value") {
                    stack.push(S::N(v));
                }
            }
            "extract_operator" | "namespace_operator" => {
                stack.push(S::ExtractLhs(node));
            }
            "for_statement" => {
                if let Some(v) = node.child_by_field_name("variable") {
                    if v.kind() == "identifier" {
                        defines.insert(txt(v, src));
                    }
                }
                for c in children(node) {
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
        let f = one("lib/shapes.R", "PI <- 3.14\n");
        assert_eq!(f.module_id, "quarry::lib::shapes");
    }

    #[test]
    fn library_and_source_are_imports() {
        let f = one(
            "main.R",
            "library(stats)\nsource(\"core/engine.R\")\nprint(1)\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("external::stats", vec!["stats".to_string()]),
                ("quarry::core::engine", vec![]),
            ]
        );
        let calls: Vec<&str> = f.calls.iter().map(|c| c.to_name.as_str()).collect();
        assert_eq!(calls, vec!["print"]);
    }

    #[test]
    fn scan_extract_and_superassignment() {
        let f = one(
            "e.R",
            "render <- function(engine) {\n  count <<- count + 1\n  circle_area(engine$wheel)\n}\n",
        );
        let render = f.symbols.iter().find(|s| s.label == "render").unwrap();
        assert_eq!(render.defines, vec!["count".to_string(), "engine".to_string()]);
        assert_eq!(render.writes, vec!["count".to_string()]);
        assert_eq!(render.refs, vec!["circle_area".to_string()]);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("f <- function(x) {\n  ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1\n}\n");
        let f = one("deep.R", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
