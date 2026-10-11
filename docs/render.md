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

## The SVG

`render_svg` consumes the spec alone (any consumer can do the
same). Fixed geometry: node boxes 190×30 px on a 214×56 px grid,
16 px margin; a fixed per-kind (fill, stroke) table; per-edge-kind
stroke/dash table; straight centre-to-centre lines drawn under the
nodes with one shared arrow marker. Labels truncate at 26 chars
with an ellipsis; the full node id rides in a `<title>` element.
All label/id text is XML-escaped. The document carries no
timestamp and no generator string beyond its `<title>` counts.

## Size behaviour

Output is linear in the graph (`nodes + edges` elements). A
10,000-file workspace produces a spec of ~46k nodes — valid, and
large; for human viewing, render a stored neighbourhood-sized
index or expect a multi-megabyte SVG. Layout time on the scale
fixture is dominated by the graph load, not the layout.

## Tests

- `codetopo-core` render unit tests: layering direction, cycle
  sharing, unique placement, byte determinism of spec + SVG,
  escaping/truncation, empty canvas.
- `codetopo-cli` render integration tests: the shipped binary
  renders a fixture twice to byte-identical files, and the stdout
  default carries the spec.
