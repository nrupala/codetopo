<!-- Copyright (C) 2026 Nrupal Akolkar -->
<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta. -->

# Security policy

## Reporting a vulnerability

Please report security issues privately, not in a public issue:

- Preferred: GitHub's private vulnerability reporting on this repository
  (**Security → Report a vulnerability**).
- Alternatively, contact the maintainer through the contact options on
  https://aimlds.org/codetopo.

You should receive an acknowledgement within a few days. Please include
the affected component (CLI, HTTP server, MCP server, ACP agent, or a
library crate), the version, and a minimal reproduction if you have one.

## Scope notes

codetopo is read-only by construction: it parses source code (never
executes it), exposes no mutation endpoints, and verifies snapshot
integrity (schema, audit chain, and proof certificate) before a restore
writes anything. Reports that matter most:

- Any path where indexing or querying executes, writes, or follows
  something outside the indexed tree or the chosen database.
- Authentication or metering bypasses in `codetopo-server`.
- Certificate or audit-chain verification that accepts a tampered
  snapshot or log.
- Panics or unbounded memory use on malformed source trees (the scale
  gate's territory — a crash on adversarial input is a defect here).

## Supported versions

Only the latest published release is supported with security fixes.
Fixes ship as a new patch release on crates.io and a note in the
CHANGELOG.
