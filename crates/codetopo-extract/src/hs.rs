// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Haskell extractor (`.hs`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, C, Java, Go, Shell, C++, Swift, Ruby, R,
//! Julia, C#, PHP, and Scala.
//!
//! Node kinds below were verified against
//! `tree-sitter-haskell-0.24.1/src/node-types.json` in the
//! crates.io registry cache and against parse trees of the
//! fixture (the pinned crate, never guessed): `haskell`
//! (fields `declarations`, `imports`; a `header` child),
//! `header` (field `module` — a `module` node whose
//! `module_id` children are the name's segments), `import`
//! (field `module`, field `alias`, field `names` an
//! `import_list` of `import_name`s; the `qualified` and
//! `hiding` keywords are unnamed children), `class` (fields
//! `name`, `patterns`, `context`, `declarations` a
//! `class_declarations`), `data_type` (fields `name`,
//! `constructors`, `deriving`), `newtype` (fields `name`,
//! `constructor`), `type_synonym` (fields `name`, `type`),
//! `instance` (field `name` the class, field `patterns` the
//! `type_patterns`, field `declarations` an
//! `instance_declarations`), `deriving_instance` (the same
//! two fields), `signature` (fields `name` — a `variable`
//! or `prefix_id` — and `type`; a multi-name signature has
//! a `names` `binding_list`), `function` (fields `name`,
//! `patterns`, `match` — one per guarded alternative — and
//! `binds` a `local_binds`; an infix definition has no
//! `name` field, its head is an `infix` child whose
//! operands are the two patterns), `bind` (fields `name`,
//! `pattern`, `match`, `binds`; a `do`-block generator is a
//! `bind` with a `pattern` and an `expression` instead),
//! `apply` (fields `function`, `argument` — a chain
//! `f a b` nests to the left), `qualified` (fields
//! `module`, `id`), `conditional` (fields `if`, `then`,
//! `else`), `case` (a scrutinee child and an `alternatives`
//! wrapper), `local_binds` (its named children are the
//! `decl`s: `bind` / `function` / `signature`).
//!
//! The mapping follows the Julia extractor's spec shape:
//! module ids are path-derived (the shared rule). The
//! module header is not a symbol — a Haskell file declares
//! exactly one module and the builder materialises the
//! module node from the module id (the Java package rule);
//! the header anchors import resolution. Symbols (§3.2 —
//! `data` / `newtype` / `type` declarations are classes; a
//! typeclass is an interface, Haskell's conformance analog;
//! class members are methods, a signature with a default
//! body and the body forming one symbol; instance members
//! are methods nested under the instance's *type* (under
//! the class they would collide with the class's own
//! members); a binding with patterns is a function and a
//! pattern-less binding a variable; several equations of
//! one function — pattern alternatives in separate
//! `function` nodes, guarded alternatives in separate
//! `match`es — are ONE symbol, at the first equation's
//! position; a type signature and its binding are one
//! declaration whose line and signature are the
//! signature's; record fields and data constructors are
//! not symbols; `let` / `where` bindings are not symbols).
//! Imports (internal when the first segment is the file's
//! own module root or matches the package name
//! case-insensitively — the root segment is read as the
//! package name — else `external::{first segment}`; an
//! explicit list binds the names as written, a qualified
//! import binds its alias or the module's last segment, a
//! `hiding` import binds nothing). Calls (§3.4 — the
//! callee text as written: an application chain is one
//! call, a constructor application is a call on the
//! constructor, a qualified application's full qualified
//! text is the name, a backtick infix names a call;
//! symbolic operators are not calls). Heritage (an
//! `instance C T` is `T` Implements `C`; a `deriving`
//! clause is a compiler-generated instance, Implements of
//! the same kind; a class's superclass constraints are
//! Inherits). The defines/refs/writes scan (§3.6's marimo
//! discipline: pattern variables and `let` / `where` /
//! `do` bindings define, reads that are also defines are
//! dropped, a pattern's constructor is a read, only the
//! `id` of a qualified name is a read, a record field name
//! is not a read; **writes are always empty — Haskell
//! bindings are immutable, a definition is a define, never
//! a write**); decisions (§3.2's enumeration, Haskell form:
//! `if` +1, a `case` contributes its alternatives − 1,
//! each guard +1 except a bare `otherwise`, each `&&` /
//! `||` +1; decisions inside `let` / `where` bindings are
//! the bindings' own and are pruned, as nested definitions
//! are everywhere). The walk runs on an explicit heap
//! stack (the E-1 lesson).
//!
//! Partial by design, named: `.lhs` is not supported
//! (literate Haskell's prose is not Haskell syntax to this
//! grammar); top-level pattern bindings (`(a, b) = …`)
//! are not symbols (their right-hand sides still walk for
//! calls); associated types, type families, pattern
//! synonyms, GADT constructors, and datatype contexts are
//! not extracted; instance methods for two classes over
//! one type in one file share the type's path segment.

use std::collections::{HashMap, HashSet};
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

/// A type signature declaration, pre-read so the binding it
/// belongs to can take its line, text, and type refs.
#[derive(Debug, Clone)]
struct SigInfo {
    line: u32,
    text: Option<String>,
    type_refs: Vec<String>,
}

pub(crate) fn extract(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_haskell::LANGUAGE.into())
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;
    let module_id = module_id_for(package, rel_path);
    let root = tree.root_node();
    // The declared module's root segment anchors the import
    // rule (the Java package-root rule).
    let mut module_root: Option<String> = None;
    for c in children(root) {
        if c.kind() == "header" {
            if let Some(m) = c.child_by_field_name("module") {
                module_root = module_segments(m, source.as_bytes()).into_iter().next();
            }
        }
    }
    let mut w = Walker {
        src: source.as_bytes(),
        pkg: package.to_string(),
        module_root,
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

/// The segments of a `module` node (`Quarry.Shapes` → two
/// `module_id` children).
fn module_segments(node: Node, src: &[u8]) -> Vec<String> {
    children(node)
        .into_iter()
        .filter(|c| c.kind() == "module_id")
        .map(|c| txt(c, src))
        .collect()
}

/// The name a declaration-name node denotes: a plain
/// variable or type name as written; a parenthesised
/// operator (`prefix_id`) by its operator text; a qualified
/// name by its `id` part.
fn decl_name(node: Node, src: &[u8]) -> String {
    match node.kind() {
        "prefix_id" => {
            let mut stack = vec![node];
            while let Some(n) = stack.pop() {
                if n.kind() == "operator" {
                    return txt(n, src);
                }
                for c in children(n) {
                    stack.push(c);
                }
            }
            txt(node, src)
                .trim_matches(|c| c == '(' || c == ')')
                .to_string()
        }
        "qualified" => node
            .child_by_field_name("id")
            .map(|i| txt(i, src))
            .unwrap_or_else(|| txt(node, src)),
        _ => txt(node, src),
    }
}

/// The name an `infix_id` denotes (`` `plus` ``,
/// `` `Shapes.plus` ``): the inner name, qualified by its
/// `id` part.
fn infix_id_name(node: Node, src: &[u8]) -> String {
    for c in children(node) {
        match c.kind() {
            "variable" | "constructor" | "name" => return txt(c, src),
            "qualified" => return decl_name(c, src),
            _ => {}
        }
    }
    txt(node, src).trim_matches('`').to_string()
}

/// The names a `signature` declaration names (one, or a
/// `binding_list` of them). A pattern-synonym signature
/// (a `synonym` field) names no function here.
fn signature_names(node: Node, src: &[u8]) -> Vec<String> {
    if node.child_by_field_name("synonym").is_some() {
        return Vec::new();
    }
    if let Some(n) = node.child_by_field_name("name") {
        return vec![decl_name(n, src)];
    }
    if let Some(list) = node.child_by_field_name("names") {
        return children(list)
            .into_iter()
            .map(|c| decl_name(c, src))
            .collect();
    }
    Vec::new()
}

/// The pre-read form of one `signature` node.
fn sig_info(node: Node, src: &[u8]) -> SigInfo {
    let mut type_refs: Vec<String> = Vec::new();
    if let Some(t) = node.child_by_field_name("type") {
        let mut set = HashSet::new();
        collect_type_refs(t, src, &mut set);
        type_refs = set.into_iter().collect();
        type_refs.sort();
    }
    SigInfo {
        line: line_of(node),
        text: signature_of(node, None, src),
        type_refs,
    }
}

/// Collect the type names a type expression mentions:
/// every `name` (and type-position `constructor`) in the
/// subtree. Type variables are `variable`s — not refs.
fn collect_type_refs(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "name" | "constructor" => {
                refs.insert(txt(n, src));
                continue;
            }
            "module_id" => continue,
            _ => {}
        }
        for c in children(n) {
            stack.push(c);
        }
    }
}

/// The name a `function` node defines: its `name` field,
/// or — for an infix definition (`x \`plus\` y = …`,
/// `x +++ y = …`) — the head `infix` child's operator.
fn function_name(node: Node, src: &[u8]) -> Option<String> {
    if let Some(n) = node.child_by_field_name("name") {
        return Some(decl_name(n, src));
    }
    for c in children(node) {
        if c.kind() == "infix" {
            if let Some(op) = c.child_by_field_name("operator") {
                return Some(match op.kind() {
                    "infix_id" => infix_id_name(op, src),
                    _ => txt(op, src),
                });
            }
        }
    }
    None
}

/// The pattern roots of a `function` / `bind` node: the
/// `patterns` wrapper, an infix head's two operands, or a
/// `bind`'s single `pattern`.
fn pattern_roots<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    if let Some(p) = node.child_by_field_name("patterns") {
        out.push(p);
    }
    if let Some(p) = node.child_by_field_name("pattern") {
        out.push(p);
    }
    if node.child_by_field_name("name").is_none() {
        for c in children(node) {
            if c.kind() == "infix" {
                if let Some(l) = c.child_by_field_name("left_operand") {
                    out.push(l);
                }
                if let Some(r) = c.child_by_field_name("right_operand") {
                    out.push(r);
                }
            }
        }
    }
    out
}

/// The body roots of a `function` / `bind` node: every
/// `match` child plus the `binds` (`where`) child.
fn body_roots<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    for c in children(node) {
        if c.kind() == "match" {
            out.push(c);
        }
    }
    if let Some(b) = node.child_by_field_name("binds") {
        out.push(b);
    }
    out
}

/// The head type name of an instance's `type_patterns`:
/// the first `name` in reading order (the class name lives
/// in the instance's own `name` field, outside this
/// subtree).
fn type_head_name(node: Node, src: &[u8]) -> Option<String> {
    let mut stack = vec![node];
    // Pre-order: reverse-push so the first child pops first.
    while let Some(n) = stack.pop() {
        if n.kind() == "name" {
            return Some(txt(n, src));
        }
        if n.kind() == "qualified" {
            if let Some(i) = n.child_by_field_name("id") {
                return Some(txt(i, src));
            }
            continue;
        }
        for c in children(n).into_iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// Heritage claims of one top-level declaration, computed
/// purely so the caller can emit them in source order:
/// `deriving` clauses and instances are Implements; a
/// class's superclass constraints are Inherits.
fn heritage_of_decl(node: Node, src: &[u8]) -> Vec<RawHeritage> {
    let mut out = Vec::new();
    match node.kind() {
        "data_type" | "newtype" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return out;
            };
            let type_name = decl_name(name_node, src);
            let mut cursor = node.walk();
            for d in node.children_by_field_name("deriving", &mut cursor) {
                if let Some(classes) = d.child_by_field_name("classes") {
                    let targets: Vec<String> = if classes.kind() == "name" {
                        vec![txt(classes, src)]
                    } else {
                        children(classes)
                            .into_iter()
                            .filter(|c| c.kind() == "name")
                            .map(|c| txt(c, src))
                            .collect()
                    };
                    for target in targets {
                        out.push(RawHeritage {
                            type_name: type_name.clone(),
                            target_name: target,
                            kind: EdgeKind::Implements,
                            line: line_of(d),
                        });
                    }
                }
            }
        }
        "instance" | "deriving_instance" => {
            let head = node
                .child_by_field_name("patterns")
                .and_then(|pats| type_head_name(pats, src));
            if let (Some(cls), Some(head)) = (node.child_by_field_name("name"), head) {
                out.push(RawHeritage {
                    type_name: head,
                    target_name: txt(cls, src),
                    kind: EdgeKind::Implements,
                    line: line_of(node),
                });
            }
        }
        "class" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return out;
            };
            let type_name = decl_name(name_node, src);
            if let Some(ctx) = node.child_by_field_name("context") {
                let mut stack = vec![ctx];
                let mut supers: Vec<String> = Vec::new();
                while let Some(n) = stack.pop() {
                    if n.kind() == "apply" {
                        if let Some(c) = n.child_by_field_name("constructor") {
                            if c.kind() == "name" {
                                let t = txt(c, src);
                                if !supers.contains(&t) {
                                    supers.push(t);
                                }
                            }
                        }
                        continue;
                    }
                    for c in children(n) {
                        stack.push(c);
                    }
                }
                supers.reverse();
                for target in supers {
                    out.push(RawHeritage {
                        type_name: type_name.clone(),
                        target_name: target,
                        kind: EdgeKind::Inherits,
                        line: line_of(node),
                    });
                }
            }
        }
        _ => {}
    }
    out
}

struct Walker<'a> {
    src: &'a [u8],
    pkg: String,
    /// The declared module's root segment, anchoring the
    /// import rule.
    module_root: Option<String>,
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
        line_override: Option<u32>,
    },
    LeaveSymbol {
        in_class: bool,
    },
    EnterPath {
        name: String,
    },
    LeavePath,
    DeclList {
        node: Node<'t>,
    },
}

/// One schedulable unit of a declaration list: a single
/// declaration, or one function's whole equation group.
enum Unit<'t> {
    Single(Node<'t>),
    Group { name: String, nodes: Vec<Node<'t>> },
    Bodiless { name: String, sig: SigInfo, node: Node<'t> },
    WalkOnly(Node<'t>),
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
                    line_override,
                } => {
                    self.push_symbol(
                        kind,
                        &names,
                        node,
                        defines,
                        refs,
                        writes,
                        end_line,
                        signature,
                        decisions,
                        line_override,
                    );
                    self.in_class = in_class;
                }
                Step::LeaveSymbol { in_class } => {
                    self.in_class = in_class;
                    self.pop_symbol();
                }
                Step::EnterPath { name } => {
                    self.path_counts.push(1);
                    self.path.push(name);
                }
                Step::LeavePath => {
                    let n = self.path_counts.pop().unwrap_or(0);
                    for _ in 0..n {
                        self.path.pop();
                    }
                }
                Step::DeclList { node } => self.dispatch_members(node, &mut stack),
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
            "declarations" => self.handle_declarations(node, stack),
            "import" => self.handle_import(node),
            "apply" => self.handle_apply(node, stack),
            "infix" => self.handle_infix(node, stack),
            "record" => {
                if let Some(e) = node.child_by_field_name("expression") {
                    if e.kind() == "constructor" {
                        self.calls.push(RawCall {
                            from_id: self.current_scope(),
                            to_name: self.text(e),
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

    /// One application chain is one call (§3.4): unwrap the
    /// left-nested `apply` spine to its head; a name-like
    /// head (variable, constructor, qualified name, or a
    /// parenthesised operator) is the callee, its text as
    /// written. The arguments walk in source order; a
    /// non-name head (a computed function) walks as an
    /// expression and records no call of its own.
    fn handle_apply<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let mut cur = node;
        let mut args: Vec<Node> = Vec::new();
        while cur.kind() == "apply" {
            if let Some(a) = cur.child_by_field_name("argument") {
                args.push(a);
            }
            match cur.child_by_field_name("function") {
                Some(f) => cur = f,
                None => break,
            }
        }
        let head = cur;
        let to_name = match head.kind() {
            "variable" | "constructor" | "qualified" | "prefix_id" => Some(self.text(head)),
            _ => None,
        };
        if let Some(t) = to_name.filter(|t| !t.is_empty()) {
            self.calls.push(RawCall {
                from_id: self.current_scope(),
                to_name: t,
                line: line_of(node),
            });
        }
        // Push in collected order (outermost argument first)
        // so the stack pops them in source order; a non-name
        // head goes last so it pops first.
        if !matches!(
            head.kind(),
            "variable" | "constructor" | "qualified" | "prefix_id"
        ) {
            stack.push(Step::Enter(head));
        }
        for a in args {
            stack.push(Step::Enter(a));
        }
    }

    /// An infix expression: a backtick operator names a
    /// function — a call; a symbolic or constructor operator
    /// is syntax, not a call. Only the operands walk.
    fn handle_infix<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        if let Some(op) = node.child_by_field_name("operator") {
            if op.kind() == "infix_id" {
                self.calls.push(RawCall {
                    from_id: self.current_scope(),
                    to_name: infix_id_name(op, self.src),
                    line: line_of(node),
                });
            }
        }
        if let Some(r) = node.child_by_field_name("right_operand") {
            stack.push(Step::Enter(r));
        }
        if let Some(l) = node.child_by_field_name("left_operand") {
            stack.push(Step::Enter(l));
        }
    }

    /// The top-level declaration list: heritage and imports
    /// are emitted in source order first (they are computed
    /// purely); the symbol units are then dispatched in
    /// reverse so the step stack unwinds them in source
    /// order.
    fn handle_declarations<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let kids = children(node);
        for c in &kids {
            if c.kind() == "import" {
                self.handle_import(*c);
            } else {
                self.heritage.extend(heritage_of_decl(*c, self.src));
            }
        }
        let (sigs, bodies) = decl_maps(&kids, self.src);
        let units = build_units(&kids, self.src, &sigs, &bodies, true);
        for unit in units.into_iter().rev() {
            self.dispatch_top_unit(unit, &sigs, stack);
        }
    }

    fn dispatch_top_unit<'t>(
        &mut self,
        unit: Unit<'t>,
        sigs: &HashMap<String, SigInfo>,
        stack: &mut Vec<Step<'t>>,
    ) {
        match unit {
            Unit::Group { name, nodes } => {
                self.schedule_callable(&nodes, NodeKind::Function, sigs.get(&name).cloned(), stack)
            }
            Unit::Bodiless { name, sig, node } => {
                self.schedule_bodiless(NodeKind::Function, name, sig, node, stack)
            }
            Unit::WalkOnly(node) => {
                for r in body_roots(node).into_iter().rev() {
                    stack.push(Step::Enter(r));
                }
            }
            Unit::Single(node) => match node.kind() {
                "data_type" | "newtype" | "type_synonym" => self.schedule_type(node, stack),
                "class" => self.schedule_class(node, stack),
                "instance" => {
                    let head = node
                        .child_by_field_name("patterns")
                        .and_then(|pats| type_head_name(pats, self.src));
                    if let (Some(head), Some(decls)) =
                        (head, node.child_by_field_name("declarations"))
                    {
                        stack.push(Step::LeavePath);
                        stack.push(Step::DeclList { node: decls });
                        stack.push(Step::EnterPath { name: head });
                    }
                }
                "bind" => {
                    if let Some(n) = node.child_by_field_name("name") {
                        let name = decl_name(n, self.src);
                        self.schedule_variable(node, sigs.get(&name).cloned(), stack);
                    }
                }
                "foreign_import" => self.schedule_foreign(node, stack),
                _ => {}
            },
        }
    }

    /// A class / instance member list: the same unit
    /// machinery as the top level, with methods for
    /// callables and the list's own signatures.
    fn dispatch_members<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let kids = children(node);
        let (sigs, bodies) = decl_maps(&kids, self.src);
        let units = build_units(&kids, self.src, &sigs, &bodies, false);
        for unit in units.into_iter().rev() {
            match unit {
                Unit::Group { name, nodes } => self.schedule_callable(
                    &nodes,
                    NodeKind::Method,
                    sigs.get(&name).cloned(),
                    stack,
                ),
                Unit::Bodiless { name, sig, node } => {
                    self.schedule_bodiless(NodeKind::Method, name, sig, node, stack)
                }
                Unit::WalkOnly(node) => {
                    for r in body_roots(node).into_iter().rev() {
                        stack.push(Step::Enter(r));
                    }
                }
                Unit::Single(node) => {
                    // A pattern-less member binding is a
                    // method with no patterns (a class default
                    // value, an instance value member).
                    if node.kind() == "bind" {
                        if let Some(n) = node.child_by_field_name("name") {
                            let name = decl_name(n, self.src);
                            self.schedule_callable(
                                &[node],
                                NodeKind::Method,
                                sigs.get(&name).cloned(),
                                stack,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Schedule a `data` / `newtype` / `type` declaration:
    /// a class symbol, opened and closed at once (its
    /// constructors and fields are not symbols and carry
    /// no calls).
    fn schedule_type<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = decl_name(name_node, self.src);
        let cut = match node.kind() {
            "data_type" => node
                .child_by_field_name("constructors")
                .or_else(|| children(node).into_iter().find(|c| c.kind() == "deriving")),
            "newtype" => node.child_by_field_name("constructor"),
            _ => node.child_by_field_name("type"),
        };
        let signature = signature_of(node, cut, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Class,
            names: vec![name],
            node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature,
            decisions: None,
            in_class: true,
            line_override: None,
        });
    }

    /// Schedule a typeclass: an interface symbol whose
    /// member list walks under it.
    fn schedule_class<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = decl_name(name_node, self.src);
        let decls = node.child_by_field_name("declarations");
        // The signature is the header: cut at the `where`
        // keyword (the declarations node starts after it).
        let mut cursor = node.walk();
        let cut = node
            .children(&mut cursor)
            .find(|c| c.kind() == "where")
            .or(decls);
        let signature = signature_of(node, cut, self.src);
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        if let Some(d) = decls {
            stack.push(Step::DeclList { node: d });
        }
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Interface,
            names: vec![name],
            node,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature,
            decisions: None,
            in_class: true,
            line_override: None,
        });
    }

    /// Schedule one callable (a whole equation group): a
    /// function at the top level, a method in a class or
    /// instance. The signature, when one was pre-read,
    /// supplies the line, the signature text, and the type
    /// refs.
    fn schedule_callable<'t>(
        &mut self,
        nodes: &[Node<'t>],
        kind: NodeKind,
        sig: Option<SigInfo>,
        stack: &mut Vec<Step<'t>>,
    ) {
        let Some(first) = nodes.first().copied() else {
            return;
        };
        let Some(name) = function_name(first, self.src)
            .or_else(|| {
                first
                    .child_by_field_name("name")
                    .map(|n| decl_name(n, self.src))
            })
        else {
            return;
        };
        let mut pat_roots: Vec<Node> = Vec::new();
        let mut bodies: Vec<Node> = Vec::new();
        for n in nodes {
            pat_roots.extend(pattern_roots(*n));
            bodies.extend(body_roots(*n));
        }
        let sig_refs: Vec<String> = sig.as_ref().map(|s| s.type_refs.clone()).unwrap_or_default();
        let (defines, refs) = fn_body_info(self.src, &pat_roots, &bodies, &sig_refs);
        let decisions = Some(count_decisions(&bodies, self.src));
        let signature = match &sig {
            Some(s) if s.text.is_some() => s.text.clone(),
            _ => {
                let first_match = bodies.iter().find(|b| b.kind() == "match").copied();
                signature_of(first, first_match, self.src)
            }
        };
        let end_line = nodes.iter().map(|n| end_line_of(*n)).max();
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        for b in bodies.iter().rev() {
            stack.push(Step::Enter(*b));
        }
        stack.push(Step::OpenSymbol {
            kind,
            names: vec![name],
            node: first,
            defines,
            refs,
            writes: Vec::new(),
            end_line,
            signature,
            decisions,
            in_class: false,
            line_override: sig.as_ref().map(|s| s.line),
        });
    }

    /// Schedule a top-level pattern-less binding: a
    /// variable. The symbol closes before its right-hand
    /// side walks, so calls in the initializer belong to
    /// the enclosing scope (the Julia variable rule — the
    /// fixture's `main.jl` / `Program.cs` calls are the
    /// module's).
    fn schedule_variable<'t>(
        &mut self,
        node: Node<'t>,
        sig: Option<SigInfo>,
        stack: &mut Vec<Step<'t>>,
    ) {
        let Some(n) = node.child_by_field_name("name") else {
            return;
        };
        let name = decl_name(n, self.src);
        let bodies = body_roots(node);
        let sig_refs: Vec<String> = sig.as_ref().map(|s| s.type_refs.clone()).unwrap_or_default();
        let (defines, refs) = fn_body_info(self.src, &[], &bodies, &sig_refs);
        let signature = match &sig {
            Some(s) if s.text.is_some() => s.text.clone(),
            _ => {
                let first_match = bodies.iter().find(|b| b.kind() == "match").copied();
                signature_of(node, first_match, self.src)
            }
        };
        for b in bodies.iter().rev() {
            stack.push(Step::Enter(*b));
        }
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        stack.push(Step::OpenSymbol {
            kind: NodeKind::Variable,
            names: vec![name],
            node,
            defines,
            refs,
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature,
            decisions: None,
            in_class: false,
            line_override: sig.as_ref().map(|s| s.line),
        });
    }

    /// A signature with no body in this list (a class
    /// member without a default, a lone top-level
    /// signature, a foreign function): a bodiless callable
    /// symbol, opened and closed at once.
    fn schedule_bodiless<'t>(
        &mut self,
        kind: NodeKind,
        name: String,
        sig: SigInfo,
        node: Node<'t>,
        stack: &mut Vec<Step<'t>>,
    ) {
        stack.push(Step::LeaveSymbol {
            in_class: self.in_class,
        });
        stack.push(Step::OpenSymbol {
            kind,
            names: vec![name],
            node,
            defines: Vec::new(),
            refs: sig.type_refs,
            writes: Vec::new(),
            end_line: Some(end_line_of(node)),
            signature: sig.text,
            decisions: None,
            in_class: false,
            line_override: None,
        });
    }

    /// A `foreign import`: its inner signature is a
    /// bodiless function.
    fn schedule_foreign<'t>(&mut self, node: Node<'t>, stack: &mut Vec<Step<'t>>) {
        for c in children(node) {
            if c.kind() == "signature" {
                let sig = sig_info(c, self.src);
                for name in signature_names(c, self.src).into_iter().rev() {
                    self.schedule_bodiless(NodeKind::Function, name, sig.clone(), c, stack);
                }
            }
        }
    }

    /// An `import` declaration → one RawImport.
    fn handle_import(&mut self, node: Node) {
        let Some(module_node) = node.child_by_field_name("module") else {
            return;
        };
        let segs = module_segments(module_node, self.src);
        if segs.is_empty() {
            return;
        }
        let mut qualified = false;
        let mut hiding = false;
        let mut cursor = node.walk();
        for c in node.children(&mut cursor) {
            match c.kind() {
                "qualified" => qualified = true,
                "hiding" => hiding = true,
                _ => {}
            }
        }
        let alias = node
            .child_by_field_name("alias")
            .map(|a| self.text(a).trim().to_string());
        let bindings: Vec<String> = if hiding {
            Vec::new()
        } else if qualified {
            vec![alias.unwrap_or_else(|| segs[segs.len() - 1].clone())]
        } else if let Some(list) = node.child_by_field_name("names") {
            children(list)
                .into_iter()
                .filter(|c| c.kind() == "import_name")
                .filter_map(|c| import_name_binding(c, self.src))
                .collect()
        } else if let Some(a) = alias {
            vec![a]
        } else {
            Vec::new()
        };
        let internal = self
            .module_root
            .as_deref()
            .map(|r| r == segs[0])
            .unwrap_or(false)
            || segs[0].eq_ignore_ascii_case(&self.pkg);
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
            line: line_of(node),
        });
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
        line_override: Option<u32>,
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
            line: line_override.unwrap_or_else(|| line_of(node)),
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

/// The binding an `import_name` brings: a type name, a
/// variable (a qualified one by its `id`), or an operator.
fn import_name_binding(node: Node, src: &[u8]) -> Option<String> {
    if let Some(t) = node.child_by_field_name("type") {
        return Some(txt(t, src));
    }
    if let Some(v) = node.child_by_field_name("variable") {
        return Some(decl_name(v, src));
    }
    if let Some(o) = node.child_by_field_name("operator") {
        return Some(decl_name(o, src));
    }
    if let Some(n) = node.child_by_field_name("namespace") {
        return Some(txt(n, src).trim().to_string());
    }
    None
}

/// The (signature map, bodies set) of one declaration
/// list: signatures by the name they declare; bodies the
/// names with a `function` group, a named `bind`, or a
/// foreign import in the list.
fn decl_maps(
    kids: &[Node],
    src: &[u8],
) -> (HashMap<String, SigInfo>, HashSet<String>) {
    let mut sigs: HashMap<String, SigInfo> = HashMap::new();
    let mut bodies: HashSet<String> = HashSet::new();
    for c in kids {
        match c.kind() {
            "signature" => {
                let info = sig_info(*c, src);
                for name in signature_names(*c, src) {
                    sigs.entry(name).or_insert_with(|| info.clone());
                }
            }
            "function" => {
                if let Some(n) = function_name(*c, src) {
                    bodies.insert(n);
                }
            }
            "bind" => {
                if let Some(n) = c.child_by_field_name("name") {
                    bodies.insert(decl_name(n, src));
                }
            }
            "foreign_import" => {
                for inner in children(*c) {
                    if inner.kind() == "signature" {
                        for name in signature_names(inner, src) {
                            bodies.insert(name);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (sigs, bodies)
}

/// Group one declaration list's children into schedulable
/// units, in source order: `function`s group by name (one
/// symbol per equation group), a signature whose name has
/// a body is consumed by it, a signature without one is a
/// bodiless unit, a named `bind` and every other symbol-
/// bearing declaration are single units, and a pattern
/// binding walks only its body. At the top level,
/// heritage-only declarations (`deriving instance`) and
/// non-symbol declarations produce no unit.
fn build_units<'t>(
    kids: &[Node<'t>],
    src: &[u8],
    sigs: &HashMap<String, SigInfo>,
    bodies: &HashSet<String>,
    top_level: bool,
) -> Vec<Unit<'t>> {
    let mut units: Vec<Unit<'t>> = Vec::new();
    let mut groups: HashMap<String, usize> = HashMap::new();
    for c in kids {
        match c.kind() {
            "function" => {
                let Some(name) = function_name(*c, src) else {
                    continue;
                };
                if let Some(&idx) = groups.get(&name) {
                    if let Unit::Group { nodes, .. } = &mut units[idx] {
                        nodes.push(*c);
                    }
                } else {
                    groups.insert(name.clone(), units.len());
                    units.push(Unit::Group {
                        name,
                        nodes: vec![*c],
                    });
                }
            }
            "signature" => {
                for name in signature_names(*c, src) {
                    if !bodies.contains(&name) {
                        if let Some(sig) = sigs.get(&name) {
                            units.push(Unit::Bodiless {
                                name,
                                sig: sig.clone(),
                                node: *c,
                            });
                        }
                    }
                }
            }
            "bind" => {
                if c.child_by_field_name("name").is_some() {
                    units.push(Unit::Single(*c));
                } else if top_level {
                    units.push(Unit::WalkOnly(*c));
                }
            }
            "data_type" | "newtype" | "type_synonym" | "class" | "instance"
            | "foreign_import"
                if top_level =>
            {
                units.push(Unit::Single(*c));
            }
            _ => {}
        }
    }
    units
}

// ---------------------------------------------------------------------------
// The defines/refs/writes scan (spec §3.6), Haskell form
// ---------------------------------------------------------------------------

/// Compute one callable's (defines, refs): the pattern
/// variables define; the body's own scan and the
/// signature's type names read; reads that are also
/// defines are dropped. Writes are always empty (see the
/// module header: Haskell bindings are immutable).
fn fn_body_info(
    src: &[u8],
    pat_roots: &[Node],
    body_roots: &[Node],
    sig_refs: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut defines: HashSet<String> = HashSet::new();
    let mut refs: HashSet<String> = sig_refs.iter().cloned().collect();
    for p in pat_roots {
        collect_pattern(*p, src, &mut defines, &mut refs);
    }
    for b in body_roots {
        scan_w(*b, src, &mut defines, &mut refs);
    }
    refs = refs.difference(&defines).cloned().collect();
    let mut d: Vec<String> = defines.into_iter().collect();
    d.sort();
    let mut r: Vec<String> = refs.into_iter().collect();
    r.sort();
    (d, r)
}

/// The names one pattern binds (its variables) and reads
/// (its constructors and type names). A record field name
/// is neither (punned fields are the named exception in
/// the module header's discipline: the field name subtree
/// is skipped whole).
fn collect_pattern(
    root: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
) {
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "variable" => {
                defines.insert(txt(n, src));
                continue;
            }
            "constructor" | "name" => {
                refs.insert(txt(n, src));
                continue;
            }
            "field_name" => continue,
            _ => {}
        }
        for c in children(n) {
            stack.push(c);
        }
    }
}

/// One walk computing defines/refs for a body. Iterative.
fn scan_w(
    root: Node,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "local_binds" => {
                for decl in children(node) {
                    scan_local_decl(decl, src, defines, refs, &mut stack);
                }
            }
            "bind" => {
                // A do-block generator (a declaration `bind`
                // is consumed by the local_binds arm and
                // never reaches this walk as a root child).
                if let Some(p) = node.child_by_field_name("pattern") {
                    collect_pattern(p, src, defines, refs);
                }
                if let Some(e) = node.child_by_field_name("expression") {
                    stack.push(e);
                }
                if let Some(m) = node.child_by_field_name("match") {
                    stack.push(m);
                }
            }
            "lambda" => {
                if let Some(p) = node.child_by_field_name("patterns") {
                    collect_pattern(p, src, defines, refs);
                }
                if let Some(e) = node.child_by_field_name("expression") {
                    stack.push(e);
                }
            }
            "alternative" => {
                if let Some(p) = node.child_by_field_name("pattern") {
                    collect_pattern(p, src, defines, refs);
                }
                if let Some(p) = node.child_by_field_name("patterns") {
                    collect_pattern(p, src, defines, refs);
                }
                for c in children(node) {
                    if c.kind() == "match" {
                        stack.push(c);
                    }
                }
                if let Some(b) = node.child_by_field_name("binds") {
                    stack.push(b);
                }
            }
            "generator" | "pattern_guard" => {
                if let Some(p) = node.child_by_field_name("pattern") {
                    collect_pattern(p, src, defines, refs);
                }
                if let Some(e) = node.child_by_field_name("expression") {
                    stack.push(e);
                }
            }
            "boolean" => {
                // A guard's expression — except a bare
                // `otherwise`, which is the else marker, not
                // a read (the PHP `$this` rule).
                let kids = children(node);
                let bare_otherwise = kids.len() == 1
                    && kids[0].kind() == "variable"
                    && txt(kids[0], src) == "otherwise";
                if !bare_otherwise {
                    for c in kids {
                        stack.push(c);
                    }
                }
            }
            "field_update" => {
                if let Some(e) = node.child_by_field_name("expression") {
                    stack.push(e);
                }
            }
            "field_name" => {}
            "qualified" => {
                if let Some(i) = node.child_by_field_name("id") {
                    refs.insert(txt(i, src));
                }
            }
            "variable" => {
                refs.insert(txt(node, src));
            }
            "constructor" => {
                refs.insert(txt(node, src));
            }
            "infix_id" => {
                refs.insert(infix_id_name(node, src));
            }
            "signature" => {
                // An expression type ascription (or a local
                // signature reached generically): the
                // expression reads, the type names read; the
                // name is a definition site, handled by the
                // local_binds arm.
                if let Some(e) = node.child_by_field_name("expression") {
                    stack.push(e);
                }
                if let Some(t) = node.child_by_field_name("type") {
                    collect_type_refs(t, src, refs);
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

/// One `let` / `where` declaration inside the scan: its
/// name defines, its patterns define, its signature's
/// types read, and its body is scanned (pushed) — a local
/// binding's reads are the enclosing function's reads.
fn scan_local_decl<'a>(
    decl: Node<'a>,
    src: &[u8],
    defines: &mut HashSet<String>,
    refs: &mut HashSet<String>,
    stack: &mut Vec<Node<'a>>,
) {
    match decl.kind() {
        "signature" => {
            if let Some(t) = decl.child_by_field_name("type") {
                collect_type_refs(t, src, refs);
            }
        }
        "bind" | "function" => {
            if let Some(n) = decl.child_by_field_name("name") {
                defines.insert(decl_name(n, src));
            } else if let Some(name) = function_name(decl, src) {
                defines.insert(name);
            }
            for p in pattern_roots(decl) {
                collect_pattern(p, src, defines, refs);
            }
            for b in body_roots(decl) {
                stack.push(b);
            }
        }
        _ => {}
    }
}

/// Decision points in a body (the Haskell form of §3.2's
/// enumeration). Iterative (E-1); `let` / `where` bindings
/// are pruned — their decisions are their own, as nested
/// definitions' are everywhere.
fn count_decisions(roots: &[Node], src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack: Vec<Node> = roots.to_vec();
    while let Some(node) = stack.pop() {
        match node.kind() {
            "local_binds" | "quoted_decls" => continue,
            "conditional" => n += 1,
            "case" | "lambda_case" | "lambda_cases" => {
                let mut alts = 0u32;
                for c in children(node) {
                    if c.kind() == "alternatives" {
                        alts += c.named_child_count() as u32;
                    } else if c.kind() == "alternative" {
                        alts += 1;
                    }
                }
                n += alts.saturating_sub(1);
            }
            "guards" => {
                for g in children(node) {
                    n += match g.kind() {
                        "boolean" => {
                            let is_otherwise = children(g).into_iter().any(|e| {
                                e.kind() == "variable" && txt(e, src) == "otherwise"
                            });
                            if is_otherwise {
                                0
                            } else {
                                1
                            }
                        }
                        "pattern_guard" => 1,
                        _ => 0,
                    };
                }
            }
            "infix" => {
                if let Some(op) = node.child_by_field_name("operator") {
                    if op.kind() == "operator" {
                        let t = txt(op, src);
                        if t == "&&" || t == "||" {
                            n += 1;
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn one(rel: &str, src: &str) -> FileSymbols {
        extract("quarry", &PathBuf::from(rel), src).unwrap()
    }

    #[test]
    fn module_ids_follow_the_path_rule() {
        let f = one("Core/Engine.hs", "module Quarry.Core.Engine where\n");
        assert_eq!(f.module_id, "quarry::Core::Engine");
    }

    #[test]
    fn instance_is_heritage_and_methods_nest_under_the_type() {
        let f = one(
            "S.hs",
            "module Quarry.S where\n\nclass Shape a where\n  area :: a -> Double\n\ndata Circle = Circle Double\n\ninstance Shape Circle where\n  area (Circle r) = r\n",
        );
        let rows: Vec<(&str, &str, EdgeKind)> = f
            .heritage
            .iter()
            .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind))
            .collect();
        assert_eq!(rows, vec![("Circle", "Shape", EdgeKind::Implements)]);
        let ids: Vec<&str> = f.symbols.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"quarry::S::Circle::area"));
        let m = f.symbols.iter().find(|s| s.label == "area" && s.kind == NodeKind::Method && s.id.contains("Circle")).unwrap();
        assert_eq!(m.defines, vec!["r".to_string()]);
        assert_eq!(m.refs, vec!["Circle".to_string()]);
    }

    #[test]
    fn class_superclass_is_inherits_and_deriving_is_implements() {
        let f = one(
            "S.hs",
            "module Quarry.S where\n\nclass (Eq a) => Drawable a where\n  draw :: a -> String\n\ndata Dot = Dot deriving (Show, Eq)\n",
        );
        let rows: Vec<(&str, &str, EdgeKind)> = f
            .heritage
            .iter()
            .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Drawable", "Eq", EdgeKind::Inherits),
                ("Dot", "Show", EdgeKind::Implements),
                ("Dot", "Eq", EdgeKind::Implements),
            ]
        );
    }

    #[test]
    fn imports_internal_external_qualified_hiding() {
        let f = one(
            "Core/Engine.hs",
            "module Quarry.Core.Engine where\n\nimport Quarry.Shapes (Circle, area)\nimport qualified Data.Map as Map\nimport Data.List hiding (nub)\nimport Data.Maybe\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("quarry::Shapes", vec!["Circle".to_string(), "area".to_string()]),
                ("external::Data", vec!["Map".to_string()]),
                ("external::Data", vec![]),
                ("external::Data", vec![]),
            ]
        );
    }

    #[test]
    fn equation_group_is_one_symbol() {
        let f = one(
            "C.hs",
            "module Quarry.C where\n\nclassify :: Int -> String\nclassify 0 = \"zero\"\nclassify n\n  | n > 0 = \"pos\"\n  | otherwise = \"neg\"\n",
        );
        let fns: Vec<&RawSymbol> = f.symbols.iter().filter(|s| s.label == "classify").collect();
        assert_eq!(fns.len(), 1);
        let c = fns[0];
        assert_eq!(c.kind, NodeKind::Function);
        assert_eq!(c.line, 3);
        assert_eq!(c.defines, vec!["n".to_string()]);
        assert_eq!(c.refs, vec!["Int".to_string(), "String".to_string()]);
        assert_eq!(c.decisions, Some(1));
        assert_eq!(c.signature.as_deref(), Some("classify :: Int -> String"));
    }

    #[test]
    fn scan_function_body_reads_shared_values() {
        let f = one(
            "E.hs",
            "module Quarry.E where\n\nrender :: Int -> Int\nrender wheel =\n  let scaled = area wheel * scaleFactor\n  in if scaled > 0 && wheel > 1 then scaled else 0\n",
        );
        let render = f.symbols.iter().find(|s| s.label == "render").unwrap();
        assert_eq!(render.defines, vec!["scaled".to_string(), "wheel".to_string()]);
        assert_eq!(
            render.refs,
            vec!["Int".to_string(), "area".to_string(), "scaleFactor".to_string()]
        );
        assert!(render.writes.is_empty());
        assert_eq!(render.decisions, Some(2));
        let calls: Vec<&str> = f.calls.iter().map(|c| c.to_name.as_str()).collect();
        assert_eq!(calls, vec!["area"]);
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("module Quarry.Deep where\n\ndeep :: Int\ndeep = ");
        for _ in 0..50_000 {
            src.push_str("1 + ");
        }
        src.push_str("1\n");
        let f = one("deep.hs", &src);
        assert_eq!(f.symbols.len(), 1);
        assert_eq!(f.symbols[0].label, "deep");
    }
}
