<!-- Copyright (C) 2026 Nrupal Akolkar -->
<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
<!-- Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta. -->

# Governance

codetopo is a single-maintainer project. This document describes how it is
actually run, so contributors, users, and funders can see the structure
without guessing.

## Maintainer

- **Maintainer:** Nrupal Akolkar ([@nrupala](https://github.com/nrupala))
- The maintainer is the project's decision-maker for scope, merges,
  releases, and security response. There is no committee and no second
  maintainer today; that is a known limitation of a young project, stated
  plainly rather than dressed up.

## How decisions are made

- Proposals, bug reports, and design discussion happen in GitHub Issues
  and Pull Requests, in public.
- The maintainer decides, after considering input. Decisions and their
  reasoning are recorded in the issue or PR where they were made, and
  user-visible changes are recorded in `CHANGELOG.md`.
- Significant design work is written down before it is built (see
  `DESIGN.md` and the design documents under `docs/`).

## Contributions

- All changes land through Pull Requests, per `CONTRIBUTING.md`.
- A PR merges only when CI is green on its exact head: the workspace test
  suite, clippy with warnings denied, the Windows build of the agent
  binaries, and the container build.
- The maintainer reviews every PR. Contributors keep the copyright to
  their contributions; contributions are accepted under the project's
  license (AGPL-3.0-or-later).

## Releases

- Versions follow semantic versioning across the workspace crates.
- The maintainer cuts releases, tags them (`v*`), publishes the crates to
  crates.io, and writes the release notes. Only the latest published
  release receives security fixes (see `SECURITY.md`).

## Security

- The maintainer is also the security lead. Vulnerability reports follow
  `SECURITY.md`: GitHub private vulnerability reporting is the preferred
  route, reports are acknowledged within a few days, and fixes ship as a
  new patch release with a `CHANGELOG.md` note before the report is
  discussed publicly.

## Continuity and changes to this document

- The project's accounts (GitHub, crates.io, registry listings) are held
  by the maintainer. If regular contributors emerge, the maintainer will
  add co-maintainers and update this document to match — governance here
  describes reality, and it will change when reality does.
- Changes to this document are made by Pull Request and merged by the
  maintainer like any other change.
