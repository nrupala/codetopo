# Phase 2 Verification Report

Branch: `phase-2-api`
Workdir: `/tmp/opencode/codetopo`
Toolchain: cargo 1.98.0 aarch64 (PATH=/var/lib/oc-bridge/.rustup/toolchains/stable-aarch64-unknown-linux-gnu/bin, HOME=/var/lib/oc-bridge)

## Safety / Commit State
- `git status --short`: clean
- `git log --oneline -6` before fix: 3a085f6 (schemas), cd7601d (mcp), a8f669d (server), 5f8eeeb (design), 78e8b19 (main). Four files modified (`crates/codetopo-mcp/src/lib.rs`, `tests/mcp_stdio.rs`, `schemas/README.md`, `schemas/mcp-tools.json`).
- Committed as ONE commit: `6f98b80 Phase 2: codetopo_snapshot MCP tool (whole-graph export)` (8th MCP tool, maps to GET /v1/graphs/{id}/snapshot per docs/phase-2-design.md §9).

## 1. `cargo test --workspace`
Fully green. Per-crate results (all `ok`):
- `codetopo_cli`: 2 passed (tests/cli.rs)
- `codetopo_core`: 13 passed
- `codetopo_extract`: 20 passed
- `codetopo_mcp`: 31 passed + 4 passed (mcp_stdio.rs) + 0 doc-tests
- `codetopo_server`: 31 passed + 2 passed (main) + 19 passed (tests/api.rs)
- `codetopo_store`: 9 passed
- Others (`codetopo`, doc-tests): 0 passed / ok.
No failures, no ignored, no measured.

## 2. `cargo clippy --workspace --all-targets -- -D warnings`
Zero warnings. Output: `Finished dev profile` with no warning lines.

## 3. Server boot + round-trip
- Built: `cargo build -p codetopo-server` (binary `codetopo-server`).
- Data dir: `/tmp/ct2/data`; keys `/tmp/ct2/api_keys` (mode 600, `testkey1:s3cr3t`); metering `/tmp/ct2/metering.log`; bind `127.0.0.1:18080`.
- Server started via `nohup` + `env`; listening line confirmed (`codetopo-server: listening on http://127.0.0.1:18080 ...`).
- Fixture: `crates/codetopo-server/tests/fixture-repo`.
- HTTP index (`POST /v1/index` with `Bearer testkey1:s3cr3t`): `{"diagnostics":2,"edges":23,"files_failed":0,"files_indexed":3,"index_id":"df9862fb-8932-4f81-9898-70884a533b38","nodes":15}`.
- CLI index to `--db /tmp/ct2/cli.db`: `files failed: 0; symbols: 15; nodes: 15; edges: 23; diagnostics: 2`.
- CLI stats (`stats --db /tmp/ct2/cli.db`): nodes 15 (package 1, module 3, file 3, function 6, external 2); edges 23 (calls 5, references 6, contains 12).
- **Server index == CLI stats**: 15 nodes / 23 edges = pass.

### Descendants
- CLI node id chosen: `fixture-repo::alpha::entry` (from DB query: `fixture-repo::alpha::entry`). CLI `descendants --expand`: `fixture-repo::alpha::helper_a`, `fixture-repo::alpha::helper_b` (2 descendants).
- HTTP query: `GET /v1/graphs/df9862fb-.../descendants?node=unknown::alpha::entry&expand=true` (HTTP index uses package default `unknown`; CLI uses repo stem `fixture-repo`). Response nodes: `['unknown::alpha::helper_a', 'unknown::alpha::helper_b']` (total 2).
- Normalized comparison (strip `fixture-repo::` vs `unknown::`) produces identical sorted lists.
- Diff (normalized): empty = pass.

### Auth
- `curl -o /dev/null -w '%{http_code}'` with NO header on `/v1/graphs/<id>/stats`: `401`.
- Wrong key (`Bearer testkey1:wrongsecret`): `401`.
- Correct key (`Bearer testkey1:s3cr3t`): 200 (verified via index/post/stats/descendants).

### Metering
- `/tmp/ct2/metering.log`: 5 lines.
- Every line valid JSON; all contain `"key_id":"testkey1"`. No bad lines.

### Server stopped
- Process killed; `pgrep` confirms 0 remaining `target/debug/codetopo-server` processes.

## 4. MCP
- `cargo test -p codetopo-mcp`: 4 passed (mcp_stdio.rs: malformed frame, notification silence, stdout JSON-only, session answers recipe). 31 passed + 0 doc-tests (lib). All green.
- Manual stdio: piped `tools/list` and `tools/call codetopo_stats` into `./target/debug/codetopo-mcp` via Python `subprocess` with line-delimited JSON-RPC.
- Responses validated with `python3 -c json.load`: both load successfully (init result has `serverInfo.name=="codetopo"`; list result has 8 tools; stats call returns JSON result with `isError=true` due to missing param, which is valid JSON response).

## 5. Final git state
- `git log --oneline -8` (since main 78e8b19):
  - `6f98b80 Phase 2: codetopo_snapshot MCP tool (whole-graph export)`
  - `3a085f6 Phase 2: schemas (OpenAPI 3.1 + MCP tool catalogue)`
  - `cd7601d Phase 2: codetopo-mcp (stdio JSON-RPC server, sealed stdout)`
  - `a8f669d Phase 2: server (axum JSON API, bearer auth, metering)`
  - `5f8eeeb Phase 2: design doc for agent query API (HTTP + MCP)`
  - `78e8b19 Phase 1 workstream 5: integration fixes + exit-criteria verification`
  - `614c959 Phase 1 workstream 4: codetopo-cli (headless CLI)`
- `git status --short`: empty.

## Push
- `git push origin phase-2-api`: attempted (see below for result).
