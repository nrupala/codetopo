# Survey: danyQe/codebase-mcp

**Repo:** https://github.com/danyQe/codebase-mcp
**What it is:** Open-source AI dev assistant via the Model Context Protocol — turns Claude Desktop (or any MCP client) into a coding assistant with semantic code search, persistent memory, git workflows, and code writing tools. Python + React/TS focus.
**Scale:** ~50 stars, 8 forks, 6 commits, created 2025-07-13. Small, early, community project — treat claims as directional, not battle-tested.

## What it renders (chart/graph types, interaction model)

- Almost nothing visual in the chart/graph sense: **project structure trees** ("enhanced tree with stats") and a **file structure visualization** on a local web dashboard (`localhost:6789`, FastAPI-served).
- The primary interface is **not visual at all** — it's 13 MCP tools consumed by the LLM through chat. The dashboard is secondary.
- Interaction model: agent calls typed tools (`search_tool`, `list_file_symbols_tool`, `project_structure_tool`, `read_code_tool`…) over MCP (stdio) → tools proxy over HTTP/REST to a FastAPI core engine → local stores. Output is text/JSON the model consumes, not pixels.

## Data model (how data is structured, fed in, queried)

- **Ingest pipeline:** source files → per-language **chunkers** (`chunkers/`) → local embeddings (`AllMiniLM-L6-v2`, runs on-machine) → **FAISS** vector index for semantic search; parallel **symbol extraction** into a **SQLite** metadata store (functions, classes, interfaces, per-file symbols). `schemas/`, `semantic_search/`, `project_structure/` directories hold this logic.
- **Stores:** FAISS (vectors), SQLite (structured metadata), plus a `.codebase` git dir tracking AI changes separately from the user's `.git`, and a `memory_system/` (categorized learnings: progress, mistakes, solutions, architecture — with semantic search + importance scoring).
- **Query surface (13 MCP tools):**
  - `search_tool` — 4 modes: **semantic, fuzzy, text, symbol**
  - `list_file_symbols_tool` / `read_symbol_from_database` — symbol-level retrieval: get a function's definition without reading the whole file
  - `read_code_tool` — smart reading: symbol-level, line ranges, whole file
  - `project_structure_tool` — project visualization (tree + stats)
  - `project_context_tool` — structure, dependencies, overview
  - `code_analysis_tool` — syntax, linting, imports, dependencies
  - plus `session_tool`, `memory_tool`, `git_tool`, `write_tool`, `edit_file`
- **Key architectural idea: dual store.** Deterministic structured index (SQLite symbols) as one retrieval path, vector embeddings (FAISS) as another. The agent picks the mode per question.

## What makes it good (distinctive strengths)

1. **Agent-native from the start** — the tool surface, not a dashboard, is the product. Every capability is a typed tool with a schema the model can call; nothing requires eyeballs. This is the exact posture our library needs (data first, visuals as byproduct).
2. **Symbol-level retrieval (context economy)** — `read_symbol_from_database` returns the smallest structural unit (one function/class) instead of whole files. An agent asking "where is X defined?" gets X, not a 2,000-line file. This is the granularity discipline we should copy.
3. **Dual-store: structure + semantics** — SQLite for exact facts (symbols, metadata), FAISS for fuzzy questions ("where is auth handled?"). Deterministic truth plus semantic overlay, each queried in its own mode.

## Where it is human-eyes-first (the gap we exploit)

- The **tree visualizations** (project structure tree, "enhanced tree with stats") are human browsing aids — an agent doesn't browse a tree, it queries. And critically: the tree it returns is **formatted text for reading, not a queryable graph**.
- **No edges.** It indexes symbols (nodes) but exposes no relationship data: no import graph, no call graph, no "who calls this" — dependency tracking is listed as a feature but not as structured edge data an agent can traverse. Our library's entire reason to exist is the edges.
- The `project_context_tool` returns an **overview narrative** (human-summary-shaped), not a schema the agent can join/filter against.
- Web dashboard exists but adds nothing the MCP tools don't already return — it's a comfort object for humans watching the agent work.

**The gap:** codebase-mcp proved agents want *tools over dashboards* and *symbols over files*, but it stops at nodes. Nobody gives the agent the graph — typed edges (calls, imports, inherits), traversable, aggregatable server-side. That's our white space.

## License — verified, and what it means for us

- **Apache License 2.0** — confirmed from the GitHub license field on the repo page and the `LICENSE` file at root; README also states "Open source - Apache 2.0 license, community-driven."
- **What is fair:** Apache-2.0 is fully compatible — we may **reuse its code with attribution** (keep license notices, note modifications). In practice its codebase is small and young; the realistic take is *patterns*, not wholesale code: the MCP tool-surface design, the dual-store (SQLite + FAISS) architecture, symbol-level retrieval granularity.
- **Verdict:** ideas AND code are fair game (with attribution). Still, treat it as a sketch to improve on, not a foundation to build on — 50 stars / 6 commits is not a hardened dependency.

## 2–3 concrete techniques to steal (incorporate, Apache-2.0, attribute)

1. **Tool-surface-first API design.** Don't ship a REST blob and hope; ship typed tools an agent calls directly: `get_symbol`, `get_callers`, `get_callees`, `dependency_subgraph`, `blast_radius`. Each tool returns the minimal structured unit. *How:* define our library's public contract as MCP tools (Conduit-style) first, HTTP/REST second, visuals third. Attribute: "tool-surface pattern inspired by danyQe/codebase-mcp (Apache-2.0)."
2. **Dual-store architecture: deterministic index + semantic overlay.** SQLite (or equivalent) holds the ground-truth symbol/edge graph; a vector index sits *beside* it for fuzzy queries. Never let embeddings be the source of structural truth. *How:* `nodes`/`edges` tables as primary store with exact queries; embeddings as an optional secondary search mode — mirroring their semantic/fuzzy/text/symbol four-mode search tool.
3. **Smallest-unit retrieval.** Their `read_symbol_from_database` (one symbol, not one file) is the right granularity instinct — extend it to edges: return a node *with its incident edges* as the default unit, and a subgraph only on explicit request. This is context budgeting by design. *How:* default tool responses are node+edges summaries with counts; full dumps require an explicit `expand` parameter.
