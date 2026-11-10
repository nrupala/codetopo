<!--
Copyright (C) 2026 2351485 ALBERTA INC.
SPDX-License-Identifier: AGPL-3.0-or-later
Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
-->

# Codetopo.jl — the Julia surface of codetopo

A Julia package that wraps the codetopo Rust core through its stable C
ABI (`libcodetopo_c`, crate `codetopo-c`), called directly with `ccall`
from Base Julia. No other bridge is involved: jlrs is convenience only
and is **not** used — the surface stands on Julia itself plus the cdylib,
exactly as the diaspora buildability proof prescribes.

The boundary is CODE_SCHEMA JSON (CODE_SCHEMA §7): every answer is the
same JSON the CLI, HTTP, MCP and C surfaces produce for the same
question. This package is **not registered** in the Julia General
registry and is not published anywhere; it lives in this repository at
`surfaces/julia/`, outside the Rust workspace.

## JSON handling — the fallback in effect ⚠

The intended design parses answers with JSON.jl. **That dependency is
not in effect in this build**: the Julia General registry was
unreachable from the build sandbox (Pkg stalled installing the registry;
`pkg.julialang.org` and the registry's GitHub route both timed out — the
probe record is in the lane log that accompanied this surface), so, per
the build brief's fallback clause:

- every query/index function returns the **raw JSON string**;
- the package has **no external dependencies** (stdlib `Libdl` only), so
  `Pkg.instantiate` needs no network;
- the conformance suite checks answers by **raw-string extraction**,
  mirroring the C ABI lane's own conformance helpers.

If you have JSON.jl available, `JSON.parse(Codetopo.stats(g))` and
friends work unchanged. Absent measurements stay absent in the string:
a metric that cannot be measured arrives as `"measured":false` with a
reason, never as `0`.

## Setup

Build the native library once, from the repository root:

```sh
cargo build -p codetopo-c --release   # produces target/release/libcodetopo_c.so
```

Point the package at it (or rely on the default search — see
`Codetopo.library_path()`):

```sh
export CODETOPO_C_LIB="$PWD/target/release/libcodetopo_c.so"
```

Default search, in order: `$CODETOPO_C_LIB`, then
`<repo>/target/{release,debug}/libcodetopo_c.*`, then
`<package>/lib/`, then the system library path.

## Use

```julia
using Codetopo

Codetopo.codetopo_version()   # "0.9.1" — the bound core
Codetopo.schema_version()     # "1.2"   — CODE_SCHEMA

report = Codetopo.index_repo("path/to/repo", "index.db"; package="mypkg")
g = Codetopo.open_db("index.db")
Codetopo.stats(g)                          # raw JSON string
Codetopo.rank(g; by="pagerank", limit=10)  # `by` is the only metric key
Codetopo.blast_radius(g, "mypkg::core::Engine")
snap = Codetopo.snapshot(g)
g2 = Codetopo.load_snapshot(snap)
Codetopo.verify(g2)                        # audit chain, as JSON
close(g); close(g2)
```

Handles free themselves via a finalizer; `close` is explicit and
idempotent. Every string the C ABI returns is copied and then freed
with `codetopo_free_string` inside the wrappers — no leaks by
construction.

## Strictness

Wrappers validate their keyword arguments: an unknown keyword throws
`ArgumentError` **naming the key**. `rank` accepts `by=` only — a
`metric=` keyword throws at the Julia layer, before any C call (the
0.9.1 lesson: a surface that silently ignores a misspelled metric
parameter re-ships the defect). C-layer failures throw
`CodetopoError(code, message)` with the stable error-code vocabulary of
the other surfaces.

## Tests

The Test-stdlib suite in `test/runtests.jl` runs the shared diaspora
conformance battery (F1–F5) through `ccall`: the oracle goldens at the
core's own tolerances, the quarry known-answer package (26 nodes /
33 edges / 2 diagnostics), the Aarona sibling tripwire (imports 7/7,
calls 7/7, `cyclic_groups: 2`, `rank` key discipline), the lifecycle
round-trip compared by graph content, and determinism.

```sh
CODETOPO_C_LIB=/path/to/libcodetopo_c.so \
  julia --project=surfaces/julia surfaces/julia/test/runtests.jl
```

`Manifest.toml` is deliberately **not committed**: with no external
dependencies there is nothing to pin, and a Manifest would only record
one machine's stdlib build.
