# ATTRIBUTION.md — codetopo incorporation ledger

**Formula (standing):** "Owned by Nrupal Akolkar · Built with Muse by Meta"
**Updated:** 2026-09-26

Every incorporation below is from observable behavior, public docs, or
compatibly-licensed code — nothing proprietary, nothing behind access controls.
This ledger records source, license, what was taken, and the commit where it
landed. **Reuse** rows may contribute code (with license notices preserved);
**clean-room** rows contribute behavior/docs understanding only — never code.

## Reuse with attribution (code fair game)

| Source | License | What we take | Landed |
|---|---|---|---|
| apache/echarts | Apache-2.0 | Direct dependency; `option` spec pattern as the agent↔visual contract; SSR SVG pipeline; `emphasis.focus:'adjacency'` → `neighborhood()` query primitive | — |
| marimo-team/marimo | Apache-2.0 | `DirectedGraph` coordinator pattern; `{defs,refs}`→edges pure rule; `descendants`/`ancestors`/`get_path`/topo-sort impact queries; validate-then-apply mutation discipline | — |
| danyQe/codebase-mcp | Apache-2.0 | Tool-surface-first API contract; dual-store (deterministic graph = truth, vector = fuzzy overlay); smallest-unit retrieval; full dumps opt-in | — |
| jc01rho/omo-herdr-dag | MIT | Event-sourced JSON snapshot schema; dependency-edges vs containment-links kept distinct; event-subscription update pattern | — |
| Arize-ai/openinference | Apache-2.0 | Typed-schema pattern: kind-scoped attributes, spec-first ontology design | — |
| observablehq/notebook-kit | ISC | Reactive cell-dependency graph as prior art for "code as queryable graph, visuals as byproduct"; notebook→static publish pipeline | — |
| observablehq/oss-analytics | ISC | CI-regenerated viz artifacts embedded in docs/PRs — maps-on-every-push | — |
| opensearch-project/OpenSearch-Dashboards | Apache-2.0 | Reference architecture: one backend serving structured query API + rich human visuals | — |
| langfuse/langfuse (core only) | MIT | Nested-span trace timeline viz as the analog for visualizing agent analysis runs. **Stay out of `ee/` dirs (commercial)** | — |

## Clean-room only (behavior/docs, NEVER code)

| Source | License | Why clean-room | What we take (understanding only) |
|---|---|---|---|
| Arize-ai/phoenix | Elastic 2.0 | Non-OSI license | Typed entity schema drives rendering/filtering; derived views from one model; MCP + server-side aggregation economics |
| code-yeongyu/oh-my-openagent | SUL-1.0 | Source-available, not open | Graph-as-workspace; named surgical mutation ops; LLM-emittable minimal node schema |
| felicityblueish/developer-roadmap | Custom restrictive | Restrictive terms | Model–renderer split; per-node content addressing; tier-as-data |
| ByteByteGoHq/system-design-101 | CC BY-NC-ND 4.0 | No derivatives | Human visual grammar for architecture maps (study only) |

## License preservation

- Apache-2.0 portions: keep `LICENSE`/`NOTICE` attributions on redistribution.
- MIT/ISC portions: keep copyright headers.
- This file is updated on every incorporation, before the code lands.

## Our own licensing (decided 2026-09-26)

- **Core engine:** AGPL-3.0 (same as RA).
- **Adoption-facing parts** (CODE_SCHEMA spec, client SDKs, API/tool contract): Apache-2.0.
