# Stars Scan — Additional code-viz-adjacent repos in nrupala's stars

**Date:** 2026-09-26
**Scope:** 237 starred repos scanned (full list via `GET /users/nrupala/starred`).
**Already covered (not re-surveyed):** apache/echarts, code-yeongyu/oh-my-openagent, felicityblueish/developer-roadmap, marimo-team/marimo, Arize-ai/phoenix, danyQe/codebase-mcp.

## INCLUDED — recommended for the agent-optimized code-viz design study

### 1. observablehq/notebook-kit
- **Visualizes:** Observable Notebook cells (code + charts) compiled into static websites.
- **License:** ISC (GitHub API confirms).
- **Why relevant:** Observable's reactive runtime is a live cell-dependency dataflow graph — conceptually the closest existing system to "code as a queryable graph that renders visuals as a byproduct." notebook-kit is the build path from interactive notebook viz to deployable static artifact — the exact publish pipeline our library will need.

### 2. observablehq/oss-analytics
- **Visualizes:** npm download-count dashboards (Observable Plot charts), regenerated daily by GitHub Actions and embedded as images in GitHub READMEs.
- **License:** ISC.
- **Why relevant:** The pattern matters more than the content: viz produced by CI as versioned build artifacts, embedded into docs/PRs automatically. Our code-viz library should aim for exactly this — `main` gets fresh dependency/call-graph maps on every push, no human opening a dashboard required.

### 3. opensearch-project/OpenSearch-Dashboards
- **Visualizes:** Search/analytics data via full dashboard platform (charts, maps, tables, query builders).
- **License:** Apache-2.0.
- **Why relevant:** Reference architecture for the human-facing layer *on top of* an agent-first data model. Its query-first dashboards (Vega/Vega-Lite support included) show how to serve both a structured query API and rich visuals from the same backend — the "structured data first, visuals as a byproduct" split we want.

### 4. langfuse/langfuse
- **Visualizes:** LLM/agent execution traces (nested span timelines), eval scores, prompt versions, cost/usage analytics.
- **License:** MIT for the open-source core (`ee/`, `web/src/ee/`, `worker/src/ee/` under a separate commercial license).
- **Why relevant:** Trace-tree visualization is the closest existing analog to visualizing an *agent's execution over code* — how Langfuse renders nested spans as queryable timelines is a pattern to borrow directly for agent-driven code-analysis runs. Complements Phoenix (already covered) rather than duplicating it: two independent design languages for the same problem class.

### 5. ByteByteGoHq/system-design-101
- **Visualizes:** System architectures (boxes, layers, arrows) as educational diagrams explaining complex systems in simple visual terms.
- **License:** CC BY-NC-ND 4.0 — ⚠️ **not software-OSS; attribution required, non-commercial, no derivatives.** Grammar can be *studied*; diagrams/derivatives cannot be shipped.
- **Why relevant:** Pure visual-grammar reference: it's what 90k-star worth of engineers expect an architecture map to look like. Our agent-optimized library must emit data agents can query — but when it renders for humans, it should speak this visual dialect.

## REJECTED — considered, not included

1. **cordiverse/cordis** (MIT) — Suspected candidate. It is a TypeScript plugin runtime (Koishi chatbot lineage; powers DeepSeek's agent harness), "meta-framework of spatiotemporal composability" of *component lifecycles* — zero visualization. Interesting for agent harnesses, out of scope for code-viz.
2. **any4ai/AnyCrawl** (MIT) — Node crawler turning websites into LLM-ready data. Data ingestion, no viz layer. Rejected: wrong layer of the pipeline.
3. **EncyclopediaWorld/howaiworks** (Apache-2.0) — Interactive educational viz of AI history (50 model demos). Human-facing explainers, not code-structure visualization. Rejected: wrong domain.
4. **HannahRitchie/energy-use-comparisons** (no license, 9 stars) — OWID-style interactive dataviz for energy data by an Our World in Data researcher. Rejected for *this* study (code-viz); flag as a possible input to the separate human dataviz study track.
5. **plotly/dash-oil-and-gas-demo** (no license) — A single demo app; Plotly Dash itself isn't starred. Rejected: demo, not a viz system.
6. **gurgeous/tennis** (MIT) — Stylish CSV tables in terminal. Text formatting, not visualization. Rejected.
7. **nilbuild/developer-roadmap** — Appears to be the roadmap.sh original under a different handle; content coverage duplicates felicityblueish/developer-roadmap (already confirmed). Rejected: duplicate.

**Note on the rest:** the remaining ~225 stars were swept (LLM/agent frameworks, ML libs, crypto, infra, learning resources, lists) — nothing else visualized anything.
