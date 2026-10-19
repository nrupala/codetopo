## [Unreleased]

- MCPB bundle: the manifest now carries each tool's `title` and `annotations` from the catalogue, an `icon` (the node-graph glyph, `assets/icon-512.png`, shipped in the bundle as `icon.png`), and a `privacy_policies` entry pointing at the README's Privacy Policy section — the Claude directory's requirements for local connectors.

- MCPB bundle launcher: extract release archives with `tar --no-same-owner`. The archives carry the build runner's uid/gid, and tar running as root (e.g. in a container) failed the extraction trying to restore that ownership, even though the binary extracted fine. Also, the packager's default release tag now derives from the crate version instead of a pinned literal.

- Added glama.json for the Glama directory listing.
- Added a Gemini CLI extension manifest (`gemini-extension.json`) and context file (`GEMINI.md`) at the repository root. With the `gemini-cli-extension` GitHub topic set, the repository is indexed by the Gemini CLI extensions gallery, and Gemini CLI users can enable codetopo as an extension. The extension launches the `codetopo-mcp` binary over stdio; installing it first (`cargo install codetopo-mcp`) is the one prerequisite, stated in the manifest description and in `GEMINI.md`.

- Added the OpenSSF Scorecard workflow (`.github/workflows/scorecard.yml`, `publish_results: true`) and the Scorecard badge in the README, so the repository's security posture is scanned on every push to `main` and weekly, with results published to api.scorecard.dev and the code-scanning dashboard.

- Added a CodeQL workflow (`.github/workflows/codeql.yml`): vulnerability-focused static analysis of the Rust workspace on every push to `main`, every pull request, and weekly, with findings in the code-scanning dashboard — alongside the existing clippy quality gate.

- Added the missing root `LICENSE` file (canonical AGPL-3.0 text). The crates have always declared AGPL-3.0-or-later, but the license file itself was absent from the repository root — found by the docs-standard sweep's inventory scan; the ACP Registry submission's license_url pointed at it.

- **codetopo-mcp 0.3.4 — Windows build fixed:** the stdio stdout seal used Unix-only fd APIs (`std::os::fd`, `libc::dup2`), so the crate never compiled on Windows. The seal now has a Win32 implementation (duplicate the stdout handle for the protocol writer; `SetStdHandle` to `NUL` before std's lazy first handle fetch), and CI gains a Windows check job for the two shipped agent binaries so this cannot regress silently. Found by the release-binaries pipeline — the first Windows build in the project's history.

- Release binaries now also cover `codetopo-mcp`, and `scripts/package_smithery_mcpb.py` packages it as a Smithery MCPB bundle: five vendored binaries exceed Smithery's 25 MB limit, so the bundle's zero-dependency Node launcher downloads the pinned platform archive from the GitHub release on first run and refuses to run it unless its SHA-256 matches the pin baked in at pack time (verified then against the release's published sidecar). Tool schemas embedded from `schemas/mcp-tools.json`.


- Added `SECURITY.md`: private vulnerability reporting (GitHub PVR) and the security scope notes for the read-only surfaces.

# Changelog

All notable changes to this project are documented here.

## [Unreleased]

- **codetopo-bridge — the key now also arrives by email.** On the owner's instruction ("one email is the receipt, and another one is the key"), a purchase produces two emails: Stripe's receipt, and the bridge's own key email. The checkout handler captures the buyer address from `checkout.session.completed` (`customer_details.email`, falling back to `customer_email`) and stores it per subscription — the record a support resend would work from. When SMTP submission is configured (new `CODETOPO_SMTP_HOST` / `CODETOPO_SMTP_PORT` / `CODETOPO_SMTP_TLS` / `CODETOPO_SMTP_USERNAME` / `CODETOPO_SMTP_PASSWORD` / `CODETOPO_SMTP_FROM` environment family; the password is environment-only and never logged), the bridge sends one plain-text email per issued key — at whichever event completes the set of entitlement + live delivery + address, so checkout-first and creation-first orderings both send — carrying the full credential exactly once, the API base (`https://codetopo-api.aimlds.org`), the plan's limits read from the server's own limits table, and the support address. The delivery row gains an `emailed_at` marker (existing stores migrate on open): replays and later events never re-send, and a replacement key is emailed as its own issuance. Email is a delivery channel, never an authority — claims, revocation, and the price guard are untouched, the claim page still delivers the key, and a failed or unconfigured send changes nothing about the purchase (the failure is logged with its event id; a later event may retry inside the claim window). Unconfigured, the bridge behaves exactly as before. New transport: `lettre` (rustls) for SMTP submission. Tests: +4 in `config` (the SMTP family parses strictly, off without a host), +3 in `store` (buyer-email round-trip, the emailed marker, migration of a pre-email store), +1 in `stripe` (buyer-email precedence), +5 in `mail` (the credential appears exactly once, limits/support stated, the production transport delivering to a hermetic loopback SMTP sink, a refused connection as a clean failure), +7 in `service` (send on convergence in both event orderings, no resend on replay or later events, failure leaves purchase and claim intact, a later event retries a failed send, no address no send, a replacement key emailed as its own issuance).

- **Development log — demo Worker copy.** The public development log (previous entry) also serves from the demo Worker at codetopo.aimlds.org/log — the front-door site's own page in its own style, linked from the demo landing's nav and footer and listed in its `llms.txt` — and the `CONTRIBUTING.md` rule now names all three kept copies (this repo's two, plus the Worker's) so a release updates them together. Worker suite: 25/25. (No version bump — docs/site only.)

- **Public development log.** codetopo.aimlds.org gains a development-log page (`space-static/log/index.html`, mirrored as `space/log.html` for the Docker front door, which serves it at `/log`): the build story so far in plain words — distilled from this changelog and the GitHub release record, no dates those records do not carry — plus the directions ahead from `DESIGN.md` in phase words, no invented dates. Both landing pages link to it, the README's Docs list carries the live URL, and `CONTRIBUTING.md` records the standing rule that every release adds its log entry in the same change. The front door's test suite gains a byte-for-byte pin for `/log`. (No version bump in this PR — site/docs only, per the W3/W4 precedent.)

- **Bridge claim page: prefilled from the checkout redirect.** `GET /claim` now renders the token field prefilled from the `session` query parameter (the Stripe Payment Link's `{CHECKOUT_SESSION_ID}` redirect placeholder; `token` works as an alias), so the buyer arriving from checkout presses the button once instead of hunting down and pasting the session id. The value is HTML-escaped into the field's `value` attribute — a hostile value renders only in escaped form — and nothing auto-submits: the claim stays a deliberate act, and `POST /claim` semantics are unchanged.

- **Release-gate fix loop (fleet gate 2026-10-14, findings F-1/F-3/F-5/F-7/F-8/F-9):** Six fixes. **F-1 (data leakage):** `ApiError::body()` — the single render point behind every API response — now answers the `Internal` variant with the generic message `internal error` and writes the detail (host paths, rusqlite text) to the server's stderr only; construction sites are unchanged, and the demo front door (`space/frontdoor.py`) likewise keeps an upstream 5xx status but substitutes a generic body, so a demo failure cannot publish internals. **F-3 (planted credential):** the README quickstart no longer creates its keys file with the published example credential `demo-key:change-me` — it generates a fresh random secret (`openssl rand -hex 16`, exactly as docker-compose does) — and the legacy keys-file import now refuses any file still carrying the documented example secret, naming it in the startup error and importing nothing. **F-5 (ownerless indexes):** a legacy index (no ownership record) is no longer readable by any key by default: it stays readable while the key store holds a single active key (the self-hosted upgrade path) and is denied — answered as `unknown_index` — once a second active key exists; the new `CODETOPO_DENY_OWNERLESS_INDEXES` flag (`1`/`true` deny always, `0`/`false` allow always) overrides the default in both directions. **F-7:** `.gitignore` covers `.env*` (with a `!.env.example` negation). **F-8:** the front door's landing page is pinned by a new test suite (`space/test_frontdoor.py`) to answer every `Accept` header; no routing change was needed — the front door never gated on `Accept`, and the live demo now answers 200 for all `Accept` values. **F-9:** the README Privacy Policy section states in words how to delete your data (delete the local `.db` files; nothing is held server-side) and that export is the `snapshot` command. Tests: +1 in `error`, +2 in `keys`, +2 in `tenancy`, +2 HTTP-level in `tests/tenancy.rs` (net; the pre-existing blanket legacy test became the lone-key case), +4 in the new frontdoor suite; workspace total 301 → 308 Rust tests, 307 run and passing (one pre-existing ignored), 0 failed.

- **codetopo-bridge — Paddle billing bridge (paid-path S4):** A new workspace crate and binary, `codetopo-bridge`, turns Paddle subscription events into hosted API keys. It is a separate binary with its own small listener, so `codetopo-server` gains no surface — S1's posture (key administration is never an HTTP endpoint on the server) is untouched — and every key action goes through the server crate's `KeyStore`, the same library code the admin subcommand uses, against the same `keys.db`. Webhooks (`POST /billing/webhook`) are HMAC-verified over the raw body before any JSON is trusted (invalid signature → `400`, no state change; the secret comes from the environment and is never logged), are idempotent by Paddle event id, and are ordered per subscription so an out-of-order delivery is stale rather than destructive. The price id that maps to the `hosted` plan is environment configuration, not code. Event semantics: `subscription.created`/`activated` (or a `transaction.completed` carrying the price) issues one key per subscription, bound to the subscription id; `subscription.canceled` stores a scheduled revocation at the cancellation's effective end and the key lives until then; `subscription.past_due` marks the entitlement past due and revokes after a 7-day grace unless the subscription returns to active first; a restore after revocation issues a replacement key (the old secret is never resurrected). Because the store keeps only hashes, a key's plaintext exists only at issuance: the bridge seals it (AES-256-GCM under an environment-only key) as a delivery the buyer claims exactly once — the checkout success redirect lands on the bridge's claim page (`GET`/`POST /claim`) carrying the transaction id, the subscription id from the Paddle receipt is the second claim token, and claiming or seven days unclaimed destroys the plaintext copy. A sweep at startup and every minute applies due revocations and purges expired deliveries. Sandbox objects for the proof run (product "codetopo Hosted", US$19/month price, webhook destination at a placeholder URL the deploy step replaces) exist in the Paddle sandbox only; deployment is S5.

- **codetopo-server — abuse envelope (paid-path S3):** Per-plan limits now bind the hosted tier, all read from one table keyed by the key's plan (`src/limits.rs`). The hosted plan (US$19/month per key) allows 25 active indexes, 10,000 queries per month, and 50,000 files per index; `self-hosted` keys carry no limits, and an unrecognized plan name resolves to the hosted envelope, never to unlimited. Enforcement: creating an index past the active-index quota, querying past the monthly quota, and indexing a tree past the file cap are each refused with a new `429` error (`index_quota_exceeded`, `query_quota_exceeded`, `file_quota_exceeded` — the messages name the limit, the current count, and the month, which is the UTC calendar month); a second concurrent index operation for one key is refused `429 index_concurrency_exceeded`; query endpoints carry a 30 s per-request timeout (new `504 request_timeout`) and a global cap of 4 concurrent queries sized to the Aetheris host (4 vCPUs; a query loads its whole graph into RAM). Monthly usage is counted from the metering log itself — the same records the metering contract pins — so usage and billing cannot disagree, and index creation is never counted as a query. The `keys list` admin command now shows each key's active indexes and monthly queries beside the limits they are measured against. S2's interim ingest caps (64 MiB upload, 512 MiB tree, 100k archive entries, 120 s clone) became the hosted row of the limits table. The clone's DNS-rebinding window is closed where git supports it: validated addresses are pinned into the clone via `http.curloptResolve`, so git connects to the addresses validation approved (a git build too old to know the option ignores it silently — that residual is documented in `src/ingest.rs`). `schemas/openapi.yaml` documents the new 429/504 responses; no route or success-shape changes.

- **codetopo-server — hosted ingest (paid-path S2):** `POST /v1/index` gains two ways for a key to get a repository in, alongside the existing host-local `repo_path`. A JSON `repo_url` names a public HTTPS git URL: the server validates it (HTTPS only, no embedded credentials, port 443 only, and the host must resolve exclusively to public addresses — the SSRF guard; a check-then-clone rebinding window remains and is documented in the code) and shallow-clones it with hardened git settings (no redirects, no submodule recursion, no LFS smudge, 120 s timeout, 512 MiB tree cap). An archive upload sends a `.zip` or `.tar.gz` as the request body (64 MiB cap), extracted entry by entry with traversal and link entries refused and extracted bytes counted as written. Both modes land in a server-owned workdir, `<data_dir>/ingest/<index_id>/`, whose lifetime is the index's: expiry now deletes the tree with the database, and a failed ingest leaves no partial tree. Plan gating: `repo_path` is restricted to `self-hosted` keys — a hosted key naming a host path is refused `invalid_repo_path` — while both hosted modes are open to every plan. New error code `invalid_repo_url`; the route set and response shape are unchanged, and `schemas/openapi.yaml` documents the new modes.

- **codetopo-server — runtime key store + index tenancy (paid-path S1):** API keys move from a flat file loaded once at startup into a SQLite store in the data dir (`keys.db`). Secrets are stored only as salted SHA-256 hashes; the `key_id:secret` bearer format and constant-time comparison are unchanged. Keys carry a plan name and an active/revoked status and can be issued, rotated, and revoked at runtime through a new admin subcommand (`codetopo-server keys issue|rotate|revoke|list`) — changes take effect on the running server with no restart. The legacy flat keys file is imported automatically on first boot when the store is empty, so existing deployments keep working. Every index created through the API is now owned by its creating key: the graph query routes answer another key's index exactly like a missing one (404 `unknown_index`). Indexes record a creation time and expire after `CODETOPO_INDEX_TTL_SECS` when the operator sets one; expired indexes are refused and their files deleted (at startup and lazily on access). Indexes written before tenancy (no ownership record) stay readable by any authenticated key. Metering records gain the key's `plan`, so monthly per-key query/index counters can be read straight off the JSONL log. No route, response, or wire-format changes.

## [0.5.0] — 2026-10-14

- **Version unification:** all seven crates — codetopo-core, codetopo-extract, codetopo-store, codetopo-cli, codetopo-server, codetopo-mcp, codetopo-acp — move to a single version line, 0.5.0. The workspace previously carried three versions at once (0.3.3, 0.3.5 for codetopo-mcp, 0.4.0 for codetopo-acp). Internal dependency requirements move with them; the tool catalogue is regenerated from the 0.5.0 binary. No behavior changes in any crate.

## [0.3.5] — 2026-10-13

- **codetopo-mcp 0.3.5:** every tool in `tools/list` now carries a human-readable `title` and MCP `annotations` (read-only, destructive, and open-world hints), for client and directory conformance. No tool behavior changes. Other crates unchanged.
- README gains a Privacy Policy section.

## [0.4.0] — 2026-10-10

- **codetopo-acp 0.4.0 — ACP Terminal Auth (trust setup):** the agent now advertises one terminal auth method, `codetopo-trust-setup`, until setup is recorded (and only to clients declaring terminal-auth support, per the spec). `codetopo-acp login` provisions the HMAC proof-verification key locally (0600, hidden entry; keyless setup offered) — codetopo has no accounts, and the one secret in its trust model is that key. Snapshots the agent takes are signed when a key is provisioned, and answers cite the certificate serial and signer fingerprint. Design: `docs/acp-trust-setup.md`. Other crates unchanged at 0.3.3.

## [0.3.3] — 2026-10-10

All seven crates; no code changes. Fixes how the crates appear on crates.io.

- The hand-rolled crates.io publisher sent the README *filename* where the registry protocol expects the README *content*, so every version up to 0.3.2 renders a one-line placeholder instead of the README on its crates.io page. 0.3.3 is published with the corrected publisher; the full README (including the MCP Registry identity line) now renders. Functionality is identical to 0.3.0.
- `codetopo-mcp` 0.3.1 and 0.3.2 were registry-listing iterations on the way here (ownership marker, then the marker as visible text); 0.3.3 supersedes them and restores one version across the workspace.

## [0.3.2] — 2026-10-10

`codetopo-mcp` only; no code changes in any crate.

- The MCP Registry ownership marker in the README is now a visible line: the registry validates against crates.io's *rendered* README, which strips HTML comments, so the comment form in 0.3.1 did not validate. All other crates remain at 0.3.0; `codetopo-mcp` 0.3.1 is superseded by 0.3.2.

## [0.3.1] — 2026-10-10

`codetopo-mcp` only; no code changes in any crate.

- README carries the `mcp-name: io.github.nrupala/codetopo` ownership marker the official MCP Registry requires for Cargo packages, so v0.3.1 supersedes v0.3.0 for `codetopo-mcp` on crates.io. All other crates remain at 0.3.0.

## [0.3.0] — 2026-10-10

### Added

- **The Space bundle recaptures itself.** The Hugging Face Space's
  data (`space-static/data/`) was a one-time capture from 0.2.5;
  the demo-graph workflow refreshed only the aimlds demo, so the
  Space kept rendering the old snapshot after merges. A new
  `space-recapture` workflow now rebuilds `codetopo-server` from
  every merge to main, re-indexes this repository with it, and
  commits a fresh byte-for-byte capture
  (`scripts/capture-space-bundle.sh`) — but only after the honesty
  gates pass (snapshot counts equal stats totals; snapshot audit
  head equals the verify head with a valid chain). The bot's data
  commit is what `hf-sync` then mirrors to the Space; two guards
  (paths-ignore + a head-commit check) keep the workflow from
  re-triggering on its own output.
- **Deterministic render: `codetopo render`.** A layout spec
  (JSON) and an SVG drawing that are pure functions of the stored
  graph — same graph in, same bytes out, on any machine. Layers
  follow the longest dependency path over the SCC-condensed
  dependency DAG (cycles share a layer; containment is drawn but
  never layers); in-layer order is by node id; coordinates are
  integer grid cells. The spec is self-contained and versioned
  (`layout_version: 1`); the SVG consumes the spec alone. Rules
  and size behaviour: `docs/render.md`.
- **Self-hosted container.** A multi-stage `Dockerfile` (four
  binaries on a slim Debian runtime — they link only libc/libm)
  and a `docker-compose.yml` that runs the HTTP API over a `/data`
  volume, with the server's env contract
  (`CODETOPO_BIND`/`DATA_DIR`/`KEYS_FILE`/`METERING_LOG`) wired in.
  The server still refuses to start without a keys file.
  Workspace crates version 0.2.8.

### Fixed

- **MCP `tools/call` results are CallToolResults now.** Success
  results were the tool's bare JSON object; the MCP schema requires
  a `content` array, and strict SDK clients parsed the old shape as
  no content at all (in-house tests read the bare object and stayed
  green — the defect class this fix's wire-level test exists to
  catch). Results now carry `content` (the result JSON as a text
  block) plus `structuredContent` with the same object.
- **ACP speaks ACP.** The agent's wire protocol was a home-grown
  dialect: `protocolVersion` as the string `"1.0"` (the spec's is
  the integer `1`), string prompts (the spec's are content-block
  arrays), invented `session/new` result fields, and answers in a
  custom field instead of a streamed `agent_message_chunk` +
  `stopReason`. Rewritten to ACP v1: spec handshake (integer
  version, honest capabilities — no session loading, text-only
  prompts, no auth methods), `{sessionId}` results, content-block
  prompt text, streamed chunks, `end_turn`/`refusal` stop reasons,
  and `session/cancel` handled as a notification. Workspace crates
  version 0.2.7; live frames from both protocol binaries validate
  against the protocols' official JSON schemas
  (`docs/protocol-proof.md`).

### Added

- **Real import resolution in the graph builder.** Discovery named
  the packages; this makes their imports *link*. `use a::b::Item`
  now lands on module `a::b` (the longest module prefix — before,
  every Rust `use` of an item became an external stub); bare
  workspace-package imports (npm `from 'ui-kit'`, cross-crate
  `use`) resolve through their bindings to the defining module,
  with hyphen/underscore normalization; an extractor's
  `external::` pre-mark no longer doubles (`external::external::std`
  is gone); a package's root module is its package node — the id
  collision that silently dropped root modules and rejected their
  imports with `UnknownSymbol` is fixed; and `crate::`-qualified
  calls resolve into the caller's package. Also: `pyproject.toml`
  (`[project]` / `[tool.poetry]`) joins Cargo.toml and package.json
  as a discovery manifest (CODE_SCHEMA names PyPI projects as
  packages). Rules documented in CODE_SCHEMA §3.1.
- **Package/workspace discovery in the indexer.** Indexing now
  finds the manifest that owns each file — a Cargo.toml
  `[package]` name or a package.json `name`, nearest manifest
  wins — and indexes the file under that package with paths
  relative to the package root, which is what the extract
  contract always specified. Until now one caller-chosen package
  and repo-relative paths covered the whole tree, so a workspace
  indexed as `corpus::crates::codetopo-core::src::…`; ids now
  read `codetopo-core::…`. Files under no manifest keep the
  caller's package (`--package`, defaulting to the repo
  directory name) exactly as before. Workspace crates version
  0.2.6.

### Added

- **Scale gate passed at 10,000 files** (`docs/scale-gate.md`). A
  deterministic 10k-file fixture (generator:
  `scripts/gen_scale_fixture.py`; gate test:
  `codetopo-cli/tests/scale_gate.rs`, run deliberately in release
  mode) indexes in ~3.5 s at 134 MB peak RSS (46,001 nodes / 90,017
  edges), and the long-pole queries answer in well under a second
  in-process. **Defect found and fixed:** cycle detection (Tarjan
  SCC) was recursive — a 10,000-deep call chain overflowed a 2 MB
  thread stack and aborted the process; it is now an iterative
  Tarjan with identical results, so depth is bounded by heap, not
  by the caller's stack. Also corrected `schemas/README.md`'s MCP
  tool count (eight → twelve). All seven crates bumped
  `0.2.4` → `0.2.5`.
- **Contract drift gates + ACP version honesty.** The published
  contracts can no longer drift silently: workspace tests (run in CI)
  now fail when `schemas/mcp-tools.json` differs from the live MCP
  tool catalogue or its recorded server version, and when
  `schemas/openapi.yaml`'s info version or route set differs from
  what `codetopo-server` serves (its version had been stale at 0.1.0;
  now 0.2.4). The ACP agent's `initialize` handshake reports the
  crate version instead of a hardcoded `0.1.0`. All seven crates
  bumped `0.2.3` → `0.2.4`.
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

- **README quickstart verified end-to-end; three drifts corrected**
  (transcript in `docs/quickstart-verification.md`). The install
  command now names the four packages — a bare
  `cargo install --git https://github.com/nrupala/codetopo` fails with
  cargo's "multiple packages with binaries found" error. The example
  query id now matches what indexing `~/my-project` actually produces
  (`my-project::…` — the directory name, hyphen included). The
  `descendants` example output header now matches the binary
  (`2 descendants of …`, count first). The fixture numbers (15 nodes /
  23 edges and the full `stats` breakdown) were re-run against the
  built CLI and confirmed exact, and the previously undocumented
  `restore` command is now listed with the other queries. (Rebased
  onto the final-shape main before merge: the `stats` breakdown's
  `references` kind is `reads` since extractor conformance (W4) —
  the README example line was updated to match the current binary
  during the rebase, and the fixture was re-run to confirm it.) (The
  `0.2.1` → `0.2.2` crate bump this change originally carried was
  superseded while the PR waited: the final-shape work took the
  crates to `0.2.3`, so this change ships no version bump.)

### Added

- **Portfolio certification rollout.** `CONTRIBUTING.md` now documents
  the PR-flow discipline (draft PR → tests green → owner merges;
  every PR adds a CHANGELOG entry under Unreleased and bumps crate
  versions; merge commits reference PR numbers; releases tagged
  `vX.Y.Z`). All seven crates bumped `0.2.0` → `0.2.1`.

### Changed

- **Release 0.3.0 — first crates.io publication.** All seven
  crates publish at 0.3.0 with full registry metadata
  (descriptions, keywords, categories, homepage); internal
  dependencies now carry registry versions alongside their paths.
  Version, tag, changelog, GitHub release, and the crates.io
  versions are reconciled at this gate — the v0.2.0 tag predates
  the final 0.2.x line and remains as-is for history.

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
