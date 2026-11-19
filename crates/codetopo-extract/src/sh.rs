// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Shell extractor (`.sh`, `.bash`) — beside Rust,
//! TypeScript/JavaScript, Python, Kotlin, C, Java, and Go.
//!
//! Node kinds below were verified against
//! `tree-sitter-bash-0.25.1/src/node-types.json` in the crates.io
//! registry cache and against parse trees of the fixture (the
//! pinned crate, never guessed): `program`,
//! `function_definition` (fields `name`, `body`), `command`
//! (fields `name`, `argument`), `command_name`,
//! `variable_assignment` (fields `name`, `value`),
//! `declaration_command`, `simple_expansion`, `expansion`,
//! `variable_name`, `special_variable_name`, `if_statement`,
//! `elif_clause`, `for_statement` (fields `variable`, `value`,
//! `body`), `while_statement`, `until_statement`,
//! `case_statement`, `case_item`, `test_command`,
//! `command_substitution`, `list`, `binary_expression`.
//!
//! The mapping follows the Python extractor's spec shape:
//! module ids are path-derived (the shared rule); symbols
//! (§3.2 — functions are functions, top-level variable
//! assignments are variables; shell has no types); imports
//! (shell's import mechanism is the `source` / `.` command: a
//! command by that name whose first argument is a literal path
//! is an import — resolved on module-id segments relative to
//! the sourcing file, like a C include — recorded as an import,
//! not a call; bindings are always empty); calls (§3.4 — every
//! other command is a call, `to_name` the command name as
//! written); heritage (none — shell has no type system; the
//! section is empty by construction); the defines/refs/writes
//! scan (§3.6's marimo discipline: nested function definitions
//! pruned, reads that are also defines dropped, a plain
//! assignment both defines and writes, a command's name is a
//! ref as a called name is in Python, expansions of named
//! variables are refs while positional and special parameters
//! ($1, $@, $?, …) are not); decisions (§3.2's enumeration,
//! shell form: `if` +1, each `elif` +1, `for`/`while`/`until`
//! +1, `case` contributes items − 1, each `&&`/`||` in a list
//! or expression +1, arithmetic ternary +1). The walk runs on
//! an explicit heap stack (the E-1 lesson).

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
        .set_language(&tree_sitter_bash::LANGUAGE.into())
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
            "function_definition" => self.schedule_function(node, stack),
            "variable_assignment" if self.enclosing.is_empty() => {
                if let Some(name) = assignment_name(node, self.src) {
                    let init = node.child_by_field_name("value");
                    let mut refs: Vec<String> = init
                        .map(|v| body_refs(v, self.src))
                        .unwrap_or_default();
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
            "declaration_command" if self.enclosing.is_empty() => {
                for c in children(node) {
                    if c.kind() == "variable_assignment" {
                        stack.push(Step::Enter(c));
                    }
                }
            }
            "command" => {
                if self.handle_possible_source(node) {
                    // An import, not a call — but its arguments
                    // still walk (expansions inside belong to the
                    // enclosing scope).
                    self.push_children(stack, node);
                } else {
                    self.record_call(node);
                    self.push_children(stack, node);
                }
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

    /// One `command` → one RawCall; `to_name` is the command
    /// name as written (§3.4).
    fn record_call(&mut self, node: Node) {
        if let Some(name) = command_name_text(node, self.src) {
            if !name.is_empty() {
                self.calls.push(RawCall {
                    from_id: self.current_scope(),
                    to_name: name,
                    line: line_of(node),
                });
            }
        }
    }

    /// If `node` is a `source` / `.` command with a literal path
    /// argument, record the import and return true.
    fn handle_possible_source(&mut self, node: Node) -> bool {
        let Some(name) = command_name_text(node, self.src) else {
            return false;
        };
        if name != "source" && name != "." {
            return false;
        }
        let mut cursor = node.walk();
        let first_arg = node
            .children_by_field_name("argument", &mut cursor)
            .next();
        let Some(arg) = first_arg else { return false };
        let Some(lit) = literal_text(arg, self.src) else {
            return false;
        };
        let to = self.resolve_source_path(&lit);
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings: Vec::new(),
            line: line_of(node),
        });
        true
    }

    /// Resolve a sourced path against the sourcing file's
    /// directory segments (the C include rule).
    fn resolve_source_path(&self, include: &str) -> String {
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

    /// Schedule a function definition: scan computed now; the
    /// stack replays the body walk under the opened symbol.
    fn schedule_function<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let body = node.child_by_field_name("body");
        let (defines, refs, writes) = fn_body_info(self.src, body);
        let signature = signature_of(node, body, self.src);
        let decisions = body.map(|b| count_decisions(b, self.src));
        stack.push(Step::LeaveSymbol);
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
        });
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

/// The command name of a `command` node: the text of its
/// `command_name` child (a word, a quoted string, or a
/// concatenation of literals).
fn command_name_text(node: Node, src: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    literal_text(name_node, src).or_else(|| Some(txt(name_node, src).trim().to_string()))
}

/// The literal text of a word / string / raw string node, if it
/// contains no expansions.
fn literal_text(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "word" => Some(txt(node, src)),
        "raw_string" => Some(txt(node, src).trim_matches('\'').to_string()),
        "string" => {
            let mut out = String::new();
            for c in children(node) {
                if c.kind() != "string_content" {
                    return None;
                }
                out.push_str(&txt(c, src));
            }
            Some(out)
        }
        "command_name" => {
            let parts: Vec<String> = children(node)
                .iter()
                .filter_map(|c| literal_text(*c, src))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.concat())
            }
        }
        "concatenation" => {
            let mut out = String::new();
            for c in children(node) {
                out.push_str(&literal_text(c, src)?);
            }
            Some(out)
        }
        _ => None,
    }
}

/// The name a variable_assignment binds: the variable_name, or
/// the base name of a subscript target.
fn assignment_name(node: Node, src: &[u8]) -> Option<String> {
    let name_node = node.child_by_field_name("name")?;
    match name_node.kind() {
        "variable_name" => Some(txt(name_node, src)),
        "subscript" => name_node
            .child_by_field_name("name")
            .map(|n| txt(n, src))
            .or_else(|| {
                (0..name_node.named_child_count())
                    .filter_map(|i| name_node.named_child(i))
                    .find(|c| c.kind() == "variable_name")
                    .map(|n| txt(n, src))
            }),
        _ => None,
    }
}

/// Decision points in a body (the shell form of §3.2's
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
            "if_statement" | "elif_clause" | "for_statement" | "while_statement"
            | "until_statement" | "ternary_expression" => 1,
            "case_statement" => {
                let items = (0..node.named_child_count())
                    .filter_map(|i| node.named_child(i))
                    .filter(|c| c.kind() == "case_item")
                    .count() as u32;
                items.saturating_sub(1)
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
            "list" => {
                // `a && b || c`: the operators are unnamed
                // children of the list node.
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .filter(|c| matches!(c.kind(), "&&" | "||"))
                    .count() as u32
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
// The defines/refs/writes scan (spec §3.6), shell form
// ---------------------------------------------------------------------------

/// Compute one function's (defines, refs, writes) from its body.
fn fn_body_info(src: &[u8], body: Option<Node>) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = HashSet::new();
    let mut writes: HashSet<String> = HashSet::new();
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

/// Positional and special parameters ($1, $@, $?, $$, $#, $*,
/// $!, $-, $0) are not named refs.
fn is_special_param(name: &str) -> bool {
    name.chars().all(|c| c.is_ascii_digit()) && !name.is_empty()
        || matches!(name, "@" | "*" | "#" | "?" | "$" | "!" | "-")
}

/// Record an assignment target in the scan: a plain name both
/// defines and writes; a subscript target writes its base name.
fn scan_assignment_target(
    node: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    writes: &mut HashSet<String>,
) {
    if let Some(name_node) = node.child_by_field_name("name") {
        match name_node.kind() {
            "variable_name" => {
                let name = txt(name_node, src);
                defines.insert(name.clone());
                writes.insert(name);
            }
            "subscript" => {
                if let Some(base) = assignment_name(node, src) {
                    writes.insert(base);
                }
            }
            _ => {}
        }
    }
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
            // Nested function definitions are separate symbols.
            "function_definition" => {}
            "variable_assignment" => {
                scan_assignment_target(node, src, defines, writes);
                if let Some(v) = node.child_by_field_name("value") {
                    stack.push(v);
                }
            }
            "declaration_command" => {
                let cmd = children(node)
                    .first()
                    .map(|c| txt(*c, src))
                    .unwrap_or_default();
                for c in children(node) {
                    match c.kind() {
                        "variable_assignment" => stack.push(c),
                        "variable_name" => {
                            let name = txt(c, src);
                            if cmd == "export" {
                                refs.insert(name);
                            } else {
                                // local / declare / typeset /
                                // readonly declare the name.
                                defines.insert(name);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "for_statement" => {
                if let Some(v) = node.child_by_field_name("variable") {
                    defines.insert(txt(v, src));
                }
                for c in children(node) {
                    if c.kind() != "variable_name" {
                        stack.push(c);
                    }
                }
            }
            "command" => {
                // The command's name is a ref (the called-name
                // rule); source commands are imports, not refs.
                if let Some(name) = command_name_text(node, src) {
                    if name != "source" && name != "." {
                        refs.insert(name);
                    }
                }
                for c in children(node) {
                    if c.kind() != "command_name" {
                        stack.push(c);
                    }
                }
            }
            "simple_expansion" => {
                for c in children(node) {
                    if c.kind() == "variable_name" {
                        let name = txt(c, src);
                        if !is_special_param(&name) {
                            refs.insert(name);
                        }
                    }
                }
            }
            "variable_name" => {
                let name = txt(node, src);
                if !is_special_param(&name) {
                    refs.insert(name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn one(rel: &str, src: &str) -> FileSymbols {
        extract("quarry", &PathBuf::from(rel), src).unwrap()
    }

    #[test]
    fn module_ids_follow_the_path_rule() {
        let f = one("lib/util.sh", "#!/bin/sh\nX=1\n");
        assert_eq!(f.module_id, "quarry::lib::util");
    }

    #[test]
    fn source_commands_are_imports_not_calls() {
        let f = one(
            "core/engine.sh",
            "#!/bin/sh\n. \"../lib/util.sh\"\nsource lib/more.sh\nrun_it\n",
        );
        let tos: Vec<&str> = f.imports.iter().map(|i| i.to_module_id.as_str()).collect();
        assert_eq!(tos, vec!["quarry::lib::util", "quarry::core::lib::more"]);
        let calls: Vec<&str> = f.calls.iter().map(|c| c.to_name.as_str()).collect();
        assert_eq!(calls, vec!["run_it"]);
    }

    #[test]
    fn scan_locals_expansions_and_positionals() {
        let f = one(
            "f.sh",
            "#!/bin/sh\nf() {\n    local size=\"$1\"\n    total=$((total + size))\n    echo \"$total\"\n}\n",
        );
        let f_sym = f.symbols.iter().find(|s| s.label == "f").unwrap();
        assert_eq!(f_sym.defines, vec!["size".to_string(), "total".to_string()]);
        assert_eq!(f_sym.writes, vec!["size".to_string(), "total".to_string()]);
        assert_eq!(f_sym.refs, vec!["echo".to_string()]);
    }

    #[test]
    fn decisions_count_ifs_loops_cases_and_lists() {
        let f = one(
            "d.sh",
            "#!/bin/sh\nd() {\n    if a; then b; elif c; then d; fi\n    for x in y; do e; done\n    case \"$x\" in\n        a) f ;;\n        b) g ;;\n        *) h ;;\n    esac\n    p && q || r\n}\n",
        );
        let d = f.symbols.iter().find(|s| s.label == "d").unwrap();
        // if 1 + elif 1 + for 1 + case (3 items - 1) 2 + list && || 2 = 7
        assert_eq!(d.decisions, Some(7));
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("#!/bin/sh\nf() {\n    x=$((x");
        for _ in 0..50_000 {
            src.push_str(" + x");
        }
        src.push_str("))\n}\n");
        let f = one("deep.sh", &src);
        assert_eq!(f.symbols.len(), 1);
    }
}
