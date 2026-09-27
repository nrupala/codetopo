# CODE_SCHEMA — codetopo typed code-entity ontology

**Version:** 1.0-draft (2026-09-26)
**License:** Apache-2.0 — this spec is adoption-facing; anyone may implement against it.
**Ownership:** Owned by Nrupal Akolkar · Built with Muse by Meta.

This is the spec-first ontology for codetopo: the single typed schema every
extractor emits, every query reads, and every renderer consumes. Written before
any code, versioned independently of any implementation.

## 1. Design principles

1. **Agents first.** Every construct must be answerable by a query, not just drawable.
2. **Minimal write path.** Emitting a node is trivially simple; richness lives in
   derived views, never in what the extractor must produce.
3. **Typed edges.** An edge's kind determines which attributes it may carry.
4. **Containment ≠ dependency.** `contains` edges (package → module → file → symbol)
   are kept strictly separate from dependency edges (`imports`, `calls`, …).
5. **Stable IDs.** Every node ID doubles as a documentation anchor.

## 2. Node kinds

| Kind | Meaning |
|---|---|
| `package` | A distributable unit (crate, npm package, PyPI project) |
| `module` | A single compilable unit (file-level, language-appropriate) |
| `file` | A physical file (superset anchor when module mapping is ambiguous) |
| `class` | A class / struct / trait / interface definition |
| `interface` | A pure contract (trait, protocol, interface) — split from `class` where the language distinguishes |
| `function` | A free function |
| `method` | A function bound to a class/struct |
| `variable` | A module-level variable / constant worth tracking |
| `external` | Third-party or stdlib symbol (dependency boundary marker) |
| `doc` | A documentation anchor (README section, doc page) linkable from code nodes |

## 3. Minimal node record (the write path)

```json
{ "id": "mycrate::net::Client::connect", "kind": "method",
  "label": "connect", "loc": {"file": "src/net.rs", "line": 42},
  "tier": "core", "defines": ["timeout"], "refs": ["TcpStream", "Config"] }
```

- `id` — globally unique, stable across re-indexes: `{package}::{path}::{Symbol}`
- `kind` — one of the node kinds above
- `label` — human short name
- `loc` — file + line (1-based); column optional
- `tier` — `core` | `recommended` | `optional` (queryable priority, never a dash pattern)
- `defines` / `refs` — the marimo contract: symbols defined here, symbols referenced here

Everything else (signatures, docstrings, complexity) is a derived view, not the write path.

## 4. Edge kinds (typed, kind-scoped attributes)

| Edge | From → To | Attributes |
|---|---|---|
| `imports` | module → module/external | `bindings[]`, `line` |
| `calls` | function/method → function/method | `line`, `arg_shapes?` |
| `inherits` | class → class | `bases[]` |
| `implements` | class → interface | `members[]` |
| `defines` | any → symbol defined here | `line` |
| `references` | any → symbol read here | `line`, `kind` (read/write) |
| `contains` | package → module → file → class → function | (none — pure hierarchy) |
| `reads` / `writes` | function/method → variable | `line` |

No edge kind may carry attributes outside its row. Unknown relationships are
`references`, never invented kinds — the schema grows by versioned amendment only.

## 5. Well-formedness constraints

1. A non-external symbol is `defines`-ed in **at most one** place; violations are
   first-class diagnostics, not silent merges.
2. Cycles in `imports`/`calls` are detected and **reported as data**; layouts must
   not hide them.
3. Every `calls` edge's target must resolve to a node or to an `external` stub;
   unresolved targets are diagnostics, not dropped edges.
4. `contains` forms a forest (each node has at most one container).

## 6. Query semantics (binding on implementations)

- `descendants(id)` / `ancestors(id)` — transitive closure over `calls` + `imports`
- `blast_radius(id)` — `descendants` restricted to edges that propagate breakage
  (`calls`, `inherits`, `implements`, `reads`/`writes`); `imports` alone does not break
- `path(a, b)` — shortest typed path; returns edge-kind sequence, not just hops
- `neighborhood(id, depth)` — k-hop subgraph, both directions
- **Aggregate-first:** default responses are counts/summaries; full dumps require
  explicit `expand`

## 7. JSON export pathway

Every artifact codetopo emits — index snapshots, query results, spec documents for
renderers — is JSON conforming to this schema. JSON is the universal export
pathway; anything else (SVG, docs) is derived from JSON, never parallel to it.

## 8. Versioning

Additive changes (new optional attributes, new node/edge kinds) bump minor.
Removals or renames bump major. Extractors declare the schema version they emit;
consumers reject major mismatches loudly.
