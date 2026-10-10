# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Fixed

- **Blast radius walked the wrong way.** `blast_radius` returned what a
  node *depends on*; it now returns what *depends on it* — the
  dependents that break when it changes — as DESIGN §3 promises.
  CODE_SCHEMA §6 corrected to match, and the tests that enshrined the
  old direction rewritten, with a chain regression test.
- **`path` no longer fabricates directed paths.** When no directed
  path existed, the reverse walk was relabelled as a forward path.
  `path` now returns `None` honestly; the MCP test that expected a
  backwards chain is corrected, with a no-path-backwards assertion.
- **`codetopo verify` exits non-zero on a broken audit chain.** It
  printed "chain valid: false" and returned success; it now errors,
  with a tamper regression test.
- **CI added** (`.github/workflows/ci.yml`): `cargo test` +
  `cargo clippy -D warnings` on every PR and push to `main` — the
  repo's first automated gate.
- **Reindexing replaces; restore refuses a union (final-shape W1).**
  Reindexing an existing database now atomically *replaces* the stored
  graph (`Store::replace_graph`) instead of unioning new rows into the
  old index via `INSERT OR IGNORE` — a renamed or deleted symbol can no
  longer survive as a stale row. Restoring a snapshot into a populated
  database is refused with a usage error instead of silently merging
  two graphs. The `symbols:` line printed at index time now counts
  actual symbol nodes instead of repeating the total node count.

### Added

- **Persisted diagnostics (final-shape W1).** Builder diagnostics are
  stored with the index and replaced on reindex; the new
  `codetopo diagnostics --db` command prints them after the fact
  (previously only the count printed at index time existed). The store
  now stamps a format version (`PRAGMA user_version`), tracks an index
  generation per replacement, and refuses databases from newer formats.
  All seven crates bumped `0.2.1` → `0.2.2`.
- **Portfolio certification rollout.** `CONTRIBUTING.md` now documents
  the PR-flow discipline (draft PR → tests green → owner merges;
  every PR adds a CHANGELOG entry under Unreleased and bumps crate
  versions; merge commits reference PR numbers; releases tagged
  `vX.Y.Z`). All seven crates bumped `0.2.0` → `0.2.1`.

## [0.2.0] — 2026-10-05

First public release.

### Added

- **Phase 1 — engine + CLI.** `codetopo-core`: the codebase as a queryable graph
  (nodes, edges, traversal queries, snapshots). `codetopo-extract`:
  tree-sitter extractors for Rust and TypeScript/JavaScript.
  `codetopo-store`: SQLite persistence with a hash-chained, tamper-evident
  audit log. `codetopo` CLI: headless `index` plus `descendants`,
  `ancestors`, `path`, `snapshot` queries.
- **Phase 2 — API + MCP.** `codetopo-server`: axum HTTP API, 8 endpoints under
  `/v1`, Bearer-token auth, JSONL usage metering. `codetopo-mcp`: stdio MCP
  server exposing the engine as 8 tools. `schemas/`: OpenAPI 3.1
  (`openapi.yaml`) and MCP tool definitions (`mcp-tools.json`) under
  Apache-2.0.
- **Phase 3 — ACP agent.** `codetopo-acp`: stdio agent speaking the Agent
  Client Protocol — conversational sessions (`initialize` → `session/new` →
  `session/prompt`) with deterministic intent routing to the same core
  queries. Read-only; edit requests get an honest refusal.
