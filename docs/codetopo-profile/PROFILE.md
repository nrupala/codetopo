# Codetopo Profile — Codetopo itself (nrupala/codetopo)

**What this is.** Codetopo profiling its own repository: the agent-facing
map generated with Codetopo 0.9.1 (local engine) — topology, trust
boundaries, money surface — with the graph snapshot beside it
(`snapshot.json`, `stats.json`) as the measured evidence.

- **Repo:** nrupala/codetopo (public)
- **Commit profiled:** `00bbf3766e6a8244265f41fb48981e9cc3237ec1` (main)
- **Engine:** codetopo 0.9.1 + full-language program (engine built from codetopo main `d71509d1` — all sixteen program extractors merged) · index run 2026-11-19
- **Graph:** 282 files walked · 0 failed · 11,524 nodes · 46,422 edges · audit chain valid (3 entries, head `d6011465…eda6`)
- **Nodes by kind:** external 7,420 · function 1,896 · method 809 · variable 402 · class 364 · file 279 · module 262 · doc 58 · interface 23 · package 11
- **Edges by kind:** calls 21,283 · reads 15,694 · defines 4,394 · contains 4,096 · imports 738 · writes 209 · implements 6 · inherits 2

## Coverage honesty

**Re-profiled 2026-11-19** with the full-language engine (Codetopo extractor program, 2026-11-18 ruling): every language this repo is written in is now extracted at symbol level. First profile: 2026-11-18 (Codetopo 0.9.1).

Extracted at symbol level: Rust (the engine itself), TypeScript/JavaScript and Python (client tooling and tests), Markdown as Doc nodes — and, since the full-language extractor program landed, the diaspora client bindings' own languages, named as not extracted in the first profile: the C and C++ client sources, the R package sources, and the Julia package sources are now in the graph (C #155, C++ #159, R #162, Julia #163), as is every extractor the program added (this repo's own tree now contains all sixteen program extractors). XLSX workbooks (7) are inputs to the RA workbook extractor, not code. Symbol coverage of the codebase is complete.
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
