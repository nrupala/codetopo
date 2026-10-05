# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Added

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
