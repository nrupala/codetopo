# Codetopo Profile — Codetopo itself (nrupala/codetopo)

**What this is.** Codetopo profiling its own repository: the agent-facing
map generated with Codetopo 0.9.1 (local engine) — topology, trust
boundaries, money surface — with the graph snapshot beside it
(`snapshot.json`, `stats.json`) as the measured evidence.

- **Repo:** nrupala/codetopo (public)
- **Commit profiled:** `7bc0d03e5ce160dd7c9dd63295379b744789eefb` (main)
- **Engine:** codetopo 0.9.1 · index run 2026-11-18
- **Graph:** 148 files walked · 8,756 nodes · 29,239 edges · audit chain valid
- **Nodes by kind:** function 1,259 · method 507 · class 245 · interface 12 · variable 212 · module 136 · file 148 · doc 40 · external 6,186 · package 11
- **Edges by kind:** calls 13,832 · reads 10,240 · contains 2,559 · defines 2,024 · imports 521 · writes 57 · implements 4 · inherits 2

## Coverage honesty

Extracted at symbol level: Rust (97 `.rs` — the engine itself),
TypeScript/JavaScript and Python (10 `.py` — client tooling and tests),
Markdown as Doc nodes (43). **Not extracted:** the diaspora client
bindings' own languages — R (5 files), C (3), plus C++/Julia package
sources: those are ways for other languages to *call* Codetopo, not
code Codetopo's extractors read (extractors for them are Stage-3
program items, filed in the program SEQUENCE). XLSX workbooks (7) are
inputs to the RA workbook extractor, not code.

## Topology

A deterministic structural-graph engine for code: tree-sitter
extraction → typed graph (CODE_SCHEMA) → queries, exposed three ways.
Ten crates under `crates/`:

- `codetopo-core` — the graph model, CODE_SCHEMA, rank/metrics,
  snapshot + audit chain (hash-chained index log).
- `codetopo-extract` — the tree-sitter walkers (Rust, TypeScript/
  JavaScript, Python) + Markdown docs + the RA/XLSX workbook extractor.
- `codetopo-store` — SQLite persistence for graphs.
- `codetopo-cli` — the `codetopo` binary (index/stats/verify/snapshot/
  symbol/search/blast-radius/render/…).
- `codetopo-mcp` — the `codetopo-mcp` MCP server (20 tools mirroring the
  HTTP surface).
- `codetopo-server` — the hosted HTTP API (20 endpoints; key auth,
  per-key metering, quota tiers).
- `codetopo-bridge` — the bridge service fronting the hosted API.
- `codetopo-acp` — the Agent Client Protocol surface (read-only agent).
- `codetopo-c` / `codetopo-napi` — C ABI and Node bindings.

## Trust boundaries

- **Hosted API keys are the boundary:** keys are created, claimed once
  on a single-use claim page, and metered per accepted request;
  indexes are deleted on expiry or request. Key values never appear in
  this repo or this profile.
- **Determinism is the integrity claim:** same tree in → same graph
  out; the audit chain (hash-linked) makes index history tamper-evident
  (`verify` in this pack: chain valid).
- **The engine runs no model:** extraction and queries are pure graph
  mathematics; whatever model consumes the graph does its own thinking.
  Untrusted input is repository content — walkers parse, they never
  execute indexed code.
- **Blast radius:** `codetopo-extract` and `codetopo-core` are the hubs
  (13,832 call edges) — a schema or walker change alters every profile
  Codetopo produces, including this one.

## Money surface

The engine binary itself moves no money. The **hosted service** sells
subscriptions (Team / Guardrails tiers) through the codetopo.com site
surface (Stripe checkout + Customer Portal, keys issued after payment)
— that billing code lives in the site worker, not in this repo. Inside
this repo the money-adjacent surface is the server's per-key metering
and quota enforcement (`codetopo-server`): accepted requests are
counted against the key's tier. No payment credentials are held here.

## How an agent should approach

1. Read `README.md` + `SKILL.md` (agent onboarding), then this profile.
2. Query the graph before editing a walker or the schema; run the
   extract known-answer fixtures after any extractor change.
3. Never build from the stale trees `codetopo-work/src` or `main-src`
   (0.2.x, pre-Python) — this repo's main is the only source.
4. Regenerate this profile at every release.

## Regeneration

`docs/codetopo-profile/refresh.sh` (in this pack) re-fetches the repo
at a named ref, re-runs the index, and rewrites `snapshot.json`,
`stats.json` and `verify.txt`.

## Standing-condition lines for this change

- Documentation at par: this pack is the documentation change, same PR;
  no behaviour changed.
- Release gate: NOT RUN for this profile PR · Guardian Mesh approval:
  NOT RUN. Merging is not "shipped"; the pack enters the gate queue in
  the Stage-1 evidence pack.
- OpenSSF Best Practices: project 15421 — **Passing** achieved
  2026-10-12 (tiered 115%), checked 2026-11-18. This additive docs
  change does not degrade any criterion.
