# Survey: marimo-team/marimo (reactive Python notebook — code as a dataflow graph)

Surveyed 2026-09-26 via marimo docs (dataflow guide, reactivity), repo source-structure docs (PR "split up DirectedGraph into smaller components"), and two third-party analyses of the dataflow module (pi-extensions capability-substrate docs; marimo-pair SKILL). No live-browser interaction.

## What it renders
- A **reactive Python notebook UI**: cells with rich outputs; run a cell and dependents re-run automatically (or are marked stale in lazy mode).
- **Dependencies panel** (sidebar): two tabs — **Minimap** (current cell's input cells left, output cells right) and **Graph** (full cell-dependency graph). Plus a **Variables explorer**: searchable list of every variable with name, type, value, where defined, where used.
- The visual graph is a *byproduct* of the runtime's dataflow graph — the model comes first, the picture second. This is the closest in spirit to our "structured data first, visuals as byproduct" goal.

## Its data model
A notebook is a **directed graph: nodes = cells, edges = data dependencies**.

- **Edge rule (pure, language-agnostic)**: *for each name a cell defines, its referrers become its children; for each name a cell references, its definers become its parents.* Edges need only `{defs, refs}` per unit — the graph layer never parses code.
- **Input extraction**: per-cell `defs`/`refs` come from static AST analysis (`marimo/_ast/visitor.py` → `CellImpl.variable_data` / `CellImpl.refs`). Variables are typed (variable / import / function / class).
- **Well-formedness constraints**: a variable may be defined in at most one cell; no cyclic references across cells (cycle detection via `CycleTracker`/`detect_cycle_for_edge`, BFS `get_path`).
- **Implementation** (`marimo/_runtime/dataflow/`, refactored from a monolithic `dataflow.py`): `DirectedGraph` — a **frozen dataclass coordinator** over four pure structures: `MutableGraphTopology` (`_cells`, `_children`, `_parents`; `add_node/remove_node/add_edge/get_path`), `DefinitionRegistry` (`definitions: dict[Name, set[CellId]]`, `get_multiply_defined`), `CycleTracker`, `EdgeComputer`; exports `transitive_closure`, `topological_sort`.
- **Query API**: `descendants(cid)` (what re-runs on change), `ancestors(cid)` (what this depends on), `get_path`, topological ordering, stale-marking propagation, `is_disabled`/disabled-transitively status.
- **File format is the graph**: notebooks are plain `.py` files; each cell is an `@app.cell` function whose **parameter list = names it reads** and whose **return tuple = names it defines** — the artifact itself is machine-readable structure without a parse step. Cells execute in topological order (`marimo run`, or plain `python notebook.py`).

## What makes it good (3 strengths)
1. **Graph core with zero AST coupling**: `DirectedGraph` never sees source code; everything downstream (edges, cycles, topo-sort, staleness) is pure set logic over `{defs, refs}`. That seam is directly portable to *any* language's code units (functions, modules, classes).
2. **The artifact IS the graph**: the `@app.cell` signature convention (params = reads, returns = defines) means an agent reads structure by reading the file's shape — no separate graph export needed. Closest existing embodiment of "structured data first."
3. **Agent-consumes-graph precedent**: the marimo-pair SKILL shows agents driving sessions via `ctx.graph.descendants(cid)` / `ctx.graph.ancestors(cid)` — e.g. query descendants *before* a destructive delete to assess impact. Our product thesis (graph as agent API) is already validated in the field.

## Where it is human-eyes-first (our gaps to exploit)
- **The graph exists to serve the notebook UI**: its job is reactive re-execution and feeding the sidebar minimap/graph panel. The query API is an internal runtime concern, not a standalone agent-facing service with a stable contract.
- **Cell-centric, session-centric**: assumes a single author editing cells in a live session; the unit of structure is the notebook cell, not the function/module/package hierarchy an agent exploring a *codebase* needs.
- **Explorers are UI, not endpoints**: the variables explorer and dependency graph are searchable *panels for eyes*. There's no "give me the subgraph of everything affected by this symbol as JSON" API for a headless consumer.

## License — ✅ Apache-2.0, REUSABLE WITH ATTRIBUTION
- Confirmed via GitHub's license field, marimo's own education primer ("available under the Apache 2.0 License"), and multiple downstream attributions. The `marimo/_runtime/dataflow/*` module — the most portable piece — is Apache-2.0.
- **Verdict**: code reuse permitted with attribution (retain license + copyright notice; e.g. header noting "Portions adapted from marimo-team/marimo, Apache-2.0" alongside our "Owned by Nrupal Akolkar · Built with Muse by Meta" attribution formula). Clean-room from docs/behavior also fine, and gives us language-independence marimo itself doesn't have.

## 3 concrete ideas to incorporate
1. **The `{defs, refs}` → edges pure rule as our core graph builder**: generalize "cell" to any code unit (function, class, module, package). Feed the same three pure structures — topology, definition registry, cycle tracker — from any language's parser. Port/adapt the `DirectedGraph` coordinator pattern (frozen dataclass over pure substructures) — this is Apache-2.0 and the single most valuable steal.
2. **Impact-query API as the primary product surface**: `descendants(x)` / `ancestors(x)` / `get_path(a, b)` / topological sort / stale propagation, exposed as an agent-callable API (MCP/Conduit style). marimo proves the semantics; we make the query interface, not the notebook, the product. "What breaks if I touch this symbol?" is the killer query.
3. **Artifact-is-graph encoding convention**: define a convention where the structural contract is readable without parsing — e.g. our library emits graph JSON whose node records carry `defines`/`references` sets inline (mirroring how `@app.cell` signatures carry defs/refs), plus multiple-definition and cycle diagnostics as first-class validation errors (marimo's `MultipleDefinitionError` pattern). Dry-run-validate-then-apply discipline for graph mutations, borrowed from their coordinator design.
