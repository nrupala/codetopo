<!--
Copyright (C) 2026 2351485 ALBERTA INC.
SPDX-License-Identifier: AGPL-3.0-or-later
Part of codetopo — Owned by [Nrupal Akolkar](https://nrupalakolkar.com) · Built with [Muse by Meta](https://muse.ai/join) — join Muse, then redeem code **101EEK** in Settings within 48 hours of joining and we both earn tokens.
-->

# Scale gate: 10,000 files

The final-shape audit's scale exit criterion: codetopo must index and
answer questions over a 10,000-file repository without pathological
time, memory, or — as it turned out — stack behaviour. This document
records the fixture, the method, and the measured results. Everything
below is reproducible from this repository.

## Fixture

`scripts/gen_scale_fixture.py <dir> [count]` generates a deterministic
tree (same count in, byte-identical tree out):

- 10,000 source files in 100-file directories; 60% Rust, 40% TypeScript.
- Every file defines `stepNNNNN` and `helpNNNNN`.
- `step{i}` calls `step{i+1}` — one call chain spanning the whole
  fixture (depth 10,000).
- Every 997 files, `step{i}` also calls `step{i-997}` — back edges
  that close one large cycle (the whole chain is a single SCC).
- Every file also imports the next file's module.

Indexing yields **46,001 nodes / 90,017 edges** (1 package, 10,000
modules, 10,000 files, 20,000 functions, 6,000 externals).

## The defect the gate caught

The first gate run **aborted with a stack overflow** inside the graph
builder's cycle detection. Tarjan's SCC was implemented recursively:
recursion depth equals call-chain depth, so a 10,000-deep chain
overflows a 2 MB thread stack (the test harness's default; also the
stack size of threads in many embedding hosts — MCP servers and
editor extensions included). The CLI binary survived only because a
process main thread gets 8 MB; a deeper repository would have
crashed it too, with an abort rather than an error.

Fix: `find_cycles` now runs an **iterative Tarjan** with an explicit
heap-allocated call stack. Traversal order (sorted starts, sorted
neighbours) and the cyclic rule are unchanged, so the SCC results are
identical — proven by the existing core test suite and by the fixture
producing the same single cycle diagnostic before and after. Depth is
now bounded by heap, not by the thread stack. All other whole-graph
operations (`descendants`, `ancestors`, `blast_radius`, `path`,
`neighborhood`) were already iterative (BFS/worklist).

## Measured results

Release build (`cargo build --release`), measured 2026-10-10 on a
2-vCPU AMD EPYC 9D25 VM with 8 GB RAM. Wall time and peak RSS of the
`codetopo` binary:

| Operation | Wall | Peak RSS | Notes |
|---|---|---|---|
| index 10,000 files | 3.5 s | 134 MB | tmpfs fixture; 8.6 s on a slower disk, same memory |
| index + snapshot + proof certificate | 6.9 s | 176 MB | snapshot JSON: 29.8 MB |
| stats (load graph + aggregate) | 1.7 s | 121 MB | |
| descendants / blast-radius / path (each, incl. graph load) | ~1.0 s | 121 MB | dominated by graph load |
| verify (audit chain) | < 0.1 s | 11 MB | `chain valid: true` |
| verify-proof (29.8 MB snapshot) | 0.5 s | 71 MB | `status: VALID`, certificate verified |

In-process timings (the gate test below, release mode): graph load
0.95 s; `descendants` of the chain head 52 ms (19,999 nodes);
`blast_radius` of the chain tail 126 ms (9,999 nodes); `path`
head → tail 307 ms (9,999 steps).

## The analytics gate (0.9.0, RA5)

The Rust analytics engine's measures run over the same corpus,
plus two synthetic extremes. Measured 2026-11-02, release builds,
on this machine class. Every corpus answer below was produced
TWICE with byte-identical output (SPEC §7: determinism at corpus
scale is a correctness property, asserted by the gate test).

The corpus projection (§9 dependency edges, external stubs
excluded, simple graph): **40,001 nodes / 30,008 directed edges**
from the fixture's 90,017 stored edges.

| Surface | Wall | Notes |
|---|---|---|
| rank pagerank | 0.49 s (CLI, incl. load); 0.10 s in-process | full 40,001-entry ranking |
| rank katz | 0.48 s | |
| rank eigenvector | 0.45 s | **absence** — power iteration does not converge on this graph within the method's 100 iterations; reported as an absence, never a partial vector (the designed answer) |
| rank closeness | 13 s | one BFS per node |
| rank betweenness | 33 s | exact Brandes, serial; the 50,000-node ceiling (owner decision 3) is finalized on this evidence — see below |
| communities | 0.91 s | 10,172 communities, Q = 0.9875 |
| topo-sort | 0.5 s | 30,031 groups, exactly 1 cyclic (the fixture's chain SCC) |
| render layered | 0.66 s | 19 MB spec |
| render force | 0.38 s | **declines by design**: 40,001 nodes condense to 30,031 components, above the 3,000 ceiling; the layered layout is the full-graph drawing |

**Dense-SCC stress** — a 2,000-node complete digraph (3,998,000
edges, one SCC), generated in-process by the core gate test:
pagerank 5.7 s · betweenness 35 s · closeness 150 s (the density
extreme for a per-node BFS) · communities 4.6 s (one community,
Q = 0 — correct for a complete graph) · topo-sort 2.1 s (one
group) · force layout 6.6 s. Katz reports an **absence**: the
spectral radius of a complete digraph is n−1 = 1999, so α·ρ ≈ 200
≫ 1 and non-convergence is mathematically forced — the absence is
the correct answer, and the gate asserts it as one.

**Condensed force determinism** — a 3,600-node graph of 600
six-node rings (above the 3,000-node direct ceiling, under it in
components): the force layout takes the condensed path and is
run-twice byte-identical in spec and SVG (asserted by the core
gate test).

**Betweenness ceiling, finalized at 50,000 nodes.** Evidence: the
40,001-node corpus completes in ~33 s; the density extreme (a
2,000-node complete digraph) in ~35 s; the RA1 probe series
(1,000 nodes 0.09 s; 5,000 2.6 s; 10,000 11 s) projects to
roughly a minute at the ceiling for realistic graphs. A node
ceiling cannot bound density — a declined answer therefore names
the graph's node count alongside the ceiling.

**Self-index cross-check.** Codetopo's own index (1,708-node /
5,387-edge projection) against the Python oracle: PageRank max
deviation 6.9e-18, betweenness 3.5e-18 (tolerances 1e-6 / 1e-9),
top-10s identical — and the hubs are the visibly right ones
(PageRank: `ApiError`, `EdgeKind`, `GraphBuilder`; betweenness:
the server `app`/`router`, `index_repo`, `call_tool`).

Running the analytics gates (both `#[ignore]`d; release mode):

```sh
cargo test --release -p codetopo-cli --test analytics_scale -- --ignored --nocapture
cargo test --release -p codetopo-core --test analytics_scale -- --ignored --nocapture
```

## Running the gate

The gate is a deliberately-run integration test, `#[ignore]`d so CI
stays fast:

```sh
cargo test --release -p codetopo-cli --test scale_gate -- --ignored --nocapture
```

It regenerates the fixture in Rust (the same scheme as the Python
generator), indexes it, reloads the graph, and asserts: all 10,000
files indexed with zero failures, the cycle diagnostic present, and
the long-pole queries returning end-to-end results — under generous
time tripwires (300 s index, 60 s per query) that exist to catch
pathological regressions, not to claim performance records.
