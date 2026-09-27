# Survey: Arize-ai/phoenix

**Repo:** https://github.com/Arize-ai/phoenix
**What it is:** AI observability & evaluation platform — tracing, LLM-as-judge evals, datasets, experiments, playground. Built on OpenTelemetry + OpenInference.
**Scale:** ~11.6k stars, 10k+ commits, releases multiple times/week (v18.x era, 2026).

## What it renders (chart/graph types, interaction model)

- **Trace waterfall / span tree** — the flagship view. A trace (one execution) expands into a nested span tree with start-time/duration bars, the classic distributed-tracing Gantt. This is the core visual.
- **Agent graph view** — distinctive: Phoenix converts the waterfall into a **typed node-edge graph** of agent reasoning steps and transitions (LLM → tool call → LLM loops as clickable nodes). Reportedly turns hours-long agent debugging into a fast scan. This is the closest thing in the ecosystem to an "agent understanding" diagram.
- **Span detail panels** — click any LLM span: full message history, token counts, model params; click TOOL spans: arguments and return values.
- **Metric charts** — a resizable per-table chart strip pinned above the spans/traces/sessions tables (2026 release).
- **Datasets / experiments / playground** — versioned test sets, experiment comparison, interactive prompt testing (human workflow UIs).
- **Sessions view** — multi-turn conversation timelines grouped by session.
- **Interaction model:** a web UI dashboard at port 6006 (React frontend + GraphQL/REST backend). NEW (2026): a **Remote MCP Server** at the `/mcp` endpoint plus `px` CLI — coding agents (Claude Code, Cursor) connect directly and query traces, datasets, experiments *without* the browser. Also an embedded agent, **PXI**, for trace debugging.

## Data model (how data is structured, fed in, queried)

- **Ingest:** OpenTelemetry OTLP spans, auto-instrumented via **OpenInference** (Apache-2.0 companion repo) semantic conventions. Span *kinds* are typed: `CHAIN`, `RETRIEVER`, `LLM`, `TOOL`, plus `llm.*` / `tool.*` attribute namespaces. Instrumentation is per-framework (OpenAI Agents, LangGraph, Vercel AI SDK, Mastra, CrewAI, LlamaIndex, DSPy…) — agent code stays clean, capture is automatic.
- **Storage:** spans/traces/annotations in SQLite (local default) or Postgres; relational schema with `spans`, `traces`, `projects`, `span_annotations` tables.
- **Query surface:**
  - GraphQL API (UI-driven) and an expanding OpenAPI REST interface (`arize-phoenix-client`).
  - **MCP server (remote):** `describeSqlSchema` + `executeSql` — read-only SQL tools so an agent can query the telemetry DB directly, like a SQL console.
  - **Code mode:** instead of per-tool chat calls, the agent writes a short Python program executed server-side in a restricted Monty sandbox; the program calls the MCP tools *inside* the sandbox and only the final value returns to the model. Their benchmark: same question answered in 7 turns / $0.23 vs 89 turns / $10.35 — because 3,624 spans never left the database.
- **Evals layer:** LLM-as-judge scorers attached to spans/traces as annotations; experiments diff runs over versioned datasets.

## What makes it good (distinctive strengths)

1. **Typed span schema drives everything** — because spans carry OpenInference semantic types (not just names/timestamps), Phoenix renders, filters, and graphs by *meaning* (LLM vs TOOL vs RETRIEVER). Data model and visualization are one design.
2. **Derived representations** — the same trace yields both a waterfall (temporal) and an agent graph (structural) automatically. One data model, multiple views, derived rather than hand-built.
3. **Agent-first query layer on top of a human-first UI** — the Remote MCP server + code mode is the rare mainstream case of a product adding an *agent-native* data interface to a dashboard product, and their own benchmark quantifies the token/cost win of server-side aggregation.

## Where it is human-eyes-first (the gap we exploit)

- The waterfall/tree, color-coded span kinds, icon badges, metric charts, and session timelines are all **perceptual aids** — they help a human *scan and notice*. An agent doesn't scan; it needs the typed graph directly, with edges and attributes as data, not as pixels.
- The agent graph view is still **a picture in a browser** — you click nodes; there's no "give me the agent-step graph as JSON" contract. The visual is the API.
- Phoenix traces **runtime executions**, not code structure. There is no static symbol graph (what calls what, what imports what) — for *code* visualization this whole data source doesn't exist in their world.
- Code mode is token-clever but bound to **telemetry SQL**; there's no equivalent "run my graph analysis where the graph lives" for static structure.
- Playground/experiment UIs are comparison workflows built for human judgment — an agent wants the comparison as a diff-able data structure.

**The gap:** Phoenix is proof that a typed data model + MCP/agent API *plus* server-side aggregation is the right architecture — but nobody has built the equivalent whose substrate is the *codebase itself* (symbols, calls, imports) rather than runtime spans.

## License — verified, and what it means for us

- **Phoenix itself: Elastic License 2.0 (ELv2)** — verified verbatim from `https://github.com/Arize-ai/phoenix/blob/main/LICENSE` (GitHub license field shows "Other (NOASSERTION)"). ELv2 is source-available but **not OSI-approved**; it forbids offering the software as a hosted/managed service.
- **What is fair:** observable behavior, public docs, and the *ideas* (typed span conventions, derived graph views, MCP + code-mode query pattern) are fair game for **clean-room reimplementation only**. Do **not** reuse, copy, or adapt Phoenix code — including its frontend graph components, span-processing, or server code.
- **Separately licensed, Apache-2.0 companions (code may be reused with attribution):**
  - `Arize-ai/openinference` — the span semantic conventions (the typed-schema *pattern* we want to mirror for code: typed node kinds, typed edge kinds, typed attributes). Apache-2.0.
  - `Arize-ai/coding-harness-tracing` (formerly arize-harness-tracing / arize-agent-kit) — Apache-2.0.
  - Note: the in-repo `docs/phoenix/skill.md` carries an MIT frontmatter, but that covers the doc snippet, not the platform. Treat all Phoenix *code* as ELv2.
- **Verdict:** ideas in, code out. Everything we take from Phoenix is reimplemented from observed behavior/docs.

## 2–3 concrete techniques to steal (clean-room)

1. **Typed entity schema as the foundation.** OpenInference's move — every span carries a *kind* (CHAIN/RETRIEVER/LLM/TOOL) plus kind-scoped attributes — is the single most portable idea. Our library should define a code-native analog: node kinds (`module`, `function`, `class`, `method`, `interface`, `external`) and edge kinds (`imports`, `calls`, `inherits`, `implements`, `defines`) with kind-scoped attributes (e.g. `calls` edges carry call-site line, argument shapes; `imports` edges carry binding names). Type the model first; every view derives from it. *How:* write a `CODE_SCHEMA.md` spec (our OpenInference equivalent) before any renderer.
2. **Derived views from one model, not parallel models.** Phoenix auto-derives the agent graph from the trace tree. We should auto-derive: symbol tree → dependency graph → call graph → "blast radius" subgraph, all computed from one indexed model, never maintained separately. *How:* one canonical store (SQLite-style relational: `nodes`, `edges`, `annotations`), with view functions that project it.
3. **MCP + code-mode query economics.** Expose the graph over an MCP endpoint with read-only query tools (`describeSchema`, `executeQuery`), and support server-side aggregation programs ("give me the top-10 fan-out functions," not the whole graph) so agents never drag a 50k-node graph into context. Phoenix's own 89-turns→7-turns benchmark is the argument. *How:* design the API so the *default* answer is an aggregate/summary, with explicit opt-in to full subgraph dumps; document the token-cost rationale.
