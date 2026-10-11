---
title: codetopo
emoji: 🗺️
colorFrom: indigo
colorTo: gray
sdk: static
pinned: false
short_description: codetopo's map of its own codebase — a static snapshot.
---

# codetopo — static snapshot demo

**See your codebase the way an agent sees it.** codetopo turns a source
repository into a queryable code graph — symbols, calls, references, and
structure — with JSON answers an AI coding agent can act on directly.

This Space is a **static snapshot**, not a live server. codetopo was run
once against its own source code; the genuine API responses from that run
are bundled under `data/` (see `data/ABOUT.json` for the capture date, the
exact source commit, and the tool version), and this page renders them.
Search and browsing run in your browser against that bundled snapshot
only — nothing is computed on a server, and the snapshot does not change
until the next sync replaces it.

The **live** version of this demo — a container that indexes the bundled
source at startup and answers queries from the real API — is packaged in
the source repository under `space/` (Docker Space) and replaces this
static layout if the Space is ever switched to the Docker SDK.

## Mirror rule

This Space is a **mirror**. The source of truth is the GitHub repository
[nrupala/codetopo](https://github.com/nrupala/codetopo); everything here is
synced from there by automation (`.github/workflows/hf-sync.yml`) and is
overwritten on every sync. Changes made on this side will not survive —
send them to GitHub instead.

## License

- Engine (`codetopo-*` crates): **AGPL-3.0-or-later**
- Agent-facing contracts (`schemas/`): **Apache-2.0**

Owned by Nrupal Akolkar · Built with Muse by Meta.
