# CODE_SCHEMA — codetopo typed code-entity ontology

**Version:** 1.1 (2026-10-25)
**License:** Apache-2.0 — this spec is adoption-facing; anyone may implement against it.
**Ownership:** Owned by [Nrupal Akolkar](https://nrupalakolkar.com) · Built with [Muse by Meta](https://muse.ai/join) — join Muse, then redeem code **101EEK** in Settings within 48 hours of joining and we both earn tokens.

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
- `tier` — `core` | `recommended` | `optional` (queryable priority, never a dash pattern). Classified by kind in the builder, identically for every surface: the package/module/file skeleton is `core`, class/interface declarations are `recommended`, everything else is `optional`.
- `defines` / `refs` — the marimo contract: symbols defined here, symbols referenced here

*Amendment 1.1 (2026-10-25, Phase B).* Three **optional measured fields**
join the record. They are extractor-emitted measurements, not derived
views: each is a fact read off the source text at extraction time.

- `end_line` — last source line of the declaration (1-based, inclusive).
  With `loc.line` it gives the symbol's line span.
- `signature` — the declaration text: body and initializer excluded,
  whitespace runs collapsed to single spaces, the trailing separator
  (`{`, `;`, `=`) stripped.
- `decisions` — decision points counted in the symbol's body, per the
  normative per-language enumeration in §3.2. Emitted only for
  function/method symbols that have a body.

All three are optional, and absence is meaningful: a missing field is
**not measured**, never zero. Snapshots written before 1.1 carry none of
them and read unchanged (§8: additive, minor bump); every metric built
on these fields reports absence as absence, per §9. Everything else
(docstrings, quality judgments) remains a derived view or out of scope
— complexity and the §9 metrics are *computed from* these measurements
by query code, identically on every surface.

## 3.1 Package discovery and import resolution

Indexing assigns each file to a **package** before extraction:

- The nearest ancestor directory (within the indexed tree) carrying a
  manifest with a package name wins: `Cargo.toml` (`[package] name`),
  `package.json` (`"name"`), `pyproject.toml` (`[project]` or
  `[tool.poetry]` name). A Cargo `[workspace]`-only manifest names no
  package — its members declare their own.
- Discovered files are extracted under their real package name with
  paths relative to the package root, so ids read the way the
  ecosystem names them — and node locations use that same
  package-relative path (the §3 record's form). Files with no
  manifest ancestor keep the caller-supplied package name and
  repo-relative paths.
- A package's **root module** (a crate's `lib.rs`, a package's
  `index.ts`) shares the package's id: the package node doubles as
  its root module for containment and import resolution.

Imports resolve at graph-build time, in this order: the target
verbatim when it is a module; the **longest module prefix** of the
target (`use a::b::Item` imports module `a::b`); a **workspace
package** named by the target's first segment (hyphen/underscore
insensitive — Cargo writes `my-crate`, Rust paths write `my_crate`;
`@scope/name` counts as one name), resolving bare references through
the imported bindings to the defining module; otherwise an
`external::` stub. An extractor's pre-applied `external::` marker
means "unresolved", never a doubled prefix — a workspace package of
that name wins over the stub. Path-qualified calls resolve the same
way: `crate` substitutes the caller's package root, then the tiered
suffix match applies to the last segment.

## 3.2 Decision points (normative enumeration)

Cyclomatic complexity follows McCabe (1976): for one entry point,
**v = decision points + 1**, where a decision point is a place the
control flow can diverge. The extractor counts decision points;
complexity itself is the derived view `decisions + 1` (§9). A k-way
branch contributes k−1 decision points — each arm past the first is
one more binary choice. Nested item declarations are never descended
into (their decisions belong to their own symbols); closures, arrow
functions, and function expressions are not symbols and *are*
descended into — their branches count toward the enclosing symbol.

**Rust** (tree-sitter-rust): `if` expression (including `if let`; each
`else if` is a nested `if`) +1; `while`, `for`, `loop` +1 each; `match`
+(arms − 1); the `?` operator (`try_expression`) +1; each `&&` and
`||` in a binary expression +1.

**TypeScript / JavaScript** (tree-sitter-typescript): `if` +1; `for`,
`for…in`/`for…of`, `while`, `do…while` +1 each; ternary (`?:`) +1;
`switch` + (number of `case` clauses) — `default` is the fall-through
path and is not counted; `catch` +1; each `&&`, `||`, and `??` in a
binary expression +1.

A branch-free function body counts 0 decision points in both
languages, so its complexity is exactly 1.

## 4. Edge kinds (typed, kind-scoped attributes)

| Edge | From → To | Attributes |
|---|---|---|
| `imports` | module → module/external | `bindings[]`, `line` |
| `calls` | function/method → function/method | `line`, `arg_shapes?` |
| `inherits` | class → class | `bases[]` |
| `implements` | class → interface | `members[]` |
| `defines` | any → symbol defined here | `line` |
| `references` | any → symbol (legacy: pre-2026-10 snapshots) | `line` |
| `contains` | package → module → file → class → function | (none — pure hierarchy) |
| `reads` / `writes` | any → symbol read / written here | `line` |

No edge kind may carry attributes outside its row. Unknown relationships are
`references`, never invented kinds — the schema grows by versioned amendment only.

*Amendment 2026-10-10 (final-shape pass, PR: extractor conformance).* The
table previously documented reads/writes two ways at once — as `references`
edges carrying a `kind: read|write` attribute here, and as first-class edge
kinds in §6's blast-radius definition. The first-class reading wins: the
builder now emits `reads` for every resolved ref and `writes` for every
resolved assignment target the extractors report, as their own kinds with
no attributes. `references` remains in the kind set so snapshots written
before this amendment restore unchanged; new graphs do not emit it.
`inherits` / `implements` are likewise emitted for the first time, from
heritage the extractors report (Rust `impl Trait for Type` and trait
supertraits; TypeScript `extends` / `implements` clauses).

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
- `blast_radius(id)` — the dependents of `id`: transitive closure over *incoming*
  edges that propagate breakage (`calls`, `inherits`, `implements`,
  `reads`/`writes`); `imports` alone does not break. (Direction corrected
  2026-10-10: earlier text said `descendants`, which named what `id` depends
  on — the opposite of the DESIGN §3 promise, "what breaks if I change X".)
- `path(a, b)` — shortest *directed* typed path; returns edge-kind sequence,
  not just hops. `None` when no directed path exists — implementations
  must not synthesize one by reversing edges
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

## 9. Derived metrics (definitions are normative)

codetopo reports **measurements** — facts with a stated method — and
never quality scores, grades, or verdicts. Every metric below is
computed from the §3 record by query code, identically on the CLI,
HTTP, and MCP surfaces, and states its method beside its result. A
metric whose inputs were not measured is **absent** from the answer —
never zero, never estimated.

- **Cyclomatic complexity** of a function/method: `v = decisions + 1`
  (McCabe 1976; decisions per §3.2). Reported only where `decisions`
  was measured.
- **Size** of a symbol, in lines: `end_line − loc.line + 1`. Reported
  only where `end_line` was measured.

**Types.** A *type* is a `class` or `interface` node; a method belongs
to the type whose id prefixes it by one segment (`{type}::{method}`).
*Dependency edges* — the set every coupling metric ranges over — are
all edge kinds except `contains`: `imports`, `calls`, `inherits`,
`implements`, `defines`, `reads`, `writes`, `references`.

- **CBO** (coupling between objects; Chidamber & Kemerer 1994): the
  number of distinct types coupled to this type — a dependency edge
  between this type or one of its methods and another type or one of
  its methods, in either direction.
- **RFC** (response for a class; Chidamber & Kemerer 1994): the
  type's method count plus the distinct methods of other types they
  call (`external` stubs excluded).
- **WMC** (weighted methods per class; Chidamber & Kemerer 1994): the
  sum of the methods' cyclomatic complexities. Absent unless *every*
  method's complexity was measured.
- **ATFD** (access to foreign data; Lanza & Marinescu 2006),
  operationalized: the distinct symbols outside this type — variables
  and methods — targeted by `calls`/`reads`/`writes` edges from this
  type's methods (`external` stubs excluded).
- **LCOM / TCC** (Lanza & Marinescu 2006; Bieman & Kang 1995), over
  **recorded accesses**: a method's accessed set is the targets of its
  `reads`/`writes` edges plus its recorded `refs`, intersected with
  the type's member names (its methods' labels, plus the labels of
  everything any of its methods writes). A method pair *shares* when
  their accessed sets intersect. LCOM is reported as the raw counts —
  sharing pairs and non-sharing pairs — and TCC as sharing / total
  pairs. Absent when the type has fewer than two methods or no
  recorded accesses. Stated limit of the record: field reads and
  compound assignments (`+=`) are not recorded by the current
  extractors, and a write survives only when its target resolved to a
  symbol — cohesion measures what the record holds.

**Packages.** A node's package is the first `::` segment of its id;
`external::` stubs are nobody's package. Coupling between the
repository's **own packages** is counted as distinct node-to-node
dependency pairs (the B0.3 rule):

- **Ca** (afferent coupling; Martin 1994): pairs from other packages
  into this package. **Ce** (efferent): pairs out of this package to
  other packages of the same repository.
- **Instability**: `Ce / (Ca + Ce)`; absent when Ca + Ce = 0.
- **External coupling** (`external_ce`): pairs from this package to
  `external::` stubs — counted and named separately, **never merged
  into Ce**.

**Change history (local surfaces only).** History is an input, not
something an index carries: a *transaction* is the set of files one
commit touched (merge commits excluded), supplied by a local surface
from the working tree's git log. Where no history is supplied — every
hosted surface, and any tree without recorded history — the answer is
an unmeasured report (`measured: false`, with the reason), never
zeros.

- **Hotspot** (Tornhill 2015): per file, *changes* = the transactions
  in the stated window that touched it, and *complexity* = the sum of
  its functions' cyclomatic complexities (absent unless every one was
  measured). Both factors are always reported, and their product.
- **Change coupling** (Zimmermann et al. 2005): per file pair,
  *support* = transactions containing both files, *confidence* =
  support / transactions containing either file. Reported as the pair
  counts themselves, never a bare score; pairs below the query's
  stated minimum support are not reported.
