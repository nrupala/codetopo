# Protocol proof — MCP and ACP conformance evidence

Recorded 2026-10-10 for the 0.2.7 protocol-proof workstream. The
claim each surface makes — "speaks MCP" / "speaks ACP" — is backed
here by wire-level evidence, not by the shape of the code.

## Method

1. **Strict stdio clients in the test suite.** Each protocol crate
   carries an integration test that spawns the *shipped binary*
   and speaks the protocol the way an official SDK client does:
   - `crates/codetopo-mcp/tests/stdio_strict.rs` — initialize
     negotiation, notification silence, `tools/list` shape,
     index→stats round trip, tool-error vs transport-error split.
   - `crates/codetopo-acp/tests/stdio_conformance.rs` — initialize,
     `session/new`, content-block prompts, streamed
     `agent_message_chunk` updates, `stopReason` endings (including
     `refusal`), `session/cancel` silence, error codes.
2. **Official-schema validation.** Live frames captured from the
   built binaries were validated with `jsonschema` against the
   protocols' own published schemas (below). This is the same
   contract editor clients (Zed for ACP) and SDK clients (MCP)
   implement; the frames were produced by the binaries, not
   hand-written for the check.

## Defects this phase found and fixed

- **MCP `tools/call` success results were not CallToolResults.**
  The server returned the tool's bare JSON object as the JSON-RPC
  result. The MCP schema requires a `content` array (`CallToolResult`
  requires `content`), and the official SDK result schemas parse
  exactly that — real clients saw no content while in-house tests,
  which read the bare object, stayed green. Success results now
  carry `content: [{type: "text", text: <result JSON>}]` plus
  `structuredContent` with the same object; the pre-existing stdio
  test was updated to read payloads from `structuredContent`.
- **ACP was a dialect, not the protocol.** `protocolVersion` was
  the string `"1.0"` (the spec's is the integer `1`); prompts were
  plain strings (the spec's are content-block arrays);
  `session/new` invented its result fields; answers came back in a
  custom `answer` field instead of a streamed `agent_message_chunk`
  + `stopReason`. All rewritten to ACP v1 (see
  `docs/phase-3-acp-design.md`); `session/cancel` is handled as the
  notification it is.

## Schema validation results (live binary frames)

ACP — against `schema/v1/schema.json` from
`agentclientprotocol/agent-client-protocol`:

- initialize result → `InitializeResponse` — **valid**
- `session/new` result → `NewSessionResponse` — **valid**
- `session/update` params → `SessionNotification` — **valid**
- `session/prompt` result → `PromptResponse` — **valid**
  (`{stopReason: "end_turn"}` and `{stopReason: "refusal"}`)

MCP — against `schema/2024-11-05/schema.json` from
`modelcontextprotocol/specification`:

- initialize result → `InitializeResult` — **valid**
- `tools/list` result → `ListToolsResult` — **valid**
- `tools/call` success result → `CallToolResult` — **valid**
- `tools/call` error result (`isError`) → `CallToolResult` — **valid**

## Honest limits

- Zed itself was not launched against the agent: this environment
  has no display/GPU for the editor. The ACP evidence is the
  protocol's own schema plus the wire-level client test — the same
  contract Zed implements — but it is not a screenshot of Zed.
- MCP validation used the 2024-11-05 schema, the protocol version
  the server negotiates. `structuredContent` postdates that schema
  version; it rides as a permitted extension property there and is
  standard in the 2025-03-26 revision.
