# apache/echarts — survey for agent-optimized code-viz library

Repo: https://github.com/apache/echarts · ~67.4k stars · active (Apache project, 10k+ commits)
Role in our design: **the rendering engine candidate** — the pixel/SVG byproduct layer, not the data layer.

## What it renders

- **Chart/graph types:** 20+ series types including `graph` (network), `sankey`, `tree`, `treemap`, `sunburst`, `graphGL` (WebGL, via ECharts-GL extension), plus bar/line/pie/scatter/heatmap/candlestick/geo.
- **`series-graph` data model** (the one we care about):
  ```js
  { type: 'graph',
    data: [ {id, name, value, symbolSize, category, x, y, itemStyle, label} ],  // nodes (alias: nodes)
    links: [ {source, target, value, lineStyle, label} ],                       // edges (alias: edges)
    categories: [ {name, itemStyle} ],   // node grouping / legend
    layout: 'force' | 'none' | 'circular',
    force: { repulsion, gravity, edgeLength, layoutAnimation },
    roam: true,             // pan/zoom
    draggable, cursor,
    emphasis: { focus: 'adjacency' | 'none' },  // hover-highlight neighbors
    edgeSymbol, edgeLabel, symbolSize, ... }
  ```
- **Layouts:** force-directed (`force` with repulsion/gravity/edgeLength), `circular`, or `none` (pre-computed x/y — the agent-friendly path).
- **Interaction model:** event-driven (`chart.on('click'|'mouseover', {dataType:'node'|'edge'})`), declarative re-render via `setOption()` (merge semantics), components (tooltip, legend, visualMap, toolbox, dataZoom) that compose with series.

## Data model — how data is structured, fed in, queried

- **The `option` object:** a single declarative JSON-ish spec — `{title, legend, tooltip, dataset, series[], ...}`. Everything is data; no imperative drawing code. `setOption(option)` applies it; `getOption()` reads it back.
- **`dataset`:** a shared tabular source (`source: [[...], ...]` or row objects) that multiple series map from via `seriesLayoutBy` (`'row'`/`'column'`) + `encode` mappings. Includes **dataset transforms** (`filter`, `sort`) chained as data pipelines. This is the closest thing ECharts has to a queryable data layer — declarative, separable from presentation.
- **Graph feeding:** nodes/edges arrays are fed directly (not via dataset, though transforms can pre-process). Edges reference nodes by index **or** name/id. No built-in graph queries (no neighbors(), shortest-path, subgraph) — once rendered, the graph exists only as display lists.
- **Programmatic query surface:** `chart.getModel()`, `getData()`, component/series APIs exist but are designed for driving the view, not for structural graph analysis.
- **SSR path (agent-relevant):** zero-dependency server-side rendering since v5.3:
  ```js
  const chart = echarts.init(null, null, { renderer: 'svg', ssr: true, width, height });
  chart.setOption(option);
  const svgStr = chart.renderToSVGString();  // Node only, no DOM
  ```
  plus `ssr/client` hydration for lightweight interactivity. This means: **agents can render human visuals server-side with no browser**, and (more importantly) the *same `option` spec* can be generated from an agent-held graph model as a pure data→spec transform.

## What makes it good (distinctive strengths)

1. **Declarative `option` = a spec language.** A whole interactive visualization is one serializable JSON document. An LLM/agent can *generate* it directly — this is the cleanest contract for "agent emits structure, library turns it into a picture."
2. **Batteries-included interactivity for free.** Roam, tooltip, legend toggling, emphasis-on-adjacency, animations — all come from the spec, no custom code. For the human byproduct layer, nothing else is as complete.
3. **SSR + SVG string output.** Node-only rendering to SVG text — fits an agent pipeline that produces artifacts (reports, docs, PR descriptions) without a browser.

## Where it is human-eyes-first (the gap we exploit)

- **No queryable graph.** After `setOption`, the nodes/edges are display lists — no `neighbors(node)`, `subgraph(predicate)`, `k-hop`, `paths(a,b)`, cycle detection. An agent consuming code structure needs *answers*, not pixels; ECharts gives pixels.
- **Force layout is a black box for aesthetics.** The physics sim optimizes for readable-on-screen layouts; it does not expose or care about semantically meaningful orderings (topological layering, call-depth layering, module grouping by package). An agent wants *deterministic, semantic layouts* (layered DAG by dependency direction), which ECharts only approximates via `layout:'none'` + manual x/y.
- **Interaction is pointer-oriented.** Hover, click, drag, roam — all assume a mouse and eyeballs. There is no equivalent of "programmatically expand node X's callees" except via new `setOption` calls driven by human clicks.
- **Emphasis/labels are cosmetic.** `label.formatter`, `visualMap` coloring serve human scanning; nothing is emitted as structured data (e.g., "these 5 nodes are critical-path") for downstream agent reasoning.
- **Scale strategy is rendering tricks** (progressive rendering, canvas dirty-rect) — aimed at frames-per-second, not at the agent problem: summarization/aggregation of the graph itself (collapse package → module → file).

## License

**Apache-2.0** (confirmed: GitHub repo license field + README "available under the Apache License V2" + `LICENSE` file at root).

- **Fair to incorporate:** code reuse with attribution (Apache-2.0 is fully compatible). Concretely: we can depend on `echarts` as our SVG/Canvas renderer, vendor or fork the `series-graph` force-layout implementation, reuse `dataset` transform code, or adapt the `option` schema shape — all with attribution "Owned by Nrupal Akolkar · Built with Muse by Meta" plus Apache-2.0 notices for the ECharts portions.
- **Clean-room not required** for anything in this repo. Only need the standard Apache-2.0 `LICENSE`/`NOTICE` preservation on redistributed portions.

## Steal-worthy ideas (incorporate)

1. **The `option` spec pattern — declarative chart spec as the agent↔visual contract.** Our library's *human-visual byproduct* should be one serializable spec document that an agent can generate in a single pass, exactly like `setOption`. Steal the shape: top-level `series[]` of typed renderers + shared `data`/`dataset` + `emphasis`/`tooltip` styling blocks. This is Apache-2.0, so we can even adapt field names 1:1 with attribution.
2. **`emphasis.focus: 'adjacency'` → "neighborhood query as a primitive."** ECharts turns "show me this node's neighborhood" into a first-class interaction. In our agent API, make it a first-class *query*: `graph.neighborhood(nodeId, depth)` returning a subgraph — then reuse the same adjacency logic for the human highlight. One primitive, two consumers.
3. **SSR SVG-string pipeline (`ssr:true` + `renderToSVGString()`).** Steal the architecture: the graph model lives server-side/agent-side; the renderer produces a portable SVG string with zero browser dependency. Design our library so the *entire* agent data path (build → query → aggregate) runs in Node with no DOM, and only the final "export picture" step touches a renderer. We can literally depend on echarts' SSR for v1 of the visual byproduct.

---
Surveyed 2026-09-26 · sources: repo README, echarts.apache.org docs, echarts-handbook (dataset, SSR, event docs).
