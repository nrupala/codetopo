# Contributing to codetopo

## Ground rules

- **Read-only by design.** codetopo models code structure; it never edits source, scores code, or assigns meaning. Keep it that way.
- **Thin adapters.** The CLI, HTTP server, MCP server, and ACP agent are all thin adapters over `codetopo-core` / `codetopo-store`. Graph logic lives in core — never duplicate it in an adapter.
- **Evidence over assertion.** New behavior ships with tests and a verification note. If you claim a number, show the command that produced it.
- **Deterministic.** No network calls, no LLMs, no randomness in the engine path.

## Build and test

Requires a stable Rust toolchain ([rustup](https://rustup.rs)) and a C compiler (tree-sitter grammars).

```sh
cargo build --workspace
cargo test --workspace          # must be fully green
cargo clippy --workspace --all-targets -- -D warnings   # zero warnings
```

## Pull requests

- Branch from `main`, one concern per PR.
- CI shape: build → clippy (warnings denied) → test. No regressions.
- Significant work ships with a short design doc under `docs/` (why, what, how verified).
- The owner merges. Never push directly to `main`.

## Licensing

Every new source file carries the AGPL header plus attribution:

```
// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.
```

(Adjust comment syntax per file type.) `schemas/` stays Apache-2.0. Never copy
third-party code into the repo — implement against public specs and use
official dependencies instead.

## Portfolio certification — PR-flow discipline

- All changes land via **draft PR** — never push directly to `main`.
- PR flow: draft PR → `cargo build` + clippy (zero warnings) + `cargo test`
  fully green → the owner merges.
- Every PR adds a `CHANGELOG.md` entry under `## [Unreleased]` and bumps
  semver: patch for fixes/chores, minor for features. Bumps touch
  `crates/*/Cargo.toml` and the matching `Cargo.lock` entries.
- Merge commits reference the PR number. Releases are tagged `vX.Y.Z`.
- Docs-only PRs don't need test runs, but every build/test command in the
  docs must be verified against the repo (or marked unverified).
