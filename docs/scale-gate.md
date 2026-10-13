<!--
Copyright (C) 2026 Nrupal Akolkar
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
