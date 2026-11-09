# codetopo for R

An R surface over the codetopo Rust core, built with
[extendr](https://extendr.rs/) (extendr-api + rextendr conventions).
Index a repository into a queryable code graph, then search, trace,
rank, and render it — deterministically and read-only.

Every answer is CODE_SCHEMA (v1.2) JSON produced by the same Rust
library the CLI, HTTP, and MCP surfaces serve, parsed with
`jsonlite::fromJSON(..., simplifyVector = FALSE)` so absent
measurements stay absent — never coerced to 0. Parity with the other
surfaces is by construction, and is pinned by the known-answer
conformance battery in `tests/testthat/` (oracle goldens, the quarry
package, the Aarona sibling fixture, lifecycle parity, determinism).

This package lives in the codetopo repository only. It is **not**
published to CRAN.

## Install

Requires R (>= 4.1) and a Rust toolchain (`cargo` on the PATH):

```sh
R CMD INSTALL surfaces/r
```

## Use

```r
library(codetopo)

topo <- codetopo_index("path/to/repo")          # or codetopo_open("path/to/index.db")
codetopo_stats(topo)
codetopo_search_symbols(topo, "handle")
codetopo_rank(topo, by = "pagerank", limit = 10)
codetopo_blast_radius(topo, "repo::a")
snap <- codetopo_snapshot(topo)
codetopo_verify(topo)
```

The handle is an environment of class `codetopo_handle` holding the
index database path (snapshot-born handles from
`codetopo_from_snapshot()` hold the snapshot JSON instead); the Rust
layer is stateless and loads the frozen graph from those coordinates
per call.

## Strictness

Every function captures `...` and errors **naming** any unknown
argument. Rank's metric argument is `by` **only** — the 0.9.1 lesson:
a call with `metric = "pagerank"` fails loudly naming `metric`,
instead of silently answering a default. Errors carry the core's
stable codes (`unknown_symbol`, `unsupported_schema_version`,
`invalid_param`, …) in the condition's `code` field.

## License

AGPL-3.0-or-later. The CODE_SCHEMA specification is Apache-2.0; this
implementation is a derivative of the AGPL core and carries its
license.
