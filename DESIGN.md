# Agent-Optimized Code Visualization Library — Design

**Name:** `codetopo` (decided 2026-09-26; see §0)
**Status:** Design study, 2026-09-26. Phase 0 complete — no code written yet.
**Ownership:** Owned by Nrupal Akolkar · Built with Muse by Meta. RA-style open-source posture.

## 0. Decisions locked 2026-09-26 (Nrupal)

1. **Name — `codetopo`.** The md-recommended `codelens` was rejected: npm/PyPI/crates were
   free but GitHub carries 1308 `codelens` repos, including `roslyn-codelens-mcp`
   (agent + MCP + semantic code understanding — direct concept overlap). `codetopo`
   is clean on crates.io, npm, and PyPI (all 404) with only 15 small GitHub repos,
   none a product. It also names the differentiator: topological code graphs.
2. **License — split.** Core engine: **AGPL-3.0** (same as RA). Adoption-facing parts —
   the CODE_SCHEMA spec, client SDKs, and the API/tool contract — stay **Apache-2.0**
   so nothing blocks third-party agents building against us.
3. **Language — production from day one.** Rust core (graph engine + tree-sitter
   extractors; Python is too slow for this). TS/JS for the portable agent-API layer
   (build once, run everywhere). **JSON is the universal export pathway** — every
   export (snapshots, query results, spec docs) is JSON.
4. **Packaging & business model.** Ship as a **packaged app for self-hosted
   environments** (container) **plus** a hosted service on our side. Hosted pricing:
   **2× our Cloudflare cost** — we profit on the hosting spread.
5. **Trust primitive.** Per the standing doctrine, port axiomcode's tamper-evident
   audit log + HMAC proof-certificate pattern: graph mutations and snapshots are
   hash-chained, and analysis artifacts carry proof certificates binding artifact
   hashes + signer + verification status.

## 1. Problem / gap

Every existing code-visualization tool is built for **human eyes**: diagrams to glance at,
trees to browse, waterfalls to scan. An AI agent working over a codebase doesn't glance —
it needs **answers**: what calls this, what breaks if I touch that, what's the shortest
path between A and B, where's the cycle. Today the agent gets pixels, formatted text
trees, or nothing at all.

The survey of Nrupal's starred reference repos confirms the white space:

- **codebase-mcp** (Apache-2.0) proved agents want *tools over dashboards* and *symbols
  over files* — but it indexes symbols as **nodes with no edges**. No import graph, no
  call graph, nothing traversable. Our entire reason to exist is the edges.
- **Arize Phoenix** (ELv2, clean-room only) proved the right *architecture* — typed
  entity schema → MCP/agent query surface → server-side aggregation (their benchmark:
  89 turns/$10.35 → 7 turns/$0.23 by computing server-side) — but its substrate is
  **runtime telemetry**, not static code structure.
- **marimo** (Apache-2.0) proved the graph-core pattern (`{defs, refs}` → pure graph,
  impact queries like `descendants`/`ancestors`) — but it's **cell- and session-centric**,
  built to drive a notebook UI.
- **ECharts** (Apache-2.0) is the rendering answer, **oh-my-openagent** (SUL-1.0,
  clean-room only) the graph-as-workspace answer, **roadmap.sh** (custom restrictive
  license, clean-room only) the model–renderer-split answer.

**Thesis:** build the library whose *primary consumer is an agent*: code structure
(dependency graphs, call graphs, architecture maps) as structured, queryable data
first; human visuals as a deterministic byproduct; exposed as an agent-use API in the
Conduit style (MCP tools first, REST second, visuals third).

## 2. Data model — CODE_SCHEMA (spec first, before any renderer)

One canonical typed schema, written as `CODE_SCHEMA.md` before any code — our
OpenInference equivalent (Phoenix's single most portable idea).

**Node kinds:** `module`, `package`, `file`, `class`, `interface`, `function`,
`method`, `variable`, `external` (third-party / stdlib), `doc` (documentation anchor).

**Edge kinds (typed, each with kind-scoped attributes):**
- `imports` — attrs: binding names, line
- `calls` — attrs: call-site line, argument shapes (when known)
- `inherits` / `implements` — attrs: base list
- `defines` — symbol → where it's defined (marimo's registry pattern)
- `references` — symbol → where it's read
- `contains` — package → module → file → class → function (containment, kept
  **separate from dependency edges** — the omo-herdr-dag discipline worth copying verbatim)
- `reads` / `writes` — data-flow edges where resolvable

**Node core schema — minimal and LLM-emittable** (OmO's `{id, prompt, category,
dependsOn}` lesson: keep the write path trivially simple):

```json
{ "id": "pkg/module.py::ClassName.method", "kind": "method",
  "label": "method", "loc": {"file": "pkg/module.py", "line": 42},
  "tier": "core", "defines": ["x"], "refs": ["y", "z"] }
```

Richness lives in derived views, not in the write path. Every node carries stable
`id`s that double as documentation anchors (roadmap.sh's per-node content addressing,
clean-room).

**Graph builder:** the marimo `{defs, refs}` → edges pure rule, generalized from
"cell" to any code unit. Frozen-dataclass coordinator over three pure structures:
topology (`children`/`parents`), definition registry, cycle tracker. Dry-run-validate-
then-apply for mutations; multiple-definition and cycle diagnostics as first-class
validation errors.

**Well-formedness constraints:** a symbol defined in at most one place flags a
diagnostic; cycles are detected and reported as data, not hidden by the layout.

## 3. API shape — tools first, visuals last

**Layer 1 — MCP tools (the product).** Typed tools, Conduit-style, in this priority:
- `describe_schema` — return the CODE_SCHEMA (so the agent learns the ontology)
- `get_symbol(id)` — node + incident edges + counts (smallest-unit retrieval)
- `get_callers(id)` / `get_callees(id)` — 1-hop traversals
- `neighborhood(id, depth)` — k-hop subgraph (ECharts' `emphasis.focus:'adjacency'`
  as a first-class *query*, serving both agent and human highlight)
- `dependency_subgraph(predicate)` — filter/project the graph
- `blast_radius(id)` — "what breaks if I touch this" (marimo's `descendants`, codebase-wide)
- `path(a, b)` — shortest/typed paths between symbols
- `topo_sort(subgraph)` — safe change ordering

**Context economics (non-negotiable):** the default answer is an **aggregate**
(node + edge counts, top-k fan-out, summary stats); full subgraph dumps require an
explicit `expand` parameter. Server-side aggregation programs run where the graph
lives (Phoenix's code-mode economics). Never drag a 50k-node graph into context.

**Layer 2 — REST/HTTP.** The same operations over HTTP for non-MCP consumers.

**Layer 3 — human visuals.** One serializable **spec document** (the ECharts `option`
pattern) generated from any query result; renderers are interchangeable consumers
(roadmap.sh's `renderer` discriminator, clean-room).

**Surgical graph ops** (OmO's named-mutation lesson, clean-room): `expand`, `collapse`,
`annotate`, `prune` mutate the agent's working graph incrementally — no rebuilds.

**Persistence:** event-sourced JSON snapshots (`{runId, nodeStates, edges, errors}` —
the MIT omo-herdr-dag schema pattern) so an agent resumes analysis days later, and
the same snapshot feeds both queries and renders.

## 4. Rendering approach

- **Renderer:** Apache ECharts (Apache-2.0, direct dependency, fair with attribution)
  as the human-visual byproduct engine. `series-graph` + `tree` + `treemap` +
  `sankey` cover dependency/call/containment views.
- **Semantic layouts, not physics:** force-directed layout optimizes aesthetics; we
  want deterministic, meaning-bearing layouts — topological layering by dependency
  direction, call-depth layering, package grouping. Computed from the graph, never
  stored as pixel coordinates (roadmap.sh's baked-in pixel positions are the
  anti-pattern).
- **DOM-free pipeline:** build → query → aggregate all run in Node with no DOM;
  ECharts SSR (`ssr:true` + `renderToSVGString()`) produces portable SVG strings
  server-side for reports, PR descriptions, docs. Browser touch only at final export.
- **Human visual dialect:** when rendering for humans, speak the
  **system-design-101** visual grammar (boxes, layers, arrows — what engineers
  expect an architecture map to look like). Study-only reference (CC BY-NC-ND 4.0 —
  no derivatives shipped).
- **CI as publisher:** architecture maps rebuilt on every push and embedded into
  docs/PRs automatically — the **oss-analytics** pattern (ISC): `main` always has
  fresh maps, no human opening a dashboard required.
- **Tier as data, not dash pattern:** core/recommended/optional priority is a
  queryable attribute on nodes/edges (roadmap.sh's stroke-dash hack made semantic,
  clean-room) — so an agent can answer "minimum viable path through this module"
  and humans still get their visual tiers.

## 5. What we incorporate from each reference (licenses + attribution)

| Reference | License | Status | What we take |
|---|---|---|---|
| apache/echarts | **Apache-2.0** ✅ | Reuse with attribution | Direct dependency; `option` spec pattern as the agent↔visual contract; SSR SVG pipeline; `emphasis.focus:'adjacency'` → `neighborhood()` query primitive |
| marimo-team/marimo | **Apache-2.0** ✅ | Reuse with attribution | `DirectedGraph` coordinator pattern (frozen dataclass over topology/registry/cycle-tracker); `{defs,refs}`→edges pure rule; `descendants`/`ancestors`/`get_path`/topo-sort impact queries as the product surface; validate-then-apply mutation discipline |
| danyQe/codebase-mcp | **Apache-2.0** ✅ | Reuse with attribution | Tool-surface-first API contract; dual-store (deterministic graph store = truth, vector index = fuzzy overlay); smallest-unit retrieval (node + incident edges, counts; full dumps opt-in) |
| jc01rho/omo-herdr-dag | **MIT** ✅ | Reuse with attribution | Event-sourced JSON snapshot schema; dependency-edges vs containment-links kept as distinct edge types (copy verbatim); event-subscription update pattern |
| Arize-ai/openinference | **Apache-2.0** ✅ | Reuse with attribution | The typed-schema-*pattern*: kind-scoped attributes, spec-first ontology design |
| observablehq/notebook-kit | **ISC** ✅ | Reuse with attribution | Reactive cell-dependency graph as the closest prior art for "code as queryable graph, visuals as byproduct"; notebook→static publish pipeline |
| observablehq/oss-analytics | **ISC** ✅ | Reuse with attribution | CI-regenerated viz artifacts embedded in docs/PRs — our maps-on-every-push pattern |
| opensearch-project/OpenSearch-Dashboards | **Apache-2.0** ✅ | Reuse with attribution | Reference architecture: one backend serving a structured query API + rich human visuals |
| langfuse/langfuse (core) | **MIT** ✅ | Reuse with attribution | Nested-span trace timeline viz as the analog for visualizing agent analysis runs; **stay out of `ee/` dirs (commercial)** |
| Arize-ai/phoenix | **Elastic License 2.0** ⚠️ | Clean-room only — NO code reuse | Typed entity schema drives rendering/filtering/graphing; derived views from one model (trace → waterfall + agent graph); MCP + server-side aggregation query economics |
| code-yeongyu/oh-my-openagent | **SUL-1.0** ⚠️ | Clean-room only — NO code reuse | Graph-as-workspace (the graph *is* the work); named surgical mutation ops (`retry`/`send`/`amend` → our `expand`/`collapse`/`annotate`/`prune`); LLM-emittable minimal node schema |
| felicityblueish/developer-roadmap | **Custom restrictive** ⚠️ | Clean-room only — NO code/content reuse | Model–renderer split via `renderer` discriminator; per-node content addressing (stable id → docs); tier-as-data (never stroke-dash hacks) |
| ByteByteGoHq/system-design-101 | **CC BY-NC-ND 4.0** ⚠️ | Study-only — no derivatives | Human visual grammar for architecture maps (what the byproduct should *look* like) |

Attribution formula on everything we ship: "Owned by Nrupal Akolkar · Built with
Muse by Meta", plus license-preservation notices (Apache-2.0 `LICENSE`/`NOTICE`,
MIT headers) on redistributed portions. An `ATTRIBUTION.md` ledger tracks every
incorporation: source, license, what was taken, commit where it landed.

## 6. What we build clean-room

These exist nowhere in the references in the form we need:

1. **The CODE_SCHEMA spec itself** — a typed code-entity ontology (node/edge kinds,
   kind-scoped attributes) written spec-first. OpenInference shows the *pattern*;
   the code-domain ontology is ours to author.
2. **The traversable typed edge graph** — nobody ships import/call/inherit edges as
   queryable agent data (codebase-mcp has nodes without edges; Phoenix has edges
   without code).
3. **Blast-radius / impact queries over static structure** — marimo's are cell-level
   in a live session; ours are symbol-level across whole codebases, headless.
4. **Server-side graph aggregation for static code** — Phoenix's code mode runs
   telemetry SQL; ours runs graph analytics where the graph lives.
5. **Multi-language parser front end** — tree-sitter-based extractors emitting the
   uniform `{defs, refs}` contract per language (marimo's AST coupling lives only on
   its input side — the seam we reuse).
6. **Deterministic semantic layouts** — topological/call-depth/package layering
   computed from the graph (ECharts gives us the renderer, not the layout policy).

## 7. Phased build plan

**Phase 0 — Spec & ledger (design only, no code).**
Write `CODE_SCHEMA.md` (node/edge kinds, attributes, constraints, query semantics).
Stand up `ATTRIBUTION.md` with the license table above. Decide the working name.
Exit criteria: schema reviewed; every reference classified reuse-vs-clean-room.

**Phase 1 — Graph core (Rust).**
Tree-sitter extractors (Rust first, TypeScript second) emitting `{defs, refs}` per
symbol → pure graph builder (adapted from marimo's dataflow, Apache-2.0, attributed)
→ SQLite canonical store (`nodes`, `edges`, `annotations`) → validation diagnostics
(multi-definition, cycles) as first-class errors → hash-chained audit log over
mutations (trust primitive, §0.5). Headless CLI: `codetopo index ./repo` produces
the store + a JSON snapshot.
Exit criteria: index a 10k-file repo; query `descendants`/`ancestors`/`path` from
the CLI; snapshot round-trips.

**Phase 2 — Agent query API.**
MCP server exposing the Layer-1 tools; aggregate-first responses with explicit
`expand`; server-side aggregation programs; surgical ops
(`expand`/`collapse`/`annotate`/`prune`); dual-store with optional vector overlay
(codebase-mcp pattern). Dogfood: point it at a real repo and have an agent do
impact analysis before a refactor.
Exit criteria: an agent completes "what breaks if I change X" using only MCP
tools, without reading whole files; token cost measured and documented.

**Phase 3 — Human visuals (byproduct).**
Spec-doc renderer → ECharts (dependency) SSR to SVG; semantic layout policies;
per-node doc anchors; CI job rebuilding maps on push and embedding into docs/PRs
(oss-analytics pattern). Visual grammar follows the system-design-101 dialect.
Exit criteria: `main` of a test repo always carries fresh, correct maps;
a human can read one cold and navigate the architecture.

**Phase 4 — Conduit-style agent-use API.**
Hosted or self-hosted service: multi-repo indexing, embeddings overlay, auth,
usage metering; docs written for agent consumers (tool schemas, cost guidance).
This is where it becomes the product Nrupal described — "an agent-use API /
Conduit-like". No hosted offering touches ELv2/SUL-1.0/restrictive-licensed
material (all clean-room or permissively licensed by then — verify at phase gate).
Exit criteria: external agent (not ours) integrates against the API docs alone.

## 8. Open questions — all answered 2026-09-26 (see §0)

1. ~~Working name~~ → **`codetopo`** (codelens rejected: crowded GitHub namespace + concept overlap).
2. ~~License~~ → **AGPL-3.0 core; Apache-2.0 for the adoption-facing parts** (schema spec, SDKs, API contract).
3. ~~Phase 1 languages~~ → **Rust first, TypeScript second** (Python too slow; production language from day one).
4. ~~Hosted vs self-hosted~~ → **Both**: packaged self-hostable app + our hosted service at 2× Cloudflare cost.

---
Surveys: `surveys/` (7 files: apache-echarts, oh-my-openagent, developer-roadmap,
marimo, arize-phoenix, codebase-mcp, stars-scan). Design study completed 2026-09-26.
Standing rule honored: every incorporation above is from observable behavior,
public docs, or compatibly-licensed code, with attribution — nothing proprietary,
nothing behind access controls.
