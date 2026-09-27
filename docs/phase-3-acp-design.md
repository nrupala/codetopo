# Phase 3 ACP — Design (final, corrected)

Agent: `codetopo-acp` (crate `codetopo-acp`), read-only ACP adapter.

## SDK choice
- None. Direct wire-protocol implementation against the public ACP spec
  (newline-delimited JSON-RPC 2.0 over stdio; no Content-Length headers,
  stderr free-form logging, sealed stdout). No SDK, nothing vendored,
  nothing copied.
- `agent-client-protocol = "2.2.0"` removed from `Cargo.toml` (was never
  `use`d, only mentioned in comments). Module docs and this file state
  the direct implementation plainly.
- Implementation: direct stdio loop; protocol framing matches public spec.

## Capabilities advertised (initialize)
- `agent`: "codetopo"
- `protocolVersion`: "1.0"
- `agentCapabilities`: `read_only`, `code_structure`, `session_based`, `tools`
- `agentInfo`: `{name:"codetopo", version:"0.1.0"}`

## Session lifecycle
- `session/new`: accepts `cwd`. Indexes via `codetopo_cli::index_repo()` into temp SQLite DB (`/tmp/codetopo-<pid>-<session>.sqlite`). Same extraction path CLI uses (no duplicated extraction).
- `session/prompt`: loads graph via `codetopo_cli::load_graph_from_db()`, classifies with `codetopo_acp::classify_intent()`, executes via `codetopo_acp::execute_intent()` against `Graph` (with `db_path` for Snapshot/Verify).
- Snapshot (`Intent::Snapshot`): calls `codetopo_cli::snapshot_db(&db_path, &temp_out, None)`; reports temp path + file size honestly (or error).
- Verify (`Intent::Verify`): opens session store (`codetopo_store::Store::open`) and runs `audit.verify()`; reports PASS/FAIL honestly (or error).
- Streamed updates: `session/update` notifications before final prompt response.
- Edit requests (`rename`, `refactor`, `fix`): honest refusal from `RefusalReply::standard()`.
- Unknown node / empty index: honest error text (no fabricated data).

## Stdout seal
- Only JSON-RPC frames written to stdout (`writeln!` with `serde_json::Value`).
- All diagnostics → stderr (`eprintln!`).
- `extract_quoted` deleted (unused). `SessionInfo.cwd` removed (no `#[allow(dead_code)]` fig)
  — session response still returns `cwd`; state holds only `db_path`.
- `classify_intent` cleaned: `p.contains("hope") || p.contains("maybe")` clause deleted;
  Path parsing splits on `"path from"` / `" to "` to extract real node ids.
- SDK stays out of dependencies.

## Licensing
- AGPL-3.0-or-later header preserved in all new/modified source files.
- Attribution to Nrupal Akolkar retained.
- SDK stays out. Nothing vendored, nothing copied.
