<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Deterministic render — layout spec + SVG

`codetopo render --db <index.db> [--out spec.json] [--svg map.svg]`

Rendering is a pure function of the stored graph: **same graph in,
same bytes out** — across runs, machines, and builds of the same
version. No clocks, no randomness, no hash-iteration order anywhere
on the path. That property is the point: a rendered map can be
committed, diffed, hashed, and attached to a proof like any other
codetopo artifact.

## The layout spec (JSON)

```json
{
  "layout_version": 1,
  "width": 3,
  "height": 2,
  "nodes": [
    {"id": "pkg::a", "label": "a", "kind": "function",
     "tier": "optional", "x": 0, "y": 0}
  ],
  "edges": [
    {"from": "pkg::a", "to": "pkg::b", "kind": "calls"}
  ]
}
```

- `layout_version` — consumers key on this. The rules below are
  frozen for version 1; a rule change is a new version.
- Coordinates are **grid cells** (integers): `x` is the column
  within a layer, `y` is the layer. Renderers scale cells to
  pixels as they please; the built-in SVG uses a fixed table.
- `nodes` is sorted by `(y, x)`; `edges` by `(from, to, kind)`.
  Edges are deduplicated on `(from, to, kind)` — the graph
  distinguishes call sites by line, a drawing cannot.
- `kind`/`tier` use the CODE_SCHEMA vocabularies verbatim.

## The layout rules (v1)

1. **Dependency edges layer; containment does not.** Layering runs
   over calls, imports, inherits, implements, reads and writes.
   Contains/defines/references edges are drawn but never move a
   node: hierarchy depth is not dependency depth.
2. **Condense cycles first.** Strongly connected components
   (iterative Tarjan) collapse to single DAG nodes; members of a
   cycle share one layer — a cycle has no honest top or bottom.
3. **Longest-path layering.** `layer(c) = 0` if nothing depends on
   `c` (no incoming dependency edges from other components), else
   `1 + max(layer of the components that depend on c)`. Entry
   points sit at the top; depth grows toward the most-depended-on
   code, like a call-graph flame.
4. **In-layer order is by id.** Components order by their smallest
   member id; members order by id. Columns are assigned left to
   right in that order. Ties are impossible — ids are unique.

## Scope — `full` (default) and `structure`

`codetopo render --scope structure` (HTTP
`GET /v1/graphs/{id}/render?scope=structure`, MCP `codetopo_render`
with `scope: "structure"`) lays out the graph's **structural
skeleton** instead of every node:

- **Kept kinds:** package, file, module, class, interface, doc —
  the kinds a reader navigates by.
- **Edges:** only the edges that show how the system is
  *organized* — containment, imports, and type hierarchy
  (inherits/implements). Their endpoints roll up to the nearest
  kept ancestor along containment edges, so an import declared
  inside a file appears between the kept nodes that contain its
  endpoints. Edges whose endpoints roll up to the same node
  drop; a node with no kept ancestor (an external stub is
  contained by nothing) drops out. Behaviour edges (calls,
  reads, writes) are excluded on purpose: rolled up to file
  granularity they become long cross-cutting diagonals that
  bury the skeleton — they remain in the full scope and in
  every query/stats surface.
- **Layers are containment depth.** The full scope layers by
  dependency depth; a structure map layered that way collapses
  into one long row (structural nodes have almost no dependency
  edges between them). The structure scope therefore layers by
  the containment tree — package over files over modules over
  classes — while imports and hierarchy edges are drawn without
  layering. Within a layer, nodes order by id, as everywhere.
- **The spec stays v1.** The answer is a layout spec exactly as
  above, computed over the projected node/edge universe —
  deterministic in the same way. The shared JSON answer records
  `"scope": "structure"` and `"source_nodes"` (the full graph's
  node count) so a consumer can never mistake the map for the
  whole graph; the SVG carries the projected counts in its
  `<title>`.
- **Layered only.** The force layout always takes the full
  graph; asking it for the structure scope is an error naming
  the reason, never a silent substitution.

Why this exists: the full layout of a real graph is a faithful
but unreadable drawing — thousands of symbol boxes at one
dependency depth. The structure scope is the map a person (or an
agent writing about the system) actually reads: the machine's
shape, with every edge in it still a measured edge of the graph,
rolled up — never invented.

## The SVG

`render_svg` consumes the spec alone (any consumer can do the
same). Fixed geometry: node boxes 190×30 px on a 214×56 px grid,
16 px margin; a fixed per-kind (fill, stroke) table; per-edge-kind
stroke/dash table; straight centre-to-centre lines drawn under the
nodes with one shared arrow marker. Labels truncate at 26 chars
with an ellipsis; the full node id rides in a `<title>` element.
All label/id text is XML-escaped. The document carries no
timestamp and no generator string beyond its `<title>` counts.

## Layout spec v2 — the force layout (0.9.0)

`render --layout force` (HTTP `GET /v1/graphs/{id}/render?layout=force`,
MCP `codetopo_render` with `layout: "force"`) emits layout spec
**v2**: the same self-contained discipline as v1 — geometry plus
identity, no colours or grouping baked in — with float canvas
coordinates on a 1000×1000 canvas. The layout is
Fruchterman–Reingold (1991), implemented in-house and made
deterministic the same way v1 is: no randomness (initial placement
is the circle in ascending id order), a fixed iteration count
(80) and linear cooling schedule (from 100 canvas units), both
recorded in the spec's `params` alongside the optimal distance k,
the source node count, and whether condensation ran.
Single-threaded accumulation in index order throughout; the same
graph produces byte-identical spec and SVG on any platform.

Springs act on v1's dependency adjacency over the same node
universe and drawing edges. Above 3,000 nodes the layout runs over
the SCC-condensed graph (each spec node is a component:
`id`/`label`/`kind`/`tier` from its smallest member, `members`
recording the component size, drawn larger by member count);
above 3,000 components too, the surface declines with a stated
reason naming the ceiling — the layered layout remains the
full-graph drawing. SVG for v2 mirrors v1's styling (same kind
fills and edge strokes) over the float coordinates.

## Size behaviour

Output is linear in the graph (`nodes + edges` elements). A
10,000-file workspace produces a spec of ~46k nodes — valid, and
large; for human viewing, use `--scope structure` (above) or
render a stored neighbourhood-sized index rather than expecting
a multi-megabyte SVG to be readable. Layout time on the scale
fixture is dominated by the graph load, not the layout.

## Tests

- `codetopo-core` render unit tests: layering direction, cycle
  sharing, unique placement, byte determinism of spec + SVG,
  escaping/truncation, empty canvas, the structure scope's
  roll-up (structural kinds only, no self-edges, determinism),
  and `full` scope equalling the historic layout.
- `codetopo-cli` render integration tests: the shipped binary
  renders a fixture twice to byte-identical files, and the stdout
  default carries the spec.
