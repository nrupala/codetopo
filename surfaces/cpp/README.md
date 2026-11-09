<!--
Copyright (C) 2026 2351485 ALBERTA INC.
SPDX-License-Identifier: AGPL-3.0-or-later
Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
-->

# codetopo — C and C++ face

The consumer face over codetopo's C ABI. Two artefacts, one
foundation:

1. **The shipped C header as a product** —
   `crates/codetopo-c/include/codetopo.h`, the stable C surface
   every language face stands on, proven usable by a consumer who
   has nothing else: `tests/consumer.c` includes only that header
   and links only the built `libcodetopo_c`, exactly as an
   external consumer would.
2. **A header-only C++17 wrapper** — `include/codetopo.hpp`:
   `namespace codetopo`, an RAII `Graph` (move-only; destructor
   calls `codetopo_graph_free`), a `CString` guard (frees via
   `codetopo_free_string`), free functions mirroring the ABI
   (`index`, `open_db`, `load_snapshot`, `version`,
   `schema_version`), `Graph::query(op, args_json)` for the
   twenty operations, and failures thrown as `codetopo::Error`
   carrying the parsed `code` + `message` from the ABI's error
   object.

## Bridge technology: the C ABI, not cxx

This face is built **over the C ABI, not the `cxx` crate**. The C
ABI is the house foundation — the substrate the R and Julia faces
also stand on. `cxx` would be a second, separately maintained
bridge with its own generated boundary; consuming the frozen C
ABI directly keeps one foundation and one conformance story.

## The boundary

CODE_SCHEMA JSON strings, in and out (CODE_SCHEMA §7). Every
answer is the same JSON the CLI, HTTP, MCP, and C surfaces
produce for the same question, returned as a `std::string`.
There are no C++ structs mirroring the graph schema.

**No third-party JSON dependency — anywhere in this face.** The
wrapper never interprets an answer; answers pass through as
strings for the caller to parse with their own JSON library. The
one object the wrapper itself reads is the small error object,
read with targeted field extraction (see `codetopo.hpp`). The
conformance program likewise reads its answers with small,
documented, test-only extraction helpers instead of a JSON
library, so the whole face builds with nothing beyond a C++17
compiler and the shipped header.

## Build and test

Build the foundation once (workspace root):

```sh
cargo build -p codetopo-c --release
```

Then compile and run the proofs (consumer smoke program, the
F1–F5 conformance battery, the minimal example):

```sh
surfaces/cpp/run-tests.sh
```

`run-tests.sh` locates `libcodetopo_c` via `CODETOPO_C_LIB`
(the directory holding the `.so`/`.dylib`); the default is the
workspace `target/release`. Everything compiles with
`-Wall -Wextra -Werror` — the C header standalone under C11, the
wrapper and its programs under C++17.

## Conformance

The battery is the same known-answer set every codetopo surface
must reproduce (CODE_SCHEMA v1.2 semantics):

- **F1** oracle goldens (`crates/codetopo-core/tests/fixtures/oracle/`):
  rank tolerances PageRank 1e-6, betweenness 1e-9, closeness
  1e-9, eigenvector 1e-6 after sign alignment, Katz 1e-6;
  topo-sort groups and cycle flags exact.
- **F2** the quarry package via `index`: 26 nodes / 33 edges /
  2 diagnostics over the package's Python files; the full
  fixture directory (whose README.md is itself indexed as
  Markdown) is asserted separately at 29 / 36 / 2.
- **F3** the Aarona sibling tripwire: imports 7/7, calls 7/7,
  `cyclic_groups: 2`; a rank call with a non-`by` metric key
  throws, naming the key.
- **F4** lifecycle index → snapshot → load → snapshot, compared
  by graph content (snapshots embed wall-clock audit timestamps,
  so byte-identity is not the contract); chains via `verify`.
- **F5** determinism: repeated queries byte-identical.

## Layout

```
surfaces/cpp/
  include/codetopo.hpp    the C++ wrapper (header-only)
  tests/consumer.c        C consumer smoke program (shipped header only)
  tests/conformance.cpp   the F1–F5 battery through the wrapper
  examples/minimal.cpp    smallest end-to-end C++ usage
  run-tests.sh            build + run all of the above
```
