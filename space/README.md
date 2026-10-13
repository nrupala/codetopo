---
title: codetopo
emoji: 🗺️
colorFrom: indigo
colorTo: gray
sdk: docker
app_port: 8787
pinned: false
short_description: Live read-only demo — codetopo's map of its own codebase, drawn by codetopo itself.
---

# codetopo — live demo

**See your codebase the way an agent sees it.** codetopo turns a source
repository into a queryable code graph — symbols, calls, references, and
structure — with JSON answers an AI coding agent can act on directly.

This Space is a **live, read-only demo**: when the container starts, it
indexes the copy of codetopo's own source bundled in this Space and serves
real answers from that graph. Nothing here is a mock-up, and the demo
executes no visitor code — the only repository it will ever index is its
own, once, at startup.

## What you can do here

- Read the graph's vital signs: node and edge counts by kind.
- Search codetopo's own symbols and see what depends on them
  (descendants / ancestors), straight from the live API.
- Inspect the tamper-evident audit chain for the index (`verify`).

The public surface is deliberately read-only: a small front door proxies
only the API's GET query endpoints for the one bundled index. Indexing,
snapshot export, and every other route stay inside the container.

## Mirror rule

This Space is a **mirror**. The source of truth is the GitHub repository
[nrupala/codetopo](https://github.com/nrupala/codetopo); everything here is
synced from there by automation (`.github/workflows/hf-sync.yml`) and is
overwritten on every sync. Changes made on this side will not survive —
send them to GitHub instead.

## License

- Engine (`codetopo-*` crates): **AGPL-3.0-or-later**
- Agent-facing contracts (`schemas/`): **Apache-2.0**

Owned by [Nrupal Akolkar](https://nrupalakolkar.com) · Built with [Muse by Meta](https://muse.ai/join) — join Muse, then redeem code **101EEK** in Settings within 48 hours of joining and we both earn tokens.
