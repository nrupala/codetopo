# Survey: felicityblueish/developer-roadmap (fork of kamranahmedse/developer-roadmap — roadmap.sh)

Surveyed 2026-09-26 via GitHub pages/READMEs/docs + third-party architecture analyses (openSkilTree analysis, DeepWiki, fork AGENTS.md). No live-browser interaction.

## What it renders
- **One interactive skill-tree diagram per landing page** (e.g. "Python Developer"): boxes connected by lines; the diagram itself is the product and the marketing (shareable PNG screenshots).
- Rendered with **React Flow (@xyflow) via `@roadmapsh/editor`**, a *private* workspace npm package. Local dev swaps in `@roadmapsh/dummy-editor` (a placeholder) — the real editor is not published for reuse. A visual editor exists at draw.roadmap.sh.
- Node types: `topic | subtopic | todo | section | title | label | button | paragraph | legend | vertical | horizontal`.
- **Tier markers**: solid outline = core/essential, dashed = recommended, dotted = optional — encoded via `strokeDasharray`.
- Interaction model: click a node → side panel with curated topic content and resource links (typed: `@official@`, `@opensource@`, `@article@`, `@course@`, `@podcast@`, `@video@`, `@book@`). Progress tracking per node (done / learning / skip, nanostores + `resource-progress.ts`). Also best-practices trees, question groups (quizzes), an AI-tutor chat, and PDF/JSON export per roadmap.

## Its data model
Each roadmap is a directory `src/data/roadmaps/{id}/` with:
- `{id}.json` — **React Flow graph definition**: `{ nodes: [...], edges: [...] }`, served publicly at `/jsons/roadmaps/{id}.json`.
- `{id}.md` — frontmatter metadata (`jsonUrl`, `pdfUrl`, `renderer: 'editor'`, `dimensions`, `seo`, `relatedRoadmaps`, `tags`).
- `content/{slug}@{nodeId}.md` — per-node markdown content files.

Node schema: `id` (nanoid), `type`, `position: {x, y}` (**absolute pixel coordinates**), `data: {label, href, style}`, `width/height`, `zIndex`.
Edge schema: `id`, `source`, `sourceHandle`, `target`, `targetHandle`, `type: 'smoothstep'`, `style: {stroke, strokeWidth, strokeDasharray}`, `data: {edgeStyle: 'solid'|'dashed'}`.

**Key fact (from a third-party architecture analysis, independently consistent with the JSON frontmatter): the graph JSON is a *visual layout document*, not a semantic model.** No `parentId`, no typed relationships; edges are visual connectors; hierarchy exists only as background "section" rectangles. The `renderer: 'editor'` field at least decouples model from renderer.

## What makes it good (3 strengths)
1. **One-glance full-discipline diagram** — the entire domain visible at once; screenshot-as-marketing is a proven distribution mechanism.
2. **Tier system per node** (solid/dashed/dotted) — a cheap visual encoding of priority that cuts "where do I start" paralysis.
3. **JSON-driven content/renderer split + per-node content addressing** — model decoupled from renderer (`renderer` field), each node anchored to its own docs/resources file.

## Where it is human-eyes-first (our gaps to exploit)
- **Absolute pixel layout baked in**: positions are placed by human editors in draw.roadmap.sh. An agent consuming structure doesn't care about pixels — it wants relationships, and layout should be computed, not stored.
- **Edges carry no semantics**: "solid vs dashed" encodes core-vs-optional only as a *stroke pattern* — a visual hack, not queryable metadata. There is no edge type like `requires`, `unlocks`, `alternative-to` that an agent could reason over.
- **Section rectangles instead of structure**: grouping is a background box, not a parent/child or namespace relation. No containment, no namespacing, no query ("show me everything under X").
- **The whole product is a page for eyes**: pan/zoom/click panels, shareable screenshots, SEO frontmatter. Nothing in the data model is designed to be consumed by a program that wants to *reason* about a codebase's architecture.

## License — ⚠️ RESTRICTIVE, NOT REUSABLE
- License file is a **custom, non-standard license** (verified from the fork's `license` file, identical to upstream): "Everything including text and images in this project are protected by the copyright laws. You are allowed to use this material for personal use but are **not allowed** to use it for any other purpose including publishing the images, the project files or the content in the images in any form…". GitHub's license field reports it as non-standard ("other").
- **Verdict**: zero code reuse permitted. Observable behavior + public docs are fair game for **clean-room reimplementation only** — and even then, do not lift the roadmap content itself. This is a *study reference*, not a dependency.

## 3 concrete ideas to incorporate (clean-room from behavior/docs)
1. **Tiered node/edge metadata as first-class data** (core / recommended / optional): encode priority as queryable attributes on nodes and edges (`data.edgeStyle` made semantic), never as stroke-dash patterns. An agent can then answer "what's the minimum viable path through this module?" — humans get their visual tiers as a byproduct.
2. **Per-node content addressing**: every node in our graph carries a stable id + a link to its documentation/resources (`content/{slug}@{nodeId}.md` pattern). Architecture maps where each node is a doc anchor turn the graph into a navigable knowledge graph, not just boxes.
3. **Model–renderer decoupling via a `renderer` discriminator**: store one canonical graph JSON; renderers (agent JSON API, SVG, Mermaid, React Flow) are interchangeable consumers. Roadmap.sh's `renderer: 'editor'` field is exactly this seam — steal the pattern, make the *agent query API* the primary consumer instead of the visual editor.
