# Changelog

All notable changes to this project are documented here.

## [Unreleased]

### Added

- **Contract drift gate (final-shape, contracts).**
  `scripts/check-contracts.py`, run in CI after the test/clippy
  steps, compares the published contracts against the running code
  and fails the build on any disagreement: the MCP tool catalogue
  (`schemas/mcp-tools.json`) against the built `codetopo-mcp`
  binary's live `tools/list` answer, the OpenAPI paths against the
  routes the server actually registers, and the ACP agent's
  advertised identity against its crate. The schemas README's own
  counts and tables are corrected to match (ten graph reads plus
  `/v1/schema`; twelve MCP tools and their HTTP twins).

- **One surface, three doors (final-shape W3).** The CLI, HTTP API,
  and MCP server now answer the same questions with the same shapes:
  - *Discovery everywhere.* `Graph::search_symbols` (exact < prefix <
    contains ranking) plus `codetopo search`, `GET …/search`, and
    `codetopo_search_symbols` — before this, every query needed an
    exact node id nobody could discover from the tools themselves.
  - *Neighborhood everywhere.* CODE_SCHEMA §6 names the neighborhood
    query binding, but no surface exposed it. Now: `codetopo
    neighborhood`, `GET …/neighborhood`, `codetopo_neighborhood`,
    all serving the core `Graph::neighborhood` computation.
  - *One symbol, in full:* `codetopo symbol`, `GET …/symbol`,
    `codetopo_get_symbol`; and the schema itself: `codetopo schema`,
    `GET /v1/schema`, `codetopo_describe_schema` — one generator in
    the CLI library, so the three cannot drift.
  - *One package identity.* Indexing without an explicit package now
    derives the same name on every surface (the repository
    directory's file stem). The HTTP surface previously recorded
    `unknown`, so the same tree produced different node ids — and
    different query results — depending on the door used.
  - *CLI `--json`* on descendants / ancestors / blast-radius / path /
    stats, emitting the HTTP response shapes (minus `index_id`), so
    scripts get one format from either door.
  - `schemas/mcp-tools.json` regenerated from the live binary
    (twelve tools); `schemas/openapi.yaml` documents the new routes.
  (No version bump in this PR.)
- **Extractor conformance (final-shape W4).** The graph now carries
  the schema's full edge vocabulary:
  - *Reads/writes are first-class.* A resolved ref is a `reads` edge
    and an assignment target is a `writes` edge — before this, reads
    hid inside `references` edges with a `kind: "read"` attribute,
    invisible to kind-walking queries (blast radius included).
    CODE_SCHEMA §4 amended accordingly (dated amendment note);
    `references` remains restorable from older snapshots.
  - *Heritage edges.* Rust `impl Trait for Type` → `implements`,
    trait supertraits → `inherits`; TypeScript `extends` /
    `implements` clauses likewise. New emit channel
    (`FileSymbols.heritage`) resolved by the builder.
  - *Tiers classify.* By node kind in the builder — skeleton
    (package/module/file) `core`, type declarations `recommended`,
    the rest `optional` — instead of every node carrying the default.
  - *More of the tree is seen:* `.mjs` / `.cjs` files, Markdown
    files as `doc` nodes, Rust type aliases and TS enums / type
    aliases as type symbols.
  - Verified on this repo's own code: 1,932 reads edges, writes,
    19 doc nodes, tiers distributed — and zero `implements` edges,
    correctly: every impl in this codebase targets an external trait.
  (No version bump in this PR — W2 owns the next one.)
- **Proof certificates v2 + standalone verification (final-shape W2).**
  Snapshot certificates now carry a serial number, issuer, signer-key
  fingerprint, and a signed one-year validity window; an expired
  certificate is refused. Restore and the new
  `codetopo verify-proof --snapshot` command share one verification
  gate (schema major per CODE_SCHEMA §8, audit chain, certificate) —
  restore writes only after the gate passes, and verify-proof runs
  the same gate without writing. Certificate v1 files still verify
  and are reported as v1 (no serial or validity window). All seven
  crates bumped `0.2.2` → `0.2.3`.
- **Persisted diagnostics + store format gate (restored from PR #13,
  see the reconciliation note below).** Builder diagnostics are
  stored with the index and replaced on reindex; the new
  `codetopo diagnostics --db` command prints them after the fact.
  The store stamps a format version (`PRAGMA user_version`) and
  refuses databases from newer formats. The `symbols:` line at index
  time now counts actual symbol nodes. All seven crates are at
  **0.2.2**.

### Fixed

- **Reconciliation: PR #13's W1 changes are back.** Two store-
  integrity implementations were built in parallel today (PR #12 in
  the demo thread, PR #13 in the main thread). PR #12's merge used
  its branch tree wholesale and silently dropped PR #13's tree
  changes (persisted diagnostics, the format gate, the honest
  symbols count, the 0.2.2 bump) even though both merges are in
  history. This entry's PR restores them, merged with PR #12's
  verified restore: where the two disagreed on restore semantics,
  **verify-then-replace** (PR #12) stands — a snapshot is restored
  only after its schema, audit chain, and proof certificate verify,
  and restore then replaces by design; PR #13's refuse-populated
  variant was not kept, because refusal alone verifies nothing.
  Lesson recorded: a hand-built merge must merge trees, not adopt
  one side's tree, when the base has moved.
- **The store now replaces; it never silently unions.** Reindexing or
  restoring into an existing database used `INSERT OR IGNORE`, so
  stale symbols from an older graph survived forever. Indexing and
  restore now go through `Store::replace_graph` — one transaction
  that swaps the whole graph and advances a **graph generation**
  counter; the store also carries a `meta` table with a schema
  version (v1) from here on.
- **Snapshot proofs are certificates, and restore verifies.**
  `<snapshot>.proof` is now a structured certificate (format id,
  algorithm, signer, the snapshot's SHA-256, audit head, and counts,
  HMAC-signed) instead of a bare MAC. `codetopo restore` verifies
  before writing: schema version, the snapshot's audit chain against
  its declared head (always), and the certificate whenever one
  exists (digest/head/counts/signature). Restoring without a
  certificate requires the explicit `--allow-unverified` flag and is
  loud about it. Regression tests cover reindex replacement and
  every refusal path.
- **Blast radius walked the wrong way.** `blast_radius` returned what a
  node *depends on*; it now returns what *depends on it* — the
  dependents that break when it changes — as DESIGN §3 promises.
  CODE_SCHEMA §6 corrected to match, and the tests that enshrined the
  old direction rewritten, with a chain regression test.
- **`path` no longer fabricates directed paths.** When no directed
  path existed, the reverse walk was relabelled as a forward path.
  `path` now returns `None` honestly; the MCP test that expected a
  backwards chain is corrected, with a no-path-backwards assertion.
- **`codetopo verify` exits non-zero on a broken audit chain.** It
  printed "chain valid: false" and returned success; it now errors,
  with a tamper regression test.
- **CI added** (`.github/workflows/ci.yml`): `cargo test` +
  `cargo clippy -D warnings` on every PR and push to `main` — the
  repo's first automated gate.

### Added

- **Portfolio certification rollout.** `CONTRIBUTING.md` now documents
  the PR-flow discipline (draft PR → tests green → owner merges;
  every PR adds a CHANGELOG entry under Unreleased and bumps crate
  versions; merge commits reference PR numbers; releases tagged
  `vX.Y.Z`). All seven crates bumped `0.2.0` → `0.2.1`.

### Fixed

- **The ACP agent now advertises its real identity.** Its
  `initialize` response claimed version "0.1.0" — frozen since the
  first build while the crates moved to 0.2.x — and a string
  protocol version "1.0". It now reports the crate version
  (`env!("CARGO_PKG_VERSION")`) and `protocolVersion` 1, the public
  ACP spec's integer. The phase-3 design doc carries a dated
  amendment saying plainly which parts of the wire follow the spec
  and which are a codetopo dialect; session indexing also uses the
  shared package-name default like every other surface.
  (No version bump in this PR.)

## [0.2.0] — 2026-10-05

First public release.

### Added

- **Phase 1 — engine + CLI.** `codetopo-core`: the codebase as a queryable graph
  (nodes, edges, traversal queries, snapshots). `codetopo-extract`:
  tree-sitter extractors for Rust and TypeScript/JavaScript.
  `codetopo-store`: SQLite persistence with a hash-chained, tamper-evident
  audit log. `codetopo` CLI: headless `index` plus `descendants`,
  `ancestors`, `path`, `snapshot` queries.
- **Phase 2 — API + MCP.** `codetopo-server`: axum HTTP API, 8 endpoints under
  `/v1`, Bearer-token auth, JSONL usage metering. `codetopo-mcp`: stdio MCP
  server exposing the engine as 8 tools. `schemas/`: OpenAPI 3.1
  (`openapi.yaml`) and MCP tool definitions (`mcp-tools.json`) under
  Apache-2.0.
- **Phase 3 — ACP agent.** `codetopo-acp`: stdio agent speaking the Agent
  Client Protocol — conversational sessions (`initialize` → `session/new` →
  `session/prompt`) with deterministic intent routing to the same core
  queries. Read-only; edit requests get an honest refusal.
