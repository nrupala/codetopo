# Quickstart Verification Report

Date: 2026-10-10
Base: `main` @ `1df98639850ebd7ebaf9bd27507d39458c930e94`
Toolchain: cargo 1.99.0 / rustc 1.99.0, stable x86_64-unknown-linux-gnu,
gcc 13.3.0. Fresh copy of `main` (tarball), no prior `target/`.

Closes the standing question behind issue #6: does the README's
60-second quickstart work exactly as written, and do the printed
fixture numbers match the binary? Short answer: the fixture numbers
are exact; three quickstart drifts were found and fixed in the README.

## 1. Baseline

- `cargo build --workspace` — exit 0.
- `cargo test --workspace` — exit 0. 140 passed, 0 failed
  (core 13, extract 20, store 9, cli 2, server 31 + 2 + 19,
  mcp 31 + 4, acp 9, plus doc-tests).
- Real CLI surface (`codetopo --help` and per-subcommand `--help`):
  `index`, `descendants`, `ancestors`, `blast-radius`, `path`,
  `stats`, `verify`, `snapshot`, `restore`. Every command takes a
  required `--db`. `codetopo --version` → `codetopo 0.2.1`.

## 2. The quickstart, run exactly as documented (before fixes)

### Install — FAIL

```sh
$ cargo install --git https://github.com/nrupala/codetopo
    Updating git repository `https://github.com/nrupala/codetopo`
error: multiple packages with binaries found: codetopo-acp, codetopo-cli, codetopo-mcp, codetopo-server. When installing a git repository, cargo will always search the entire repo for any Cargo.toml.
Please specify a package, e.g. `cargo install --git https://github.com/nrupala/codetopo codetopo-acp`.
(exit 101)
```

Cargo fetched the repository fine; the failure is cargo's
multi-package rule. Corrected form, verified end-to-end:

```sh
$ cargo install --git https://github.com/nrupala/codetopo codetopo-cli codetopo-server codetopo-mcp codetopo-acp
     Summary Successfully installed codetopo-cli, codetopo-server, codetopo-mcp, codetopo-acp!
(exit 0; all four binaries present; `codetopo --version` → 0.2.1)
```

### Index + query — the example id does not resolve

Indexing a project at `~/my-project` (a two-file Rust crate with
`net::Client::connect`) produces the package prefix `my-project` —
the directory's file stem, hyphen included. The README's example id
used `myproject::…`:

```sh
$ codetopo index ~/my-project --db graph.db
files indexed: 2
files failed: 0
symbols: 8
nodes: 8
edges: 8
diagnostics: 1

$ codetopo descendants myproject::net::Client::connect --db graph.db --expand
codetopo: error: unknown id 'myproject::net::Client::connect': not present in the graph
(exit 2)

$ codetopo descendants my-project::net::Client::connect --db graph.db --expand
0 descendants of my-project::net::Client::connect
(exit 0)
```

## 3. Example queries against the fixture — numbers exact

Run from the repository root with the built CLI, and re-run with the
release binary produced by the corrected install command. Output was
identical both times:

```sh
$ codetopo index crates/codetopo-server/tests/fixture-repo --db graph.db
files indexed: 3
files failed: 0
symbols: 15
nodes: 15
edges: 23
diagnostics: 2

$ codetopo stats --db graph.db
nodes: 15
  package: 1
  module: 3
  file: 3
  function: 6
  external: 2
edges: 23
  calls: 5
  references: 6
  contains: 12
```

Every line matches the README, including the per-kind breakdown.
**15 nodes / 23 edges confirmed.**

One formatting drift in the third example:

```sh
$ codetopo descendants fixture-repo::alpha::entry --db graph.db --expand
2 descendants of fixture-repo::alpha::entry
fixture-repo::alpha::helper_a
fixture-repo::alpha::helper_b
```

The README printed `descendants of fixture-repo::alpha::entry 2`;
the binary prints the count first. The id list itself was correct.

The remaining documented commands were each run against the fixture
database and behaved as the README describes: `ancestors`
(`1 ancestors of fixture-repo::alpha::helper_a` → `…::entry`),
`blast-radius` (2 nodes), `path` (renders
`fixture-repo::alpha::entry --calls--> fixture-repo::alpha::helper_a`),
`verify` (`chain valid: true`), `snapshot` (writes the JSON export).
`restore` also works (`restored nodes: 15 / restored edges: 23`,
stats identical afterwards) but was not documented anywhere — it is
now listed in the README with the other queries.

## 4. Spot checks on the other surfaces

- **HTTP API**: server booted with the README's `api_keys` flow;
  `POST /v1/index` on the fixture returned
  `{"diagnostics":2,"edges":23,"files_failed":0,"files_indexed":3,"index_id":"…","nodes":15}`;
  unauthenticated requests return 401; `descendants` and `stats`
  responses match the documented shapes; defaults
  (`127.0.0.1:8080`, `./api_keys`, the four `CODETOPO_*` env knobs)
  match `codetopo-server`'s config.
- **MCP**: `tools/list` returns exactly the 8 documented tools, with
  the documented names.
- **ACP**: the `initialize` response contains every field the
  README's example shows (`agent`, `protocolVersion` `1.0`,
  `read_only`/`code_structure` capabilities, agent name); the real
  frame carries additional fields the example abbreviates away.

## 5. What changed

README.md only (plus this note, a CHANGELOG entry, and the patch
version bump): the install command names the four packages, the
quickstart example id is `my-project::net::Client::connect`, the
`descendants` example header is count-first, and `restore` is listed.
No engine code was touched. `docs/phase-2-verification.md` and
`docs/phase-3-acp-verification.md` were re-checked: their fixture
numbers (15 / 23) agree with the binary and stand as written.
