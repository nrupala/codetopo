<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# codetopo — scaffolding record (delivery-system artifact)

**Filed:** 2026-11-10, at the v0.9.1 arming. **Read this first:**
codetopo was built before the delivery system's Stage-2 scaffolding
certification existed as a step, and **no scaffolding certification
was issued during its build**. This record does not backfill one.
What it does is state, truthfully and in one place, the scaffolding
the product actually stands on, so the gate can see the real shape
instead of an absence.

## The scaffolding that exists

- **CI as the standing guard:** `.github/workflows/ci.yml` runs the
  full workspace suite (`cargo test --workspace`) plus the project's
  lint/type legs on every pull request; merges happen only on green.
  Per-surface workflows (C ABI, C/C++, JavaScript, Julia, R) guard
  the diaspora surfaces the same way, and `release-binaries.yml`
  builds the release artifacts. CodeQL and OpenSSF Scorecard run as
  standing analysis.
- **The gate program as the release guard:** this release's Gate 17
  pack (scanner pass, independent build and test, live attacker-view
  and money-surface probes, CodeQL #74 re-audit) is filed at
  `~/workspace/release-gate-runs/codetopo-v091-2026-11-10/`, with the
  gate-runner's armed re-run appended to the same certification.
- **Deterministic fixtures as the behaviour contract:** the engine's
  known-answer fixtures (the quarry tree: 29 nodes / 36 edges /
  2 diagnostics) are re-proven by every surface's battery, so a
  surface cannot drift from the engine quietly.
- **Schema as the interchange contract:** CODE_SCHEMA (in-repo
  `CODE_SCHEMA.md`, `schemas/`) versions the graph format;
  stored-format changes are changelog events, never silent ones
  (the 0.9.1 gate's F-1 condition exists precisely because of this).

## The honest gap

The delivery system asks for a scaffolding certification produced
*before* feature work. For codetopo that artifact was never produced
and cannot be produced retroactively; the scaffolding above grew
with the product instead. Whether this record plus the gate evidence
satisfies the scaffolding obligation for the v0.9.1 RC is the gate
lane lead's call, recorded in the certification's addendum — this
file asserts nothing about it.
