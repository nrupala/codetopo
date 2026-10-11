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
Rewritten to the wire protocol, 0.2.7: the first implementation spoke
a home-grown dialect in the spec's clothing (string `"1.0"`, string
prompts, custom result fields) that no real client could drive. It
now speaks ACP v1:
- `protocolVersion`: `1` (an integer, as the spec requires)
- `agentCapabilities`: `{loadSession: false, promptCapabilities:
  {image: false, audio: false, embeddedContext: false},
  sessionCapabilities: {}}`
- `authMethods`: `[]`
- `agentInfo`: `{name: "codetopo", title: …, version: <crate
  version>}` — the version is `env!("CARGO_PKG_VERSION")`, never a
  hardcoded string.

## Session lifecycle
- `session/new`: accepts `cwd` (+ ignores `mcpServers`). The agent
  mints the session id; the result is exactly `{sessionId}`.
  Indexes via `codetopo_cli::index_repo()` into temp SQLite DB
  (`/tmp/codetopo-<pid>-<session>.sqlite`). Same extraction path
  CLI uses (no duplicated extraction).
- `session/prompt`: the prompt is a content-block array; text
  blocks are concatenated (`codetopo_acp::prompt_text`), classified
  with `classify_intent()`, executed via `execute_intent()` against
  `Graph` (with `db_path` for Snapshot/Verify). The answer streams
  as a `session/update` carrying an `agent_message_chunk`, and the
  turn ends with `{stopReason: "end_turn"}` — or `"refusal"` for
  edit requests, whose streamed text is `RefusalReply::standard()`.
- `session/cancel`: a notification, recorded and never answered
  (turns are synchronous, so a cancel can only land between turns;
  it ends a turn, never the session).
- Snapshot (`Intent::Snapshot`): calls `codetopo_cli::snapshot_db(&db_path, &temp_out, None)`; reports temp path + file size honestly (or error).
- Verify (`Intent::Verify`): opens session store (`codetopo_store::Store::open`) and runs `audit.verify()`; reports PASS/FAIL honestly (or error).
- Unknown node / empty index: honest error text (no fabricated data).
- Conformance: `crates/codetopo-acp/tests/stdio_conformance.rs`
  drives the shipped binary over stdio through the whole session;
  live frames from the binary validate against the official ACP v1
  JSON schema (`schema/v1/schema.json` of the protocol repo) —
  `docs/protocol-proof.md` records the evidence.

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
