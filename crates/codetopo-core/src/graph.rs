// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Graph core: validated builder, query engine, snapshots' graph side.
//!
//! Pattern (marimo-inspired, implemented clean-room from a written
//! description — no marimo source was read; attribution is recorded by the
//! coordinator in ATTRIBUTION.md): a coordinator struct holds the topology
//! maps (children/parents adjacency), a definition registry (symbol id →
//! first-seen location), and cycle detection; mutations are validated before
//! they are applied (validate-then-apply per [`GraphBuilder::add_file`]);
//! impact queries (`descendants`/`ancestors`/`path`) are computed on demand
//! over the frozen [`Graph`].

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::diag::Diagnostic;
use crate::emit::FileSymbols;
use crate::schema::{Edge, EdgeKind, Loc, Node, NodeKind, Tier};

// ---------------------------------------------------------------------------
// GraphBuilder
// ---------------------------------------------------------------------------

/// A pending `defines`/`refs` entry: `(from_symbol_id, name, line)`.
/// `is_defines` selects the edge kind at build time.
struct PendingRef {
    from: String,
    name: String,
    line: u32,
    is_defines: bool,
}

/// Validate-then-apply builder for [`Graph`].
///
/// Lifecycle: `new()` → `add_file(...)` per file (each returns that file's
/// diagnostics) → `build()` once, which resolves cross-file references,
/// detects cycles, and returns the frozen graph plus all diagnostics.
pub struct GraphBuilder {
    /// Node id → node (insertion order is file order; queries sort by id).
    nodes: HashMap<String, Node>,
    /// Definition registry: symbol id → location of its FIRST definition.
    /// Powers multi-definition detection (CODE_SCHEMA §5.1).
    defs: HashMap<String, Loc>,
    /// Symbol id → module id (resolution preference: same module first).
    symbol_module: HashMap<String, String>,
    /// Module id → package (resolution preference: same package second).
    module_package: HashMap<String, String>,
    /// Imports are validated per file but materialized at build time, when
    /// every module in the corpus is known — otherwise a module defined in a
    /// later file would wrongly become an external stub.
    pending_imports: Vec<crate::emit::RawImport>,
    pending_calls: Vec<crate::emit::RawCall>,
    pending_refs: Vec<PendingRef>,
    /// All edges in deterministic insertion order.
    edges: Vec<Edge>,
    /// (from, to, kind) pairs already inserted — duplicate edges collapse.
    edge_seen: HashSet<(String, String, EdgeKind)>,
    diagnostics: Vec<Diagnostic>,
    /// Target names already reported as unresolved (dedupe per build).
    unresolved_reported: HashSet<String>,
    /// Last `::`-segment → node ids. Powers suffix-match candidate lookup in
    /// [`GraphBuilder::resolve`]: for any id that is not an exact match,
    /// `id.ends_with("::{name}")` holds iff the last `::`-segment equals
    /// `name`, so the index returns exactly the same candidate set the old
    /// full node scan did — but in O(candidates) instead of O(all nodes).
    /// Ids without `::` (package roots) can never match a `::{name}` suffix
    /// and are not indexed.
    name_index: HashMap<String, Vec<String>>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        GraphBuilder {
            nodes: HashMap::new(),
            defs: HashMap::new(),
            symbol_module: HashMap::new(),
            module_package: HashMap::new(),
            pending_imports: Vec::new(),
            pending_calls: Vec::new(),
            pending_refs: Vec::new(),
            edges: Vec::new(),
            edge_seen: HashSet::new(),
            diagnostics: Vec::new(),
            unresolved_reported: HashSet::new(),
            name_index: HashMap::new(),
        }
    }

    /// Validate one file's symbols, then apply. Returns that file's
    /// diagnostics (also accumulated for [`GraphBuilder::build`]).
    ///
    /// Behavior:
    /// - Creates the package node `{package}` (once), the module node
    ///   `{module_id}` (`Module`), and the file node `{package}::/{file}`
    ///   (`File`). The `/` infix can never collide with `::`-joined module
    ///   paths. Structural nodes carry `Tier::Optional` and an empty loc for
    ///   the package (no physical location).
    /// - Creates `contains` edges package → module → file → each symbol.
    ///   Containment is pure hierarchy, strictly separate from dependency
    ///   edges (CODE_SCHEMA §1.4).
    /// - Symbol ids are taken verbatim from `RawSymbol.id`. A duplicate id
    ///   (within this file or across files) yields a [`Diagnostic::MultiDefinition`];
    ///   the first occurrence is kept and the duplicate is ignored wholesale
    ///   (no node, no contains edge, no calls/refs recorded for it).
    /// - Symbols default to `Tier::Optional` — the extractor contract carries
    ///   no tier, and we do not invent a priority.
    /// - `RawImport.from_module_id` must be a known module, else an
    ///   [`Diagnostic::UnknownSymbol`] and the import is skipped. The import
    ///   edge itself is materialized at build time (see field docs).
    /// - `RawCall`s with an unknown `from_id` yield `UnknownSymbol` and are
    ///   skipped; the rest resolve at build time.
    pub fn add_file(&mut self, fs: FileSymbols) -> Vec<Diagnostic> {
        let mark = self.diagnostics.len();

        // ---- Phase 1: validate. Mark duplicate symbol indices. No builder
        // state is mutated until every symbol in this file has been checked
        // (validate-then-apply).
        let mut seen: HashSet<&str> = HashSet::new();
        let mut is_dup = vec![false; fs.symbols.len()];
        for (i, s) in fs.symbols.iter().enumerate() {
            if !seen.insert(s.id.as_str()) || self.defs.contains_key(&s.id) {
                is_dup[i] = true;
            }
        }

        // ---- Phase 2: apply.
        let package = fs.package.clone();
        let module_id = fs.module_id.clone();
        // The `/` infix guarantees this id can never equal a `::`-joined
        // module path or a verbatim symbol id built from one.
        let file_id = format!("{}::/{}", package, fs.file);

        // Package node: created once; the empty file/0 line loc marks "no
        // physical location".
        self.ensure_node(Node {
            id: package.clone(),
            kind: NodeKind::Package,
            label: package.clone(),
            loc: Loc { file: String::new(), line: 0, column: None },
            tier: Tier::Optional,
            defines: Vec::new(),
            refs: Vec::new(),
        });
        self.module_package.entry(module_id.clone()).or_insert_with(|| package.clone());
        self.ensure_node(Node {
            id: module_id.clone(),
            kind: NodeKind::Module,
            label: module_id.clone(),
            loc: Loc { file: fs.file.clone(), line: 1, column: None },
            tier: Tier::Optional,
            defines: Vec::new(),
            refs: Vec::new(),
        });
        self.ensure_node(Node {
            id: file_id.clone(),
            kind: NodeKind::File,
            label: fs.file.clone(),
            loc: Loc { file: fs.file.clone(), line: 1, column: None },
            tier: Tier::Optional,
            defines: Vec::new(),
            refs: Vec::new(),
        });

        self.push_edge(Edge {
            from: package.clone(),
            to: module_id.clone(),
            kind: EdgeKind::Contains,
            line: None,
            attrs: Default::default(),
        });
        self.push_edge(Edge {
            from: module_id.clone(),
            to: file_id.clone(),
            kind: EdgeKind::Contains,
            line: None,
            attrs: Default::default(),
        });

        for (i, s) in fs.symbols.iter().enumerate() {
            let loc = Loc { file: fs.file.clone(), line: s.line, column: s.column };
            if is_dup[i] {
                // First occurrence was kept (it is in `defs` by now, whether
                // it came from an earlier file or earlier in this one).
                let first = self.defs.get(&s.id).cloned().unwrap_or(Loc {
                    file: String::new(),
                    line: 0,
                    column: None,
                });
                self.diagnostics.push(Diagnostic::MultiDefinition {
                    symbol: s.id.clone(),
                    locations: vec![first, loc],
                });
                continue;
            }
            self.defs.insert(s.id.clone(), loc.clone());
            self.symbol_module.insert(s.id.clone(), module_id.clone());
            self.ensure_node(Node {
                id: s.id.clone(),
                kind: s.kind,
                label: s.label.clone(),
                loc,
                tier: Tier::Optional,
                defines: s.defines.clone(),
                refs: s.refs.clone(),
            });
            self.push_edge(Edge {
                from: file_id.clone(),
                to: s.id.clone(),
                kind: EdgeKind::Contains,
                line: None,
                attrs: Default::default(),
            });
            for d in &s.defines {
                self.pending_refs.push(PendingRef {
                    from: s.id.clone(),
                    name: d.clone(),
                    line: s.line,
                    is_defines: true,
                });
            }
            for r in &s.refs {
                self.pending_refs.push(PendingRef {
                    from: s.id.clone(),
                    name: r.clone(),
                    line: s.line,
                    is_defines: false,
                });
            }
        }

        // Calls are keyed by symbol id, so a call recorded for a duplicated
        // id attaches to the surviving (first) definition.
        for c in fs.calls {
            if !self.nodes.contains_key(&c.from_id) {
                self.diagnostics
                    .push(Diagnostic::UnknownSymbol { id: c.from_id.clone() });
                continue;
            }
            self.pending_calls.push(c);
        }

        for imp in fs.imports {
            let known_module =
                matches!(self.nodes.get(&imp.from_module_id), Some(n) if n.kind == NodeKind::Module);
            if !known_module {
                self.diagnostics.push(Diagnostic::UnknownSymbol {
                    id: imp.from_module_id.clone(),
                });
                continue;
            }
            self.pending_imports.push(imp);
        }

        self.diagnostics[mark..].to_vec()
    }

    /// Insert a fully-formed edge, enforcing the CODE_SCHEMA §4 attribute
    /// allow-list. Extension point for extractor passes beyond `add_file`.
    /// Returns diagnostics (possibly empty); the edge is inserted only when
    /// both endpoints exist.
    pub fn add_typed_edge(&mut self, edge: Edge) -> Vec<Diagnostic> {
        let mark = self.diagnostics.len();
        for id in [&edge.from, &edge.to] {
            if !self.nodes.contains_key(id) {
                self.diagnostics.push(Diagnostic::UnknownSymbol { id: id.clone() });
            }
        }
        if self.diagnostics.len() == mark {
            self.push_edge(edge);
        }
        self.diagnostics[mark..].to_vec()
    }

    /// Resolve calls, detect cycles, return (graph, diagnostics).
    ///
    /// - Imports: `to_module_id` links to the known module node, else an
    ///   `external::{to_module_id}` `External` stub is created — no diagnostic
    ///   (external dependencies are normal). Duplicate (from, to) imports
    ///   merge: bindings union (sorted), smallest line kept.
    /// - Calls resolve via [`GraphBuilder::resolve`]; failures create/reuse an
    ///   `external::{to_name}` stub, record ONE deduped
    ///   [`Diagnostic::UnresolvedCall`] per distinct target name, and still
    ///   create the calls edge (CODE_SCHEMA §5.3: unresolved targets are
    ///   diagnostics, not dropped edges). Duplicate (from, to) calls keep the
    ///   smallest line.
    /// - `defines`/`refs` resolve the same way but create no stubs and are
    ///   silent on failure (best effort). `refs` become `references` edges
    ///   with the `kind` attribute set to `"read"`.
    /// - Cycles: Tarjan SCC over calls+imports edges; one
    ///   [`Diagnostic::Cycle`] per SCC of size > 1 or self-loop.
    /// - Diagnostics are deterministic: per-file processing order, then
    ///   unresolved calls sorted by (target, from), then cycles sorted by
    ///   member ids.
    pub fn build(mut self) -> (Graph, Vec<Diagnostic>) {
        // 1. Imports (all modules are known now).
        let mut import_acc: HashMap<(String, String), (Vec<String>, u32)> = HashMap::new();
        for imp in std::mem::take(&mut self.pending_imports) {
            let to_id = match self.nodes.get(&imp.to_module_id) {
                Some(n) if n.kind == NodeKind::Module => imp.to_module_id.clone(),
                // A node with that id exists but is not a module (or nothing
                // exists): imports reference modules, so this becomes an
                // external stub rather than a bogus link.
                _ => {
                    let ext = format!("external::{}", imp.to_module_id);
                    self.ensure_external(&ext, &imp.to_module_id);
                    ext
                }
            };
            let acc = import_acc
                .entry((imp.from_module_id.clone(), to_id))
                .or_insert_with(|| (Vec::new(), u32::MAX));
            for b in imp.bindings {
                if !acc.0.contains(&b) {
                    acc.0.push(b);
                }
            }
            acc.0.sort();
            acc.1 = acc.1.min(imp.line);
        }
        let mut import_list: Vec<_> = import_acc.into_iter().collect();
        import_list.sort();
        for ((from, to), (bindings, line)) in import_list {
            let mut attrs = serde_json::Map::new();
            if !bindings.is_empty() {
                attrs.insert(
                    "bindings".to_string(),
                    serde_json::Value::Array(
                        bindings.into_iter().map(serde_json::Value::String).collect(),
                    ),
                );
            }
            self.push_edge(Edge { from, to, kind: EdgeKind::Imports, line: Some(line), attrs });
        }

        // 2. Calls.
        let mut unresolved: Vec<(String, String, u32)> = Vec::new();
        let mut call_acc: HashMap<(String, String), u32> = HashMap::new();
        for c in std::mem::take(&mut self.pending_calls) {
            let to_id = match self.resolve(&c.from_id, &c.to_name) {
                Some(id) => id,
                None => {
                    let target = c.to_name.clone();
                    let ext = format!("external::{}", target);
                    self.ensure_external(&ext, &target);
                    if self.unresolved_reported.insert(target.clone()) {
                        unresolved.push((c.from_id.clone(), target, c.line));
                    }
                    ext
                }
            };
            let acc = call_acc.entry((c.from_id.clone(), to_id)).or_insert(u32::MAX);
            *acc = (*acc).min(c.line);
        }
        unresolved.sort();
        for (from, target, line) in unresolved {
            self.diagnostics.push(Diagnostic::UnresolvedCall { from, target, line });
        }
        let mut call_list: Vec<_> = call_acc.into_iter().collect();
        call_list.sort();
        for ((from, to), line) in call_list {
            self.push_edge(Edge {
                from,
                to,
                kind: EdgeKind::Calls,
                line: Some(line),
                attrs: Default::default(),
            });
        }

        // 3. defines / references — same resolution, no stubs, silent failures.
        let mut ref_acc: HashMap<(String, String, bool), u32> = HashMap::new();
        for pr in std::mem::take(&mut self.pending_refs) {
            if let Some(to_id) = self.resolve(&pr.from, &pr.name) {
                let acc = ref_acc.entry((pr.from.clone(), to_id, pr.is_defines)).or_insert(u32::MAX);
                *acc = (*acc).min(pr.line);
            }
        }
        let mut ref_list: Vec<_> = ref_acc.into_iter().collect();
        ref_list.sort();
        for ((from, to, is_defines), line) in ref_list {
            let (kind, attrs) = if is_defines {
                (EdgeKind::Defines, serde_json::Map::new())
            } else {
                let mut m = serde_json::Map::new();
                m.insert("kind".to_string(), serde_json::Value::String("read".to_string()));
                (EdgeKind::References, m)
            };
            self.push_edge(Edge { from, to, kind, line: Some(line), attrs });
        }

        // 4. Cycle detection: Tarjan SCC over calls+imports edges.
        for cycle in self.find_cycles() {
            let set: HashSet<&str> = cycle.iter().map(|s| s.as_str()).collect();
            let has_calls = self.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && set.contains(e.from.as_str())
                    && set.contains(e.to.as_str())
            });
            let edge_kind = if has_calls { EdgeKind::Calls } else { EdgeKind::Imports };
            self.diagnostics.push(Diagnostic::Cycle { edge_kind, cycle });
        }

        // 5. Freeze: build sorted adjacency maps.
        let mut children: HashMap<String, Vec<(String, EdgeKind)>> = HashMap::new();
        let mut parents: HashMap<String, Vec<(String, EdgeKind)>> = HashMap::new();
        for e in &self.edges {
            children.entry(e.from.clone()).or_default().push((e.to.clone(), e.kind));
            parents.entry(e.to.clone()).or_default().push((e.from.clone(), e.kind));
        }
        for adj in children.values_mut().chain(parents.values_mut()) {
            adj.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.rank().cmp(&b.1.rank())));
        }

        let graph = Graph { nodes: self.nodes, edges: self.edges, children, parents };
        (graph, self.diagnostics)
    }

    /// Resolve a call/ref name to a node id:
    /// 1. exact symbol-id match;
    /// 2. `::{name}` suffix match — deterministic ranking: same module first,
    ///    then same package, then the rest; within each tier symbol kinds
    ///    (Function, Method, Class, Interface, Variable) beat container kinds
    ///    (Module, File, Package), which beat External/Doc nodes; the
    ///    lexicographic id breaks any remaining tie;
    /// 3. `None` (caller decides: stub + diagnostic, or silent).
    fn resolve(&self, from_id: &str, name: &str) -> Option<String> {
        if self.nodes.contains_key(name) {
            return Some(name.to_string());
        }
        let module = self.symbol_module.get(from_id);
        let package = module.and_then(|m| self.module_package.get(m));
        let module_prefix = module.map(|m| format!("{}::", m));
        let package_prefix = package.map(|p| format!("{}::", p));
        // tiers[0] = same module, tiers[1] = same package, tiers[2] = rest.
        let mut tiers: [Vec<(&String, u8)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        // Candidate set: ids whose last `::`-segment is `name`. Exact-match
        // ids (step 1) already returned, and `::`-less ids never match a
        // `::{name}` suffix, so this is exactly the old scan's set.
        let empty: Vec<String> = Vec::new();
        let candidates = self.name_index.get(name).unwrap_or(&empty);
        for id in candidates {
            let tier = if module_prefix.as_ref().is_some_and(|p| id.starts_with(p)) {
                0
            } else if package_prefix.as_ref().is_some_and(|p| id.starts_with(p)) {
                1
            } else {
                2
            };
            tiers[tier].push((id, Self::node_kind_rank(self.nodes[id].kind)));
        }
        tiers.iter_mut().find_map(|t| {
            // Symbol kinds before containers, External/Doc last, then
            // lexicographic id — fully deterministic.
            t.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
            t.first().map(|(id, _)| (*id).clone())
        })
    }

    /// Rank of a node kind for suffix-match tie-breaking in
    /// [`GraphBuilder::resolve`]: a call targets a *symbol*, so symbol kinds
    /// sort first; container kinds (module/file/package) next; External and
    /// Doc nodes — which never host calls — last.
    fn node_kind_rank(kind: NodeKind) -> u8 {
        match kind {
            NodeKind::Function
            | NodeKind::Method
            | NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Variable => 0,
            NodeKind::Module | NodeKind::File | NodeKind::Package => 1,
            NodeKind::External | NodeKind::Doc => 2,
        }
    }

    /// Tarjan SCC over calls+imports edges. Returns each cyclic component
    /// (size > 1, or a self-loop) with member ids sorted; components sorted
    /// by their first member — deterministic.
    fn find_cycles(&self) -> Vec<Vec<String>> {
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
        for e in &self.edges {
            if matches!(e.kind, EdgeKind::Calls | EdgeKind::Imports) {
                adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
            }
        }
        for v in adj.values_mut() {
            v.sort();
        }
        let mut order: Vec<&str> = self.nodes.keys().map(|s| s.as_str()).collect();
        order.sort();

        struct State<'a> {
            adj: &'a HashMap<&'a str, Vec<&'a str>>,
            index: HashMap<&'a str, usize>,
            lowlink: HashMap<&'a str, usize>,
            stack: Vec<&'a str>,
            on_stack: HashSet<&'a str>,
            counter: usize,
            sccs: Vec<Vec<String>>,
        }

        fn strongconnect<'a>(v: &'a str, st: &mut State<'a>) {
            st.index.insert(v, st.counter);
            st.lowlink.insert(v, st.counter);
            st.counter += 1;
            st.stack.push(v);
            st.on_stack.insert(v);
            // Clone the neighbor list to end the immutable borrow of `st`
            // before recursing with `&mut st`.
            let neighbors: Vec<&'a str> =
                st.adj.get(v).cloned().unwrap_or_default();
            for w in neighbors {
                if !st.index.contains_key(w) {
                    strongconnect(w, st);
                    let low_v = st.lowlink[v].min(st.lowlink[w]);
                    st.lowlink.insert(v, low_v);
                } else if st.on_stack.contains(w) {
                    let low_v = st.lowlink[v].min(st.index[w]);
                    st.lowlink.insert(v, low_v);
                }
            }
            if st.lowlink[v] == st.index[v] {
                let mut scc = Vec::new();
                while let Some(w) = st.stack.pop() {
                    st.on_stack.remove(w);
                    scc.push(w.to_string());
                    if w == v {
                        break;
                    }
                }
                // Cyclic iff more than one member, or a self-loop.
                let cyclic = scc.len() > 1
                    || st.adj.get(v).is_some_and(|n| n.contains(&v));
                if cyclic {
                    scc.sort();
                    st.sccs.push(scc);
                }
            }
        }

        let mut st = State {
            adj: &adj,
            index: HashMap::new(),
            lowlink: HashMap::new(),
            stack: Vec::new(),
            on_stack: HashSet::new(),
            counter: 0,
            sccs: Vec::new(),
        };
        for v in order {
            if !st.index.contains_key(v) {
                strongconnect(v, &mut st);
            }
        }
        st.sccs.sort();
        st.sccs
    }

    /// Insert a node, keeping the first node ever registered under an id.
    fn ensure_node(&mut self, node: Node) {
        if self.nodes.contains_key(&node.id) {
            return;
        }
        self.index_name(&node.id);
        self.nodes.insert(node.id.clone(), node);
    }

    /// Index a node id by its last `::`-segment for suffix-match lookup.
    /// Ids without `::` can never match a `::{name}` suffix; they are
    /// skipped so the index returns exactly the old scan's candidate set.
    fn index_name(&mut self, id: &str) {
        if id.contains("::") {
            if let Some(seg) = id.rsplit("::").next() {
                self.name_index.entry(seg.to_string()).or_default().push(id.to_string());
            }
        }
    }

    /// Create an `external::{name}` stub node if absent. External nodes mark
    /// the dependency boundary (CODE_SCHEMA §2); loc is empty (no source).
    fn ensure_external(&mut self, id: &str, label: &str) {
        if !self.nodes.contains_key(id) {
            self.index_name(id);
            self.nodes.insert(
                id.to_string(),
                Node {
                    id: id.to_string(),
                    kind: NodeKind::External,
                    label: label.to_string(),
                    loc: Loc { file: String::new(), line: 0, column: None },
                    tier: Tier::Optional,
                    defines: Vec::new(),
                    refs: Vec::new(),
                },
            );
        }
    }

    /// Insert an edge, enforcing the CODE_SCHEMA §4 attribute allow-list.
    /// Offending attributes are dropped and reported as `InvalidAttribute`
    /// diagnostics; identical (from, to, kind) edges are deduplicated.
    fn push_edge(&mut self, mut edge: Edge) {
        let allowed = edge.kind.allowed_attrs();
        let mut bad: Vec<String> = edge
            .attrs
            .keys()
            .filter(|k| !allowed.contains(&k.as_str()))
            .cloned()
            .collect();
        bad.sort();
        for attr in bad {
            edge.attrs.remove(&attr);
            self.diagnostics.push(Diagnostic::InvalidAttribute {
                edge: format!("{}->{}", edge.from, edge.to),
                attr,
            });
        }
        if self.edge_seen.insert((edge.from.clone(), edge.to.clone(), edge.kind)) {
            self.edges.push(edge);
        }
    }
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

/// One typed step of a path: `from -[kind]-> to`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathStep {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

/// A k-hop subgraph: the nodes plus the edges between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subgraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Frozen, queryable code graph.
///
/// Adjacency lists are sorted by (neighbor id, edge-kind rank) at build time,
/// so every query result is deterministic. `contains` edges are structural
/// and are excluded from dependency traversals unless noted.
#[derive(Debug, Clone)]
pub struct Graph {
    nodes: HashMap<String, Node>,
    edges: Vec<Edge>,
    children: HashMap<String, Vec<(String, EdgeKind)>>,
    parents: HashMap<String, Vec<(String, EdgeKind)>>,
}

impl Graph {
    /// Assemble a graph from parts without re-validation. Used by
    /// [`crate::snapshot::Snapshot::to_graph`]; adjacency is sorted here so
    /// query determinism holds for deserialized graphs too.
    pub(crate) fn from_parts(
        nodes: HashMap<String, Node>,
        edges: Vec<Edge>,
        mut children: HashMap<String, Vec<(String, EdgeKind)>>,
        mut parents: HashMap<String, Vec<(String, EdgeKind)>>,
    ) -> Self {
        for adj in children.values_mut().chain(parents.values_mut()) {
            adj.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.rank().cmp(&b.1.rank())));
        }
        Graph { nodes, edges, children, parents }
    }

    pub fn get(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn edge_count_by_kind(&self, kind: EdgeKind) -> usize {
        self.edges.iter().filter(|e| e.kind == kind).count()
    }

    /// Nodes in deterministic order (sorted by id).
    pub fn nodes(&self) -> impl Iterator<Item = &Node> + '_ {
        let mut ids: Vec<&String> = self.nodes.keys().collect();
        ids.sort();
        ids.into_iter().map(move |id| &self.nodes[id])
    }

    /// Edges in deterministic insertion order (file order, then build order:
    /// contains → imports → calls → defines/references).
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.iter()
    }

    /// Transitive closure over calls+imports edges (CODE_SCHEMA §6).
    /// Strictly reachable *other* nodes — the start node is never included,
    /// even on a cycle. Deterministic: BFS order, deduplicated.
    pub fn descendants(&self, id: &str) -> Vec<String> {
        self.walk(id, &[EdgeKind::Calls, EdgeKind::Imports], true)
    }

    /// Reverse of [`Graph::descendants`]: transitive closure over
    /// calls+imports edges following parent links.
    pub fn ancestors(&self, id: &str) -> Vec<String> {
        self.walk(id, &[EdgeKind::Calls, EdgeKind::Imports], false)
    }

    /// `descendants` restricted to edges that propagate breakage: calls,
    /// inherits, implements, reads, writes. Imports alone does not break
    /// (CODE_SCHEMA §6).
    pub fn blast_radius(&self, id: &str) -> Vec<String> {
        self.walk(
            id,
            &[
                EdgeKind::Calls,
                EdgeKind::Inherits,
                EdgeKind::Implements,
                EdgeKind::Reads,
                EdgeKind::Writes,
            ],
            true,
        )
    }

    fn walk(&self, start: &str, kinds: &[EdgeKind], outgoing: bool) -> Vec<String> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut order = Vec::new();
        let mut queue = VecDeque::new();
        if !self.nodes.contains_key(start) {
            return order;
        }
        // Marking the start visited up front keeps it out of the result even
        // when a cycle leads back to it ("strictly reachable others").
        visited.insert(start.to_string());
        queue.push_back(start.to_string());
        while let Some(cur) = queue.pop_front() {
            let adj = if outgoing { self.children.get(&cur) } else { self.parents.get(&cur) };
            if let Some(neigh) = adj {
                for (nid, k) in neigh {
                    if kinds.contains(k) && visited.insert(nid.clone()) {
                        order.push(nid.clone());
                        queue.push_back(nid.clone());
                    }
                }
            }
        }
        order
    }

    /// Shortest typed path. Directed BFS from `from` over all non-contains
    /// edges; if none, directed BFS from `to` and the steps are reversed.
    /// `None` when disconnected (or either endpoint is unknown). A node paths
    /// to itself as an empty step list.
    pub fn path(&self, from: &str, to: &str) -> Option<Vec<PathStep>> {
        if let Some(steps) = self.directed_path(from, to) {
            return Some(steps);
        }
        self.directed_path(to, from).map(|steps| {
            steps
                .into_iter()
                .rev()
                .map(|s| PathStep { from: s.to, to: s.from, kind: s.kind })
                .collect()
        })
    }

    fn directed_path(&self, from: &str, to: &str) -> Option<Vec<PathStep>> {
        if !self.nodes.contains_key(from) || !self.nodes.contains_key(to) {
            return None;
        }
        if from == to {
            return Some(Vec::new());
        }
        let mut visited: HashSet<String> = HashSet::from([from.to_string()]);
        let mut prev: HashMap<String, (String, EdgeKind)> = HashMap::new();
        let mut queue = VecDeque::from([from.to_string()]);
        while let Some(cur) = queue.pop_front() {
            if cur == to {
                break;
            }
            if let Some(neigh) = self.children.get(&cur) {
                for (nid, k) in neigh {
                    // Containment is hierarchy, not dependency — never part of
                    // a typed path.
                    if *k == EdgeKind::Contains {
                        continue;
                    }
                    if visited.insert(nid.clone()) {
                        prev.insert(nid.clone(), (cur.clone(), *k));
                        queue.push_back(nid.clone());
                    }
                }
            }
        }
        if !visited.contains(to) {
            return None;
        }
        let mut steps = Vec::new();
        let mut cur = to.to_string();
        while cur != from {
            let (p, k) = prev.remove(&cur).expect("visited node has a predecessor");
            steps.push(PathStep { from: p.clone(), to: cur.clone(), kind: k });
            cur = p;
        }
        steps.reverse();
        Some(steps)
    }

    /// k-hop subgraph around `id`, both directions, over non-contains edges.
    /// Nodes sorted by id; edges (both endpoints inside) sorted by
    /// (from, to, kind rank). Unknown id → empty subgraph.
    pub fn neighborhood(&self, id: &str, depth: u32) -> Subgraph {
        let mut seen: HashSet<String> = HashSet::new();
        if !self.nodes.contains_key(id) {
            return Subgraph { nodes: Vec::new(), edges: Vec::new() };
        }
        seen.insert(id.to_string());
        let mut frontier = vec![id.to_string()];
        for _ in 0..depth {
            let mut next = Vec::new();
            for cur in &frontier {
                for adj in [self.children.get(cur), self.parents.get(cur)]
                    .into_iter()
                    .flatten()
                {
                    for (nid, k) in adj {
                        if *k == EdgeKind::Contains {
                            continue;
                        }
                        if seen.insert(nid.clone()) {
                            next.push(nid.clone());
                        }
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        let mut nodes: Vec<Node> = seen
            .iter()
            .map(|i| self.nodes.get(i).expect("seen ids are node ids").clone())
            .collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut edges: Vec<Edge> = self
            .edges
            .iter()
            .filter(|e| {
                e.kind != EdgeKind::Contains
                    && seen.contains(&e.from)
                    && seen.contains(&e.to)
            })
            .cloned()
            .collect();
        edges.sort_by(|a, b| {
            a.from
                .cmp(&b.from)
                .then_with(|| a.to.cmp(&b.to))
                .then_with(|| a.kind.rank().cmp(&b.kind.rank()))
        });
        Subgraph { nodes, edges }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::{RawCall, RawImport, RawSymbol};
    use crate::schema::NodeKind;

    fn sym(id: &str, kind: NodeKind, line: u32) -> RawSymbol {
        RawSymbol {
            id: id.to_string(),
            kind,
            label: id.rsplit("::").next().unwrap_or(id).to_string(),
            line,
            column: None,
            defines: Vec::new(),
            refs: Vec::new(),
        }
    }

    fn file(
        package: &str,
        file: &str,
        module: &str,
        symbols: Vec<RawSymbol>,
        imports: Vec<RawImport>,
        calls: Vec<RawCall>,
    ) -> FileSymbols {
        FileSymbols {
            package: package.to_string(),
            file: file.to_string(),
            module_id: module.to_string(),
            symbols,
            imports,
            calls,
        }
    }

    fn call(from: &str, to: &str, line: u32) -> RawCall {
        RawCall { from_id: from.to_string(), to_name: to.to_string(), line }
    }

    /// Diamond: d calls b and c; b and c both call e.
    fn diamond_builder() -> GraphBuilder {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym("t::m::d", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 10),
                sym("t::m::c", NodeKind::Function, 20),
                sym("t::m::e", NodeKind::Function, 30),
            ],
            vec![],
            vec![
                call("t::m::d", "b", 2),
                call("t::m::d", "c", 3),
                call("t::m::b", "e", 11),
                call("t::m::c", "e", 21),
            ],
        ));
        b
    }

    #[test]
    fn multi_definition_fires_and_keeps_first() {
        let mut b = GraphBuilder::new();
        let d1 = b.add_file(file(
            "pkg",
            "a.rs",
            "pkg::a",
            vec![sym("pkg::a::foo", NodeKind::Function, 10)],
            vec![],
            vec![],
        ));
        assert!(d1.is_empty(), "first definition is clean");

        let d2 = b.add_file(file(
            "pkg",
            "b.rs",
            "pkg::b",
            vec![sym("pkg::a::foo", NodeKind::Function, 20)],
            vec![],
            vec![],
        ));
        assert_eq!(d2.len(), 1);
        match &d2[0] {
            Diagnostic::MultiDefinition { symbol, locations } => {
                assert_eq!(symbol, "pkg::a::foo");
                assert_eq!(locations.len(), 2);
                assert_eq!(locations[0].line, 10, "first occurrence kept");
                assert_eq!(locations[1].line, 20);
            }
            other => panic!("expected MultiDefinition, got {:?}", other),
        }

        let (g, _) = b.build();
        // package + 2 modules + 2 files + 1 symbol (duplicate ignored)
        assert_eq!(g.node_count(), 6);
        assert_eq!(g.get("pkg::a::foo").unwrap().loc.line, 10);
    }

    #[test]
    fn cycle_diagnostic_fires_on_mutual_calls() {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym("t::m::a", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 5),
            ],
            vec![],
            vec![call("t::m::a", "b", 2), call("t::m::b", "a", 6)],
        ));
        let (g, diags) = b.build();
        assert_eq!(diags.len(), 1);
        match &diags[0] {
            Diagnostic::Cycle { edge_kind, cycle } => {
                assert_eq!(*edge_kind, EdgeKind::Calls);
                assert_eq!(cycle, &vec!["t::m::a".to_string(), "t::m::b".to_string()]);
            }
            other => panic!("expected Cycle, got {:?}", other),
        }
        assert_eq!(g.edge_count_by_kind(EdgeKind::Calls), 2);
    }

    #[test]
    fn descendants_and_ancestors_on_diamond() {
        let (g, diags) = diamond_builder().build();
        assert!(diags.is_empty());

        // BFS order, deduplicated: b, c, then e once.
        assert_eq!(
            g.descendants("t::m::d"),
            vec![
                "t::m::b".to_string(),
                "t::m::c".to_string(),
                "t::m::e".to_string()
            ]
        );
        assert_eq!(
            g.ancestors("t::m::e"),
            vec![
                "t::m::b".to_string(),
                "t::m::c".to_string(),
                "t::m::d".to_string()
            ]
        );
        // Strictly reachable others: the start node is never included, even
        // though a cycle test elsewhere shows cycles are possible.
        assert!(!g.descendants("t::m::d").contains(&"t::m::d".to_string()));
    }

    #[test]
    fn path_returns_typed_steps() {
        let (g, _) = diamond_builder().build();
        let steps = g.path("t::m::d", "t::m::e").expect("path exists");
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[0],
            PathStep { from: "t::m::d".to_string(), to: "t::m::b".to_string(), kind: EdgeKind::Calls }
        );
        assert_eq!(
            steps[1],
            PathStep { from: "t::m::b".to_string(), to: "t::m::e".to_string(), kind: EdgeKind::Calls }
        );
        // Disconnected pair -> None.
        let mut b2 = GraphBuilder::new();
        b2.add_file(file("t", "x.rs", "t::x", vec![sym("t::x::lonely", NodeKind::Function, 1)], vec![], vec![]));
        let (g2, _) = b2.build();
        assert!(g2.path("t::x::lonely", "t::m::e").is_none());
    }

    #[test]
    fn blast_radius_excludes_imports_only_reachability() {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "p",
            "m1.rs",
            "p::m1",
            vec![sym("p::m1::f1", NodeKind::Function, 1)],
            vec![RawImport {
                from_module_id: "p::m1".to_string(),
                to_module_id: "p::m2".to_string(),
                bindings: vec!["f2".to_string()],
                line: 1,
            }],
            vec![call("p::m1::f1", "f2", 2)],
        ));
        b.add_file(file(
            "p",
            "m2.rs",
            "p::m2",
            vec![sym("p::m2::f2", NodeKind::Function, 1)],
            vec![],
            vec![],
        ));
        let (g, diags) = b.build();
        assert!(diags.is_empty());

        // Module m2 is reachable from m1 via the imports edge...
        assert!(g.descendants("p::m1").contains(&"p::m2".to_string()));
        // ...but imports alone do not propagate breakage.
        assert!(!g.blast_radius("p::m1").contains(&"p::m2".to_string()));
        // The calls edge does propagate.
        assert!(g.blast_radius("p::m1::f1").contains(&"p::m2::f2".to_string()));
    }

    #[test]
    fn external_import_stub_has_no_diagnostic() {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "p",
            "a.rs",
            "p::a",
            vec![sym("p::a::x", NodeKind::Function, 1)],
            vec![RawImport {
                from_module_id: "p::a".to_string(),
                to_module_id: "serde".to_string(),
                bindings: vec!["Serialize".to_string()],
                line: 3,
            }],
            vec![],
        ));
        let (g, diags) = b.build();
        assert!(diags.is_empty(), "external deps are normal: {:?}", diags);
        let ext = g.get("external::serde").expect("external stub created");
        assert_eq!(ext.kind, NodeKind::External);
        assert_eq!(g.edge_count_by_kind(EdgeKind::Imports), 1);
    }

    #[test]
    fn unknown_from_module_yields_unknown_symbol() {
        let mut b = GraphBuilder::new();
        let d = b.add_file(file(
            "p",
            "a.rs",
            "p::a",
            vec![],
            vec![RawImport {
                from_module_id: "nope".to_string(),
                to_module_id: "serde".to_string(),
                bindings: vec![],
                line: 1,
            }],
            vec![],
        ));
        assert!(matches!(d[0], Diagnostic::UnknownSymbol { .. }));
        let (_, diags) = b.build();
        assert_eq!(diags.len(), 1, "no extra diagnostics at build");
    }

    #[test]
    fn unresolved_call_is_deduped_and_still_creates_edge() {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![
                sym("t::m::a", NodeKind::Function, 1),
                sym("t::m::b", NodeKind::Function, 5),
            ],
            vec![],
            vec![call("t::m::a", "missing_fn", 2), call("t::m::b", "missing_fn", 6)],
        ));
        let (g, diags) = b.build();
        let unresolved: Vec<_> =
            diags.iter().filter(|d| matches!(d, Diagnostic::UnresolvedCall { .. })).collect();
        assert_eq!(unresolved.len(), 1, "one diagnostic per distinct target name");
        // Edge still created, to the shared external stub (CODE_SCHEMA §5.3).
        assert!(g.get("external::missing_fn").is_some());
        assert_eq!(g.edge_count_by_kind(EdgeKind::Calls), 2);
    }

    #[test]
    fn refs_become_references_edges_with_read_kind() {
        let mut b = GraphBuilder::new();
        let mut s = sym("t::m::a", NodeKind::Function, 1);
        s.refs = vec!["Config".to_string(), "nope_nothing".to_string()];
        b.add_file(file(
            "t",
            "m.rs",
            "t::m",
            vec![s, sym("t::m::Config", NodeKind::Class, 10)],
            vec![],
            vec![],
        ));
        let (g, diags) = b.build();
        assert!(diags.is_empty());
        let refs: Vec<_> =
            g.edges().filter(|e| e.kind == EdgeKind::References).collect();
        assert_eq!(refs.len(), 1, "unresolvable refs are silent");
        assert_eq!(refs[0].from, "t::m::a");
        assert_eq!(refs[0].to, "t::m::Config");
        assert_eq!(
            refs[0].attrs.get("kind"),
            Some(&serde_json::Value::String("read".to_string()))
        );
    }

    #[test]
    fn attr_allow_list_enforced_with_invalid_attribute_diagnostic() {
        let mut b = GraphBuilder::new();
        b.add_file(file("p", "a.rs", "p::a", vec![sym("p::a::x", NodeKind::Function, 1)], vec![], vec![]));
        let mut attrs = serde_json::Map::new();
        attrs.insert("bogus".to_string(), serde_json::Value::Bool(true));
        let d = b.add_typed_edge(Edge {
            from: "p::a::x".to_string(),
            to: "p::a".to_string(),
            kind: EdgeKind::Contains, // allows no attributes at all
            line: None,
            attrs,
        });
        assert_eq!(d.len(), 1);
        assert!(matches!(&d[0], Diagnostic::InvalidAttribute { attr, .. } if attr == "bogus"));
        // The edge survives with the offending attribute stripped.
        let (g, _) = b.build();
        let e = g
            .edges()
            .find(|e| e.kind == EdgeKind::Contains && e.from == "p::a::x")
            .expect("edge kept");
        assert!(e.attrs.is_empty());
    }

    #[test]
    fn snapshot_round_trip_preserves_graph() {
        use crate::snapshot::{Snapshot, SnapshotAuditEntry};
        let (g, _) = diamond_builder().build();
        let entries = vec![SnapshotAuditEntry {
            seq: 0,
            ts: "2026-09-26T00:00:00Z".to_string(),
            op: "genesis".to_string(),
            payload: "{}".to_string(),
            prev_hash: "GENESIS".to_string(),
            hash: "audit-head-123".to_string(),
        }];
        let snap = Snapshot::from_graph(&g, &entries);
        assert_eq!(snap.schema_version, "1.0-draft");
        assert_eq!(snap.audit_log, entries);
        let json = snap.to_json_pretty().expect("serialize");
        let snap2 = Snapshot::from_json(&json).expect("deserialize");
        assert_eq!(snap2.audit_head, "audit-head-123");
        let g2 = snap2.to_graph();
        assert_eq!(g.node_count(), g2.node_count());
        assert_eq!(g.edge_count(), g2.edge_count());
        assert_eq!(g.descendants("t::m::d"), g2.descendants("t::m::d"));
        assert_eq!(g.path("t::m::d", "t::m::e"), g2.path("t::m::d", "t::m::e"));
    }

    #[test]
    fn diagnostics_display_as_one_liners() {
        let d = Diagnostic::Cycle {
            edge_kind: EdgeKind::Calls,
            cycle: vec!["a".to_string(), "b".to_string()],
        };
        assert_eq!(d.to_string(), "cycle in calls edges: a -> b -> a");
        let d = Diagnostic::MultiDefinition {
            symbol: "s".to_string(),
            locations: vec![
                Loc { file: "a.rs".to_string(), line: 1, column: None },
                Loc { file: "b.rs".to_string(), line: 2, column: Some(3) },
            ],
        };
        assert_eq!(d.to_string(), "multi-definition of 's': a.rs:1, b.rs:2:3");
    }

    /// Regression (W5): call `b()` from `fixture::a::a` resolved to the
    /// MODULE node `fixture::b` (lexicographically first suffix match)
    /// instead of the FUNCTION `fixture::b::b`. Suffix-match tie-breaking
    /// now ranks symbol kinds before container kinds.
    #[test]
    fn suffix_match_prefers_function_over_module() {
        let mut b = GraphBuilder::new();
        b.add_file(file(
            "fixture",
            "a.rs",
            "fixture::a",
            vec![sym("fixture::a::a", NodeKind::Function, 1)],
            vec![],
            vec![call("fixture::a::a", "b", 2)],
        ));
        b.add_file(file(
            "fixture",
            "b.rs",
            "fixture::b",
            vec![sym("fixture::b::b", NodeKind::Function, 10)],
            vec![],
            vec![],
        ));
        let (g, diags) = b.build();
        assert!(diags.is_empty(), "expected no diagnostics, got {:?}", diags);
        let calls: Vec<_> = g
            .edges()
            .filter(|e| e.kind == EdgeKind::Calls && e.from == "fixture::a::a")
            .collect();
        assert_eq!(calls.len(), 1, "exactly one calls edge from fixture::a::a");
        assert_eq!(
            calls[0].to, "fixture::b::b",
            "call b() must resolve to the function, not the module"
        );
    }
}
