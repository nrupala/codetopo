// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

# Phase 3 — ACP Agent (`codetopo-acp`) Design

## Why ACP
Agents are codetopo's primary users. MCP (`codetopo-mcp`) lets an agent *call tools*; ACP lets any ACP-capable editor client (Zed, Neovim, JetBrains) *converse* with codetopo as a code-intelligence agent over stdio. ACP is session-oriented with `initialize` → `session/new` → prompt/response streams — exactly the conversational model editors expect.

## Protocol → codetopo mapping

| ACP message | codetopo action | Source crate |
|---|---|---|
| `initialize` | Advertise agent name="codetopo", capabilities (read-only, code-structure), protocol v1 | SDK `Agent` builder |
| `session/new` (cwd) | Index cwd into temp `codetopo-store` via `codetopo-extract` + `codetopo-store` | `codetopo-extract`, `codetopo-store` |
| `session/prompt` | Intent routing → query | `codetopo-core` |
| `session/update` | Progress notification (streamed) | SDK notification |
| `session/cancel` | Drop session, close store | `codetopo-store` |

## Intent routing (deterministic, keyword/regex)

| Intent (prompt keyword) | Query | Read-only |
|---|---|---|
| "descendants of X" / "what does X affect" | descendants | yes |
| "ancestors of X" / "what depends on X" | ancestors | yes |
| "blast radius of X" | blast-radius | yes |
| "path from A to B" | path | yes |
| "stats" / "overview" / "summarise" | stats | yes |
| "snapshot" | snapshot export to temp file + hash | yes |
| "verify" | audit-chain verification | yes |
| anything else / edit/write | honest refusal + capability list | yes |

No LLM, no network, no file writes outside temp snapshot. All answers derived from `codetopo-core` / `codetopo-store` — same queries the CLI, HTTP server, and MCP server use. Thin adapter; no duplicated graph logic.

## Read-only guarantee
Binary is READ-ONLY. No mutations to indexed source. Snapshot is written to `/tmp` only and reported with path + SHA-256. Edit requests get: "I am a read-only code-structure agent; I cannot edit files." plus capability list.

## Composition with MCP
- MCP (`codetopo-mcp`): agent calls codetopo as *tools* (structured, tool-use oriented).
- ACP (`codetopo-acp`): editor client *converses* with codetopo as an agent (session, prompt/stream, natural-language oriented).
- Both share `codetopo-core` / `codetopo-store`; neither duplicates graph logic.

## SDK pathway
Dependency `agent-client-protocol = "2.2.0"` (official Rust SDK, github.com/agentclientprotocol/rust-sdk, Apache-2.0/MIT). No SDK source vendored. Builder model: `Agent.builder()` with per-message handler closures; no trait impl required.
