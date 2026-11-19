// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! Java extractor (`.java`) — beside Rust, TypeScript/JavaScript,
//! Python, Kotlin, and C.
//!
//! Node kinds below were verified against
//! `tree-sitter-java-0.23.5/src/node-types.json` in the crates.io
//! registry cache (the pinned crate, never guessed): `program`,
//! `package_declaration`, `import_declaration` (children:
//! `scoped_identifier`, optional `asterisk`), `class_declaration`
//! (fields `name`, `superclass`, `interfaces`, `body`),
//! `interface_declaration` (field `name`; `extends_interfaces`
//! child), `enum_declaration` (fields `name`, `interfaces`,
//! `body`), `enum_constant` (field `name`), `record_declaration`
//! (fields `name`, `interfaces`, `parameters`, `body`),
//! `annotation_type_declaration` (fields `name`, `body`),
//! `method_declaration` (fields `name`, `parameters`, `type`,
//! `body`), `constructor_declaration` (fields `name`,
//! `parameters`, `body`), `formal_parameter` (fields `name`,
//! `type`), `method_invocation` (fields `name`, `object`,
//! `arguments`), `object_creation_expression` (field `type`),
//! `assignment_expression` (fields `left`, `operator`, `right`),
//! `field_access` (fields `object`, `field`),
//! `local_variable_declaration` (fields `declarator`, `type`),
//! `variable_declarator` (fields `name`, `value`).
//!
//! The mapping follows the Python extractor's spec shape, with
//! one Java-specific rule: **module ids come from the `package`
//! declaration**, not the path — a Java type's identity is its
//! package plus its name, and source trees carry layout noise
//! (`src/main/java/…`) that is not part of that identity. The
//! module id is the package segments plus the file stem; a file
//! with no package declaration falls back to the path rule.
//! Symbols (§3.2 — classes are classes, interfaces are
//! interfaces, records and annotation types map to class and
//! interface respectively, methods and constructors are methods,
//! enum constants are variables scoped under their enum; fields
//! and record components are not symbols); imports (an import is
//! internal when its first segment equals the file's own package
//! root — then the target is the imported type's module id;
//! anything else is `external::{first segment}`; a static import
//! binds its last segment against the type's module; a wildcard
//! import targets the package module with no bindings); calls
//! (§3.4 — the invoked method's name, the Python attribute rule;
//! object creation is a call on the type name); heritage
//! (superclass and interface-extends are Inherits, implemented
//! interfaces are Implements); the defines/refs/writes scan
//! (§3.6's marimo discipline: nested declarations pruned, reads
//! that are also defines dropped, augmented-assignment targets
//! read not written, only the object of a field access is a
//! read, a call target's name is a ref); decisions (§3.2's
//! enumeration, Java form: `if` +1, `for`/enhanced `for`/`while`/
//! `do` +1, `switch` contributes labels − 1, ternary +1, each
//! `&&`/`||` +1). The walk runs on an explicit heap stack (the
//! E-1 lesson).

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
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .map_err(|e| ExtractError::Internal(e.to_string()))?;
    let tree = parser.parse(source, None).ok_or_else(|| {
        ExtractError::ParseFailed(
            display_path(rel_path),
            "tree-sitter returned no syntax tree".to_string(),
        )
    })?;
    let root = tree.root_node();
    // The package declaration (if any) decides the module id and
    // the root segment imports are matched against.
    let pkg_segments = package_segments(root, source.as_bytes());
    let module_id = module_id_for(package, rel_path, &pkg_segments);
    let mut w = Walker {
        src: source.as_bytes(),
        module_id: module_id.clone(),
        pkg_root: pkg_segments.first().cloned(),
        path: Vec::new(),
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

/// The segments of the file's `package` declaration, if it has one.
fn package_segments(root: Node, src: &[u8]) -> Vec<String> {
    for c in children(root) {
        if c.kind() == "package_declaration" {
            for part in children(c) {
                if matches!(part.kind(), "scoped_identifier" | "identifier") {
                    return txt(part, src)
                        .split('.')
                        .map(|s| s.to_string())
                        .collect();
                }
            }
        }
    }
    Vec::new()
}

/// Module id: package segments + file stem; without a package
/// declaration, the shared path rule under the caller's package.
fn module_id_for(package: &str, rel_path: &Path, pkg_segments: &[String]) -> String {
    let stem = rel_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !pkg_segments.is_empty() {
        let mut parts: Vec<String> = pkg_segments.to_vec();
        if !stem.is_empty() {
            parts.push(stem);
        }
        return parts.join("::");
    }
    let mut parts: Vec<String> = vec![package.to_string()];
    if let Some(parent) = rel_path.parent() {
        for comp in parent.components() {
            parts.push(comp.as_os_str().to_string_lossy().into_owned());
        }
    }
    if !stem.is_empty() {
        parts.push(stem);
    }
    parts.join("::")
}

struct Walker<'a> {
    src: &'a [u8],
    module_id: String,
    pkg_root: Option<String>,
    path: Vec<String>,
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
            "class_declaration" => {
                self.schedule_type(node, NodeKind::Class, stack)
            }
            "interface_declaration" => {
                self.schedule_type(node, NodeKind::Interface, stack)
            }
            "enum_declaration" => {
                self.schedule_type(node, NodeKind::Class, stack)
            }
            "record_declaration" => {
                self.schedule_type(node, NodeKind::Class, stack)
            }
            "annotation_type_declaration" => {
                self.schedule_type(node, NodeKind::Interface, stack)
            }
            "method_declaration" => self.schedule_method(node, false, stack),
            "constructor_declaration" => self.schedule_method(node, true, stack),
            "enum_constant" if self.in_class => {
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
            "import_declaration" => self.handle_import(node),
            "method_invocation" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let t = self.text(n);
                    self.calls.push(RawCall {
                        from_id: self.current_scope(),
                        to_name: t,
                        line: line_of(node),
                    });
                }
                self.push_children(stack, node);
            }
            "object_creation_expression" => {
                if let Some(t) = node.child_by_field_name("type") {
                    if let Some(name) = type_name_of(t, self.src) {
                        self.calls.push(RawCall {
                            from_id: self.current_scope(),
                            to_name: name,
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

    /// Schedule a type declaration: heritage is read from the
    /// declaration's own fields now; the stack replays — the
    /// symbol opens, the body walks under it, the scope closes.
    fn schedule_type<'t>(&mut self, node: Node<'t>, kind: NodeKind, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        // Heritage (§3.3): superclass → Inherits; implemented
        // interfaces → Implements; an interface's extended
        // interfaces → Inherits.
        let mut heritage: Vec<(String, EdgeKind, u32)> = Vec::new();
        match node.kind() {
            "class_declaration" => {
                if let Some(sc) = node.child_by_field_name("superclass") {
                    for t in children(sc) {
                        if let Some(target) = type_name_of(t, self.src) {
                            heritage.push((target, EdgeKind::Inherits, line_of(sc)));
                        }
                    }
                }
                if let Some(ifaces) = node.child_by_field_name("interfaces") {
                    for t in type_list_types(ifaces) {
                        if let Some(target) = type_name_of(t, self.src) {
                            heritage.push((target, EdgeKind::Implements, line_of(ifaces)));
                        }
                    }
                }
            }
            "interface_declaration" => {
                for c in children(node) {
                    if c.kind() == "extends_interfaces" {
                        for t in type_list_types(c) {
                            if let Some(target) = type_name_of(t, self.src) {
                                heritage.push((target, EdgeKind::Inherits, line_of(c)));
                            }
                        }
                    }
                }
            }
            "enum_declaration" | "record_declaration" => {
                if let Some(ifaces) = node.child_by_field_name("interfaces") {
                    for t in type_list_types(ifaces) {
                        if let Some(target) = type_name_of(t, self.src) {
                            heritage.push((target, EdgeKind::Implements, line_of(ifaces)));
                        }
                    }
                }
            }
            _ => {}
        }
        for (target, kind_edge, line) in heritage {
            self.heritage.push(RawHeritage {
                type_name: name.clone(),
                target_name: target,
                kind: kind_edge,
                line,
            });
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
            kind,
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
    }

    /// Schedule a method or constructor: scan computed now; the
    /// stack replays the body walk under the opened symbol.
    fn schedule_method<'t>(&mut self, node: Node<'t>, is_ctor: bool, stack: &mut Vec<Step<'t>>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.push_children(stack, node);
            return;
        };
        let name = self.text(name_node);
        let params = node.child_by_field_name("parameters");
        let body = node.child_by_field_name("body");
        let (defines, mut refs, writes) = fn_body_info(self.src, params, body);
        // The return type's names are refs (the D5 rule);
        // constructors have no return type.
        if !is_ctor {
            if let Some(t) = node.child_by_field_name("type") {
                let mut set: HashSet<String> = refs.iter().cloned().collect();
                scan_type_names(t, self.src, &mut set);
                refs = set.into_iter().filter(|r| !defines.contains(r)).collect();
                refs.sort();
            }
        }
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

    /// `import [static] a.b.C;` / `import a.b.*;` → one RawImport.
    /// Internal when the first segment equals this file's package
    /// root; a static import binds its last segment against the
    /// type's module (the path minus its last segment).
    fn handle_import(&mut self, node: Node) {
        let mut dotted: Option<String> = None;
        let mut wildcard = false;
        let mut is_static = false;
        for c in children(node) {
            match c.kind() {
                "scoped_identifier" | "identifier" => dotted = Some(self.text(c)),
                "asterisk" => wildcard = true,
                "static" => is_static = true,
                _ => {}
            }
        }
        // `static` is an unnamed keyword child; detect it in the
        // node's full child list (including unnamed children).
        if !is_static {
            let mut cursor = node.walk();
            is_static = node
                .children(&mut cursor)
                .any(|c| c.kind() == "static");
        }
        let Some(dotted) = dotted else { return };
        let segs: Vec<&str> = dotted.split('.').collect();
        let internal = self
            .pkg_root
            .as_deref()
            .map(|r| segs.first().map(|f| *f == r).unwrap_or(false))
            .unwrap_or(false);
        let (to, bindings): (String, Vec<String>) = if wildcard {
            let module = if internal {
                segs.join("::")
            } else {
                format!("external::{}", segs[0])
            };
            (module, Vec::new())
        } else if is_static && segs.len() > 1 {
            let type_path = &segs[..segs.len() - 1];
            let module = if internal {
                type_path.join("::")
            } else {
                format!("external::{}", segs[0])
            };
            (module, vec![segs[segs.len() - 1].to_string()])
        } else {
            let module = if internal {
                segs.join("::")
            } else {
                format!("external::{}", segs[0])
            };
            (module, vec![segs[segs.len() - 1].to_string()])
        };
        self.imports.push(RawImport {
            from_module_id: self.module_id.clone(),
            to_module_id: to,
            bindings,
            line: line_of(node),
        });
    }
}

/// The type nodes inside a `super_interfaces` /
/// `extends_interfaces` wrapper (through its `type_list`).
fn type_list_types(wrapper: Node) -> Vec<Node> {
    let mut out = Vec::new();
    for c in children(wrapper) {
        if c.kind() == "type_list" {
            out.extend(children(c));
        } else {
            out.push(c);
        }
    }
    out
}

/// The name a type node denotes: the last segment for a scoped
/// type, the base for a generic type, the element for an array
/// type, the text for a plain type identifier.
fn type_name_of(node: Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => Some(txt(node, src)),
        "scoped_type_identifier" => {
            let named: Vec<Node> = (0..node.named_child_count())
                .filter_map(|i| node.named_child(i))
                .collect();
            named.last().and_then(|n| type_name_of(*n, src))
        }
        "generic_type" => (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .next()
            .and_then(|n| type_name_of(n, src)),
        "array_type" => (0..node.named_child_count())
            .filter_map(|i| node.named_child(i))
            .next()
            .and_then(|n| type_name_of(n, src)),
        _ => None,
    }
}

/// Decision points in a body (the Java form of §3.2's
/// enumeration). Iterative (E-1); nested declarations pruned.
fn count_decisions(body: Node, src: &[u8]) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if matches!(
            node.kind(),
            "method_declaration"
                | "constructor_declaration"
                | "class_declaration"
                | "interface_declaration"
                | "enum_declaration"
                | "record_declaration"
                | "annotation_type_declaration"
        ) {
            continue;
        }
        n += match node.kind() {
            "if_statement" | "for_statement" | "enhanced_for_statement" | "while_statement"
            | "do_statement" | "ternary_expression" => 1,
            "switch_expression" => {
                let labels = count_kind_in(node, "switch_label")
                    + count_kind_in(node, "switch_rule");
                labels.saturating_sub(1)
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

/// Count descendants (inclusive) of `node` with the given kind.
fn count_kind_in(root: Node, kind: &str) -> u32 {
    let mut n = 0u32;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == kind {
            n += 1;
        }
        for c in children(node) {
            stack.push(c);
        }
    }
    n
}

// ---------------------------------------------------------------------------
// The defines/refs/writes scan (spec §3.6), Java form
// ---------------------------------------------------------------------------

/// Compute one method's (defines, refs, writes): the parameter
/// names, the body's own scan, the parameter types' names, minus
/// the defines from the refs. A body-less method (abstract /
/// interface) scans its parameters only.
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
            if let Some(n) = p.child_by_field_name("name") {
                defines.insert(txt(n, src));
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

/// Collect the names a type node mentions as refs: plain type
/// identifiers, generic bases and arguments, the last segment of
/// a scoped type, and primitive type names.
fn scan_type_names(node: Node, src: &[u8], refs: &mut HashSet<String>) {
    match node.kind() {
        "type_identifier" => {
            refs.insert(txt(node, src));
        }
        "scoped_type_identifier" => {
            if let Some(name) = type_name_of(node, src) {
                refs.insert(name);
            }
        }
        "integral_type" | "floating_point_type" | "boolean_type" | "void_type" => {
            refs.insert(txt(node, src));
        }
        _ => {
            for c in children(node) {
                scan_type_names(c, src, refs);
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
        FieldObj(Node<'t>),
    }
    let mut stack = vec![S::N(root)];
    while let Some(step) = stack.pop() {
        let node = match step {
            S::FieldObj(f) => {
                // Only the object of a field access is a read.
                if let Some(o) = f.child_by_field_name("object") {
                    stack.push(S::N(o));
                }
                continue;
            }
            S::N(n) => n,
        };
        match node.kind() {
            // Nested declarations are separate symbols.
            "method_declaration" | "constructor_declaration" | "class_declaration"
            | "interface_declaration" | "enum_declaration" | "record_declaration"
            | "annotation_type_declaration" => {}
            "local_variable_declaration" => {
                if let Some(t) = node.child_by_field_name("type") {
                    scan_type_names(t, src, refs);
                }
                for decl in declarator_children(node) {
                    if let Some(n) = decl.child_by_field_name("name") {
                        let name = txt(n, src);
                        defines.insert(name.clone());
                        if let Some(v) = decl.child_by_field_name("value") {
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
                                refs.insert(name);
                            }
                        }
                        "field_access" => {
                            if op == "=" {
                                if let Some(f) = l.child_by_field_name("field") {
                                    writes.insert(txt(f, src));
                                }
                            }
                            stack.push(S::FieldObj(l));
                        }
                        _ => stack.push(S::N(l)),
                    }
                }
            }
            "update_expression" => {
                for c in children(node) {
                    if c.kind() == "field_access" {
                        stack.push(S::FieldObj(c));
                    } else if c.kind() != "identifier" {
                        stack.push(S::N(c));
                    }
                }
            }
            "field_access" => stack.push(S::FieldObj(node)),
            "method_invocation" => {
                // The invoked name is a ref (the Python call rule);
                // the object and the arguments are reads.
                if let Some(n) = node.child_by_field_name("name") {
                    refs.insert(txt(n, src));
                }
                if let Some(o) = node.child_by_field_name("object") {
                    stack.push(S::N(o));
                }
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "object_creation_expression" => {
                if let Some(t) = node.child_by_field_name("type") {
                    if let Some(name) = type_name_of(t, src) {
                        refs.insert(name);
                    }
                }
                if let Some(a) = node.child_by_field_name("arguments") {
                    stack.push(S::N(a));
                }
            }
            "identifier" | "type_identifier" => {
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

/// The variable_declarator children of a declaration (the
/// `declarator` field may repeat).
fn declarator_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.children_by_field_name("declarator", &mut cursor)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn one(rel: &str, src: &str) -> FileSymbols {
        extract("t", &PathBuf::from(rel), src).unwrap()
    }

    #[test]
    fn module_id_comes_from_the_package_declaration() {
        let f = one(
            "src/main/java/quarry/core/Engine.java",
            "package quarry.core;\nclass Engine {}\n",
        );
        assert_eq!(f.module_id, "quarry::core::Engine");
        // No package declaration: the path rule under the package.
        let g = one("core/Engine.java", "class Engine {}\n");
        assert_eq!(g.module_id, "t::core::Engine");
    }

    #[test]
    fn heritage_extends_and_implements() {
        let f = one(
            "C.java",
            "package t;\nclass C extends Base implements Shape, Closeable {}\ninterface I extends Shape {}\n",
        );
        let rows: Vec<(&str, &str, EdgeKind)> = f
            .heritage
            .iter()
            .map(|h| (h.type_name.as_str(), h.target_name.as_str(), h.kind))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("C", "Base", EdgeKind::Inherits),
                ("C", "Shape", EdgeKind::Implements),
                ("C", "Closeable", EdgeKind::Implements),
                ("I", "Shape", EdgeKind::Inherits),
            ]
        );
    }

    #[test]
    fn imports_internal_external_static_wildcard() {
        let f = one(
            "E.java",
            "package quarry.core;\nimport quarry.shapes.Circle;\nimport java.util.List;\nimport static quarry.core.Engine.MAX;\nimport quarry.shapes.*;\nclass E {}\n",
        );
        let rows: Vec<(&str, Vec<String>)> = f
            .imports
            .iter()
            .map(|i| (i.to_module_id.as_str(), i.bindings.clone()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("quarry::shapes::Circle", vec!["Circle".to_string()]),
                ("external::java", vec!["List".to_string()]),
                ("quarry::core::Engine", vec!["MAX".to_string()]),
                ("quarry::shapes", vec![]),
            ]
        );
    }

    #[test]
    fn enum_constants_are_variables_under_the_enum() {
        let f = one(
            "S.java",
            "package t;\nenum Suit { SPADES, HEARTS }\n",
        );
        let names: Vec<&str> = f.symbols.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(names, vec!["Suit", "SPADES", "HEARTS"]);
        assert_eq!(f.symbols[1].id, "t::S::Suit::SPADES");
        assert_eq!(f.symbols[1].kind, NodeKind::Variable);
    }

    #[test]
    fn scan_rules_for_a_method_body() {
        let f = one(
            "E.java",
            "package t;\nclass E { int run(Circle wheel) { int local = 1; count += 1; this.size = helper(wheel); return local; } }\n",
        );
        let run = f.symbols.iter().find(|s| s.label == "run").unwrap();
        assert_eq!(run.defines, vec!["local".to_string(), "wheel".to_string()]);
        assert_eq!(run.writes, vec!["local".to_string(), "size".to_string()]);
        assert_eq!(
            run.refs,
            vec![
                "Circle".to_string(),
                "count".to_string(),
                "helper".to_string(),
                "int".to_string()
            ]
        );
    }

    #[test]
    fn deep_chain_definitions_do_not_overflow() {
        let mut src = String::from("package t;\nclass D { int f(int x) { return ");
        for _ in 0..50_000 {
            src.push_str("x + ");
        }
        src.push_str("1; } }\n");
        let f = one("D.java", &src);
        assert_eq!(f.symbols.len(), 2);
    }
}
