// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Python extractor (`.py`, `.pyi`) — the third extractor, beside Rust
//! and TypeScript/JavaScript.
//!
//! Node kinds below were verified against
//! `tree-sitter-python-0.25.0/src/node-types.json` in the crates.io
//! registry cache (the pinned crate, never guessed): `module`,
//! `function_definition`, `class_definition`, `decorated_definition`,
//! `import_statement`, `import_from_statement` (fields `module_name`,
//! `name`), `call` (fields `function`, `arguments`), `assignment`
//! (fields `left`, `right`, `type`), `parameters`, `aliased_import`
//! (fields `name`, `alias`), `attribute` (fields `object`,
//! `attribute`), `block`, `except_clause` (fields `value`, `alias`),
//! `type_alias_statement` (fields `left`, `right`). One delta against
//! the spec pass's research copy of the grammar: `tuple_expression`
//! exists there but not in the pinned 0.25.0; nothing in the mapping
//! depends on it.
//!
//! The mapping is specified in
//! `~/workspace/codetopo-work/python-extractor/SPEC.md` §3: module ids
//! (§3.1, the `__init__` collapse mirrors the TS `index` rule), symbols
//! (§3.2), and the defines/refs/writes scan (§3.6, the marimo
//! discipline shared with the TS/Rust walkers — nested declarations
//! pruned, reads that are also defines dropped). Imports, calls and
//! heritage (§3.3–§3.5) and the measured fields (§3.7) land in the
//! following parts of the series.

use std::collections::HashSet;
use std::path::Path;

use codetopo_core::emit::{FileSymbols, RawCall, RawHeritage, RawImport, RawSymbol};
use codetopo_core::schema::NodeKind;

use crate::{display_path, line_of, text_of, ExtractError};

type Node<'a> = tree_sitter::Node<'a>;

/// Node text as an owned string (empty on invalid UTF-8, which the
/// walk tolerates the way the TS/Rust walkers tolerate partial parses).
fn txt(node: Node, src: &[u8]) -> String {
    text_of(node, src).unwrap_or("").to_string()
}

pub(crate) fn extract(package: &str, rel_path: &Path, source: &str) -> Result<FileSymbols, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
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

/// `__init__.py` collapses to its package path; other files append
/// their stem (spec §3.1). `.pyi` files follow the same rule.
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
    if stem != "__init__" {
        parts.push(stem);
    }
    parts.join("::")
}

struct Walker<'a> {
    src: &'a [u8],
    module_id: String,
    /// Symbol-name path segments: class → method → nested fn → …
    path: Vec<String>,
    /// Enclosing symbol ids.
    enclosing: Vec<String>,
    /// True while walking a class body directly (defs become methods).
    in_class: bool,
    symbols: Vec<RawSymbol>,
    imports: Vec<RawImport>,
    calls: Vec<RawCall>,
    heritage: Vec<RawHeritage>,
}

impl<'a> Walker<'a> {
    fn text(&self, n: Node) -> String {
        txt(n, self.src)
    }

    fn walk_children(&mut self, node: Node) {
        for i in 0..node.named_child_count() {
            if let Some(c) = node.named_child(i) {
                self.walk(c);
            }
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => self.handle_function(node),
            "class_definition" => self.handle_class(node),
            "decorated_definition" => {
                // The symbol is the inner definition (spec D3: line and
                // span come from it); the decorator's own calls/refs are
                // recorded by the edges pass against the enclosing scope.
                if let Some(def) = node.child_by_field_name("definition") {
                    match def.kind() {
                        "function_definition" => self.handle_function(def),
                        "class_definition" => self.handle_class(def),
                        _ => self.walk_children(node),
                    }
                }
            }
            "type_alias_statement" => self.handle_type_alias(node),
            "assignment" if self.enclosing.is_empty() => self.handle_module_assignment(node),
            _ => self.walk_children(node),
        }
    }

    /// Push a symbol record; the caller walks the body for nested
    /// symbols between `push_symbol` and `pop_symbol`.
    fn push_symbol(
        &mut self,
        kind: NodeKind,
        name: &str,
        node: Node,
        defines: Vec<String>,
        refs: Vec<String>,
        writes: Vec<String>,
    ) {
        self.path.push(name.to_string());
        let id = format!("{}::{}", self.module_id, self.path.join("::"));
        self.symbols.push(RawSymbol {
            id: id.clone(),
            kind,
            label: name.to_string(),
            line: line_of(node),
            column: None,
            end_line: None,
            signature: None,
            decisions: None,
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

    fn handle_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let kind = if self.in_class {
            NodeKind::Method
        } else {
            NodeKind::Function
        };
        let (defines, mut refs, writes) = fn_body_info(
            self.src,
            node.child_by_field_name("parameters"),
            node.child_by_field_name("body"),
        );
        // Return annotation names are refs of the function (spec D5).
        if let Some(rt) = node.child_by_field_name("return_type") {
            let mut set: HashSet<String> = refs.iter().cloned().collect();
            scan_type_refs(rt, self.src, &mut set);
            refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
            refs.sort();
        }
        self.push_symbol(kind, &name, node, defines, refs, writes);
        let prev_class = self.in_class;
        self.in_class = false;
        if let Some(body) = node.child_by_field_name("body") {
            self.walk_children(body);
        }
        self.in_class = prev_class;
        self.pop_symbol();
    }

    fn handle_class(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let body = node.child_by_field_name("body");
        let refs = body.map(|b| body_refs(b, self.src)).unwrap_or_default();
        self.push_symbol(NodeKind::Class, &name, node, Vec::new(), refs, Vec::new());
        let prev_class = self.in_class;
        self.in_class = true;
        if let Some(b) = body {
            self.walk_children(b);
        }
        self.in_class = prev_class;
        self.pop_symbol();
    }

    /// PEP 695 `type X = …` — an alias is a class-kind symbol (spec §3.2).
    fn handle_type_alias(&mut self, node: Node) {
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        let name = self.text(left);
        let mut set = HashSet::new();
        if let Some(right) = node.child_by_field_name("right") {
            scan_type_refs(right, self.src, &mut set);
        }
        let mut refs: Vec<String> = set.into_iter().collect();
        refs.sort();
        self.push_symbol(NodeKind::Class, &name, node, Vec::new(), refs, Vec::new());
        self.pop_symbol();
    }

    /// Module-level plain-name assignments are `variable` symbols
    /// (spec §3.2). Chained `a = b = …` yields one symbol per name;
    /// pattern/attribute/subscript targets yield none (the TS
    /// destructuring precedent).
    fn handle_module_assignment(&mut self, mut node: Node) {
        loop {
            let left = node.child_by_field_name("left");
            let right = node.child_by_field_name("right");
            if let Some(l) = left {
                if l.kind() == "identifier" {
                    let name = self.text(l);
                    let mut set: HashSet<String> = HashSet::new();
                    if let Some(r) = right {
                        if r.kind() != "assignment" {
                            set.extend(body_refs(r, self.src));
                        }
                    }
                    if let Some(t) = node.child_by_field_name("type") {
                        scan_type_refs(t, self.src, &mut set);
                    }
                    let mut refs: Vec<String> = set.into_iter().collect();
                    refs.sort();
                    self.push_symbol(
                        NodeKind::Variable,
                        &name,
                        node,
                        Vec::new(),
                        refs,
                        Vec::new(),
                    );
                    self.pop_symbol();
                }
            }
            match right {
                Some(r) if r.kind() == "assignment" => node = r,
                _ => break,
            }
        }
    }
}

/// Accumulators for the defines/refs/writes scan.
#[derive(Default)]
struct Acc {
    defines: HashSet<String>,
    refs: HashSet<String>,
    writes: HashSet<String>,
    /// Names declared `global`/`nonlocal` in the scanned body: their
    /// defines are dropped at finalize time (spec §3.6).
    excluded: HashSet<String>,
}

/// Collect the binding names of a pattern (parameter or target):
/// plain identifiers and every name inside list/tuple/splat patterns.
fn collect_pattern(node: Node, src: &[u8], out: &mut HashSet<String>) {
    if node.kind() == "identifier" {
        out.insert(txt(node, src));
        return;
    }
    for i in 0..node.named_child_count() {
        if let Some(c) = node.named_child(i) {
            collect_pattern(c, src, out);
        }
    }
}

/// Every identifier inside a type expression is a ref (spec D5 —
/// annotation names are reads of the names they mention).
fn scan_type_refs(node: Node, src: &[u8], out: &mut HashSet<String>) {
    if node.kind() == "identifier" {
        out.insert(txt(node, src));
        return;
    }
    for i in 0..node.named_child_count() {
        if let Some(c) = node.named_child(i) {
            scan_type_refs(c, src, out);
        }
    }
}

/// One parameter node → its binding names (defines) plus, for typed /
/// defaulted parameters, the annotation names (refs) and the default
/// value's reads.
fn collect_param(node: Node, src: &[u8], acc: &mut Acc) {
    match node.kind() {
        "typed_parameter" | "default_parameter" | "typed_default_parameter" => {
            if let Some(n) = node.child_by_field_name("name") {
                collect_pattern(n, src, &mut acc.defines);
            } else if let Some(first) = node.named_child(0) {
                collect_pattern(first, src, &mut acc.defines);
            }
            if let Some(t) = node.child_by_field_name("type") {
                scan_type_refs(t, src, &mut acc.refs);
            }
            if let Some(v) = node.child_by_field_name("value") {
                scan_w(v, src, acc);
            }
        }
        _ => collect_pattern(node, src, &mut acc.defines),
    }
}

/// Record an assignment/for target. Plain names define AND write
/// (spec §3.6 lists them under both); attribute targets write their
/// attribute name; pattern targets fan out to their names; subscript
/// targets are not recorded. `binding_only` (for/with/comprehension/
/// walrus targets) defines without writing.
fn record_target(node: Node, src: &[u8], acc: &mut Acc, binding_only: bool) {
    match node.kind() {
        "identifier" => {
            let name = txt(node, src);
            if acc.excluded.contains(&name) {
                // global/nonlocal: a write, never a local define.
                acc.writes.insert(name);
            } else {
                acc.defines.insert(name.clone());
                if !binding_only {
                    acc.writes.insert(name);
                }
            }
        }
        "attribute" => {
            if !binding_only {
                if let Some(attr) = node.child_by_field_name("attribute") {
                    acc.writes.insert(txt(attr, src));
                }
            }
        }
        "list_pattern" | "tuple_pattern" | "list_splat_pattern" | "dictionary_splat_pattern"
        | "splat_pattern" => {
            let mut names = HashSet::new();
            collect_pattern(node, src, &mut names);
            for n in names {
                if acc.excluded.contains(&n) {
                    acc.writes.insert(n);
                } else {
                    acc.defines.insert(n.clone());
                    if !binding_only {
                        acc.writes.insert(n);
                    }
                }
            }
        }
        // Subscript and anything else: not recorded (spec §3.6).
        _ => {}
    }
}

/// The marimo scan over a body: collect defines / refs / writes with
/// nested declarations pruned (spec §3.6).
fn scan_w(node: Node, src: &[u8], acc: &mut Acc) {
    match node.kind() {
        // Nested declarations are their own symbols.
        "function_definition" | "class_definition" | "decorated_definition" | "import_statement"
        | "import_from_statement" => {}
        "identifier" => {
            acc.refs.insert(txt(node, src));
        }
        // Only the root of an attribute chain is a read (spec §3.6).
        "attribute" => {
            if let Some(obj) = node.child_by_field_name("object") {
                scan_w(obj, src, acc);
            }
        }
        "assignment" => {
            if let Some(t) = node.child_by_field_name("type") {
                scan_type_refs(t, src, &mut acc.refs);
            }
            if let Some(l) = node.child_by_field_name("left") {
                record_target(l, src, acc, false);
            }
            if let Some(r) = node.child_by_field_name("right") {
                scan_w(r, src, acc);
            }
        }
        // Augmented assignment: the target is READ, not written
        // (spec §3.6, the disclosed limit shared with §9).
        "augmented_assignment" => {
            if let Some(l) = node.child_by_field_name("left") {
                scan_w(l, src, acc);
            }
            if let Some(r) = node.child_by_field_name("right") {
                scan_w(r, src, acc);
            }
        }
        "for_statement" => {
            if let Some(l) = node.child_by_field_name("left") {
                record_target(l, src, acc, true);
            }
            for f in ["right", "body", "alternative"] {
                if let Some(n) = node.child_by_field_name(f) {
                    scan_w(n, src, acc);
                }
            }
        }
        "with_item" => {
            // `open(…) as fh`: the alias field binds; anything else
            // in the item is read.
            let alias = node.child_by_field_name("alias");
            if let Some(a) = alias {
                collect_pattern(a, src, &mut acc.defines);
            }
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if alias.map(|a| a.id() != c.id()).unwrap_or(true) {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "as_pattern" => {
            // `expr as name` where the grammar wraps it (match arms,
            // some with/except shapes): expression read, alias binds.
            let alias = node.child_by_field_name("alias");
            if let Some(a) = alias {
                collect_pattern(a, src, &mut acc.defines);
            }
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if alias.map(|a| a.id() != c.id()).unwrap_or(true) {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "except_clause" => {
            if let Some(v) = node.child_by_field_name("value") {
                scan_w(v, src, acc);
            }
            if let Some(a) = node.child_by_field_name("alias") {
                collect_pattern(a, src, &mut acc.defines);
            }
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if c.kind() == "block" {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "list_comprehension" | "dictionary_comprehension" | "set_comprehension"
        | "generator_expression" => {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if c.kind() == "for_in_clause" {
                        if let Some(l) = c.child_by_field_name("left") {
                            record_target(l, src, acc, true);
                        }
                        if let Some(r) = c.child_by_field_name("right") {
                            scan_w(r, src, acc);
                        }
                    } else {
                        scan_w(c, src, acc);
                    }
                }
            }
        }
        "named_expression" => {
            if let Some(n) = node.child_by_field_name("name") {
                record_target(n, src, acc, true);
            }
            if let Some(v) = node.child_by_field_name("value") {
                scan_w(v, src, acc);
            }
        }
        "global_statement" | "nonlocal_statement" => {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    if c.kind() == "identifier" {
                        acc.excluded.insert(txt(c, src));
                    }
                }
            }
        }
        "lambda" => {
            if let Some(p) = node.child_by_field_name("parameters") {
                for i in 0..p.named_child_count() {
                    if let Some(c) = p.named_child(i) {
                        collect_param(c, src, acc);
                    }
                }
            }
            if let Some(b) = node.child_by_field_name("body") {
                scan_w(b, src, acc);
            }
        }
        _ => {
            for i in 0..node.named_child_count() {
                if let Some(c) = node.named_child(i) {
                    scan_w(c, src, acc);
                }
            }
        }
    }
}

/// (defines, refs, writes) of a function: parameters bind, the body
/// scans, global/nonlocal names lose their defines, reads that are
/// also defines are dropped (the finalize step shared with TS).
fn fn_body_info(
    src: &[u8],
    params: Option<Node>,
    body: Option<Node>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut acc = Acc::default();
    if let Some(p) = params {
        for i in 0..p.named_child_count() {
            if let Some(c) = p.named_child(i) {
                collect_param(c, src, &mut acc);
            }
        }
    }
    if let Some(b) = body {
        scan_w(b, src, &mut acc);
    }
    finalize(acc)
}

/// Refs read in an initializer or class body, with locally bound
/// names filtered out (the TS `body_refs`).
fn body_refs(node: Node, src: &[u8]) -> Vec<String> {
    let mut acc = Acc::default();
    scan_w(node, src, &mut acc);
    let (_, refs, _) = finalize(acc);
    refs
}

fn finalize(mut acc: Acc) -> (Vec<String>, Vec<String>, Vec<String>) {
    for name in &acc.excluded {
        acc.defines.remove(name);
    }
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
    fn py_module_id_rules() {
        assert_eq!(module_id_for("engine", Path::new("engine.py")), "engine::engine");
        assert_eq!(module_id_for("engine", Path::new("__init__.py")), "engine");
        assert_eq!(module_id_for("engine", Path::new("core/__init__.py")), "engine::core");
        assert_eq!(
            module_id_for("engine", Path::new("core/graph.py")),
            "engine::core::graph"
        );
        assert_eq!(module_id_for("engine", Path::new("main.py")), "engine::main");
        assert_eq!(
            module_id_for("engine", Path::new("core/graph.pyi")),
            "engine::core::graph"
        );
        assert_eq!(
            module_id_for("engine", Path::new("a/b/c/util.py")),
            "engine::a::b::c::util"
        );
    }

    const SYMBOLS_PY: &str = r#"
CONST = 1
LIMIT: int = 10
A = B = 2
pair_a, pair_b = (1, 2)
handler = lambda x: x

async def fetch(url, timeout=30):
    return url

def outer(a, b: int, *args, key=None, **kw):
    def inner(c):
        return c + a
    return inner

class Engine:
    kind = "v8"
    fallback = build_default()

    def start(self):
        return 1

    @property
    def level(self):
        return 2

    @staticmethod
    def make():
        return Engine()

    class Inner:
        def deep(self):
            return 3

type Alias = int
"#;

    #[test]
    fn py_symbols_and_kinds() {
        let fs = run("core/engine.py", SYMBOLS_PY);
        assert_eq!(fs.module_id, "engine::core::engine");
        let m = by_id(&fs);
        let want: BTreeMap<&str, NodeKind> = [
            ("engine::core::engine::CONST", NodeKind::Variable),
            ("engine::core::engine::LIMIT", NodeKind::Variable),
            ("engine::core::engine::A", NodeKind::Variable),
            ("engine::core::engine::B", NodeKind::Variable),
            ("engine::core::engine::handler", NodeKind::Variable),
            ("engine::core::engine::fetch", NodeKind::Function),
            ("engine::core::engine::outer", NodeKind::Function),
            ("engine::core::engine::outer::inner", NodeKind::Function),
            ("engine::core::engine::Engine", NodeKind::Class),
            ("engine::core::engine::Engine::start", NodeKind::Method),
            ("engine::core::engine::Engine::level", NodeKind::Method),
            ("engine::core::engine::Engine::make", NodeKind::Method),
            ("engine::core::engine::Engine::Inner", NodeKind::Class),
            ("engine::core::engine::Engine::Inner::deep", NodeKind::Method),
            ("engine::core::engine::Alias", NodeKind::Class),
        ]
        .into_iter()
        .collect();
        let got: BTreeMap<&str, NodeKind> = m.iter().map(|(k, v)| (*k, v.kind)).collect();
        assert_eq!(got, want);
        // Class-level assignments and tuple targets are NOT symbols,
        // and the lambda itself is not a function symbol.
        assert!(!m.contains_key("engine::core::engine::Engine::kind"));
        assert!(!m.contains_key("engine::core::engine::pair_a"));
        assert_eq!(m["engine::core::engine::handler"].kind, NodeKind::Variable);
    }

    #[test]
    fn py_symbol_lines_and_variable_refs() {
        let fs = run("core/engine.py", SYMBOLS_PY);
        let m = by_id(&fs);
        // CONST is on source line 2 (the fixture starts with a blank line).
        assert_eq!(m["engine::core::engine::CONST"].line, 2);
        // LIMIT's annotation name is its ref (D5); CONST has none.
        assert_eq!(m["engine::core::engine::LIMIT"].refs, vec!["int".to_string()]);
        assert!(m["engine::core::engine::CONST"].refs.is_empty());
        // Class body refs: the class-level initializer's callee, with
        // the assignment target filtered out; method bodies pruned.
        assert_eq!(
            m["engine::core::engine::Engine"].refs,
            vec!["build_default".to_string()]
        );
    }

    const SCAN_PY: &str = r#"
def compute(items, factor: int = 2) -> int:
    total = 0
    total += factor
    self_count = len(items)
    obj.attr = total
    for it in items:
        total = total + it
    with open("x") as fh:
        data = fh.read()
    squares = [n * n for n in items if n > 0]
    if (m := len(items)) > 1:
        total = m
    try:
        risky()
    except ValueError as err:
        report(err)
    global counter
    counter = 1
    return total
"#;

    #[test]
    fn py_defines_refs_writes_hand_derived() {
        let fs = run("calc.py", SCAN_PY);
        let m = by_id(&fs);
        let s = m["engine::calc::compute"];
        // defines: params + every binding target; `counter` is
        // global-declared, so its define is dropped.
        assert_eq!(
            s.defines,
            sv(&[
                "items", "factor", "total", "self_count", "it", "fh", "data", "squares", "n", "m",
                "err"
            ])
        );
        // writes: plain assignment targets (define+write), the
        // attribute name of `obj.attr = …`, and the global's write.
        // NOT: for/with/comprehension/walrus targets (defines only),
        // NOT the augmented-assignment target (recorded as a ref).
        assert_eq!(
            s.writes,
            sv(&["total", "self_count", "attr", "data", "squares", "counter"])
        );
        // refs: call roots, the except type, annotation names; every
        // read of a defined name is filtered out; `obj` (the object
        // of a write target) is not a ref, per the TS precedent.
        assert_eq!(s.refs, sv(&["len", "open", "risky", "ValueError", "report", "int"]));
    }
}
