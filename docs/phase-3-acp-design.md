# Phase 3 ACP — Design (final, corrected)

Agent: `codetopo-acp` (crate `codetopo-acp`), read-only ACP adapter.

## SDK choice
- Pinned: `agent-client-protocol = "2.2.0"` (Cargo.toml, unchanged).
- 2.2.0's `Agent` builder (`Agent::builder()` + per-message closures) requires async-runtime integration for stdio server mode that exceeds this thin adapter's scope.
- Implementation: direct JSON-RPC 2.0 stdio loop against `agent-client-protocol` protocol types (v1). SDK dependency remains; never vendored.
- Divergence documented here; docs match code.

## Capabilities advertised (initialize)
- `agent`: "codetopo"
- `read_only`: true; `code_structure`: true; `session_based`: true
- Protocol version v1.

## Session lifecycle
- `session/new`: accepts `cwd`. Indexes via `codetopo_cli::index_repo()` into temp SQLite DB (`/tmp/codetopo-<pid>-<session>.sqlite`). Same extraction path CLI uses (no duplicated extraction).
- `session/prompt`: loads graph via `codetopo_cli::load_graph_from_db()`, classifies with `codetopo_acp::classify_intent()`, executes via `codetopo_acp::execute_intent()` against `Graph`.
- Streamed updates: `session/update` notifications before final prompt response.
- Edit requests (`rename`, `refactor`, `fix`): honest refusal from `RefusalReply::standard()`.
- Unknown node / empty index: honest error text (no fabricated data).

## Stdout seal
- Only JSON-RPC frames written to stdout (`writeln!` with `serde_json::Value`).
- All diagnostics → stderr (`eprintln!`).
- `codetopo-cli` index reports changed from `println!` to `eprintln!` so library use doesn't break seal.

## Licensing
- AGPL-3.0-or-later header preserved in all new/modified source files.
- Attribution to Nrupal Akolkar retained.
- SDK stays dependency.
