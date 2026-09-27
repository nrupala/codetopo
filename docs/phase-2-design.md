# codetopo Phase 2 — Agent Query API (HTTP + MCP)

**Status:** Design, 2026-09-27. Phase 1 merged to `main` (`78e8b19`).
**Branch:** `phase-2-api`
**Ownership:** Owned by Nrupal Akolkar · Built with Muse by Meta.
**License:** AGPL-3.0-or-later, matching the Phase 1 crates.

## 1. Goal

Expose codetopo's graph queries as a **machine-first API** — the joint between
the open AGPL core and a future paid hosted API.

codetopo remains a **pure code visualization engine**. It answers structural
questions about source trees — what calls what, what a change breaks, how two
symbols are connected. It carries **no domain-specific logic, ever**: no
statistics-specific rules, no domain ontology, no vertical semantics. Domain
meaning lives in *composing agents* that sit on top of this API and supply the
vocabulary; codetopo supplies only the structural substrate.

The API is therefore deliberately **thin, generic, and total**: one indexing
operation plus a small set of graph queries, all of which are pure functions of
an indexed repository. There is no scoring, no ranking heuristic, and no
opinionated default that would bake a use case into the engine.

### 1.1 Why an API surface at all

The CLI (Phase 1, `codetopo-cli`) already proves the query semantics. Phase 2
does not add new graph capability; it adds *reachability*:

- **MCP** — so an agent in-process can call the queries as tools, with the
  aggregate-first response discipline already designed into `CODE_SCHEMA.md` §6.
- **HTTP** — so a hosted, multi-tenant deployment can meter and bill by key.

Both surfaces are thin adapters over the same in-process handler layer, so the
two can never drift: the MCP tool `codetopo_stats` and `GET /v1/graphs/{id}/stats`
must return the same bytes for the same index.

### 1.2 Explicit non-goals

| Non-goal | Rationale |
|---|---|
| Domain/vertical semantics in any crate | Violates the purity rule; belongs in composing agents. |
| Mutating endpoints (write/delete graph) | Phase 2 indexes and reads. Annotation writes stay local. |
| New query semantics | Phase 1 semantics are the contract; §6 of `CODE_SCHEMA.md` is binding. |
| Public internet exposure, TLS termination, rate limiting | Deployment concerns of the future hosted tier, not the engine. |
| Streaming / incremental indexing | v1 is whole-repo index-then-query. |

## 2. Crate layout

| Crate | Purpose |
|---|---|
| `codetopo-core` | *(unchanged)* schema, graph, queries, snapshots |
| `codetopo-extract` | *(unchanged)* tree-sitter extractors |
| `codetopo-store` | *(unchanged)* SQLite + audit chain |
| `codetopo-cli` | *(unchanged)* `codetopo` CLI, also consumed as a library |
| `codetopo-api` | **(new)** HTTP server, auth, metering, storage layout, request handlers |
| `codetopo-mcp` | **(new)** MCP stdio server exposing the same handlers as tools |

`codetopo-api` and `codetopo-mcp` are both thin. Neither contains graph logic:
both call `codetopo-core` queries and `codetopo-store` reads, exactly as
`codetopo-cli` does today. Indexing is shared by depending on `codetopo-cli` as
a library (`index_repo`, which already returns `IndexReport { files_indexed,
files_failed, … }`), which is what makes exit criterion (a) — API stats identical
to CLI stats — true by construction rather than by coincidence.

## 3. Endpoint table

All responses are JSON. All routes are versioned under `/v1`. All routes require
authentication (§4).

| Method | Route | Request | Response |
|---|---|---|---|
| `POST` | `/v1/index` | body `{repo_path, package?}` | `{index_id, files_indexed, files_failed, nodes, edges, diagnostics}` |
| `GET` | `/v1/graphs/{id}/descendants?node=&expand=&limit=` | — | transitive closure (what this node affects) |
| `GET` | `/v1/graphs/{id}/ancestors?node=&expand=&limit=` | — | reverse closure |
| `GET` | `/v1/graphs/{id}/blast-radius?node=&expand=&limit=` | — | breakage propagation |
| `GET` | `/v1/graphs/{id}/path?from=&to=` | — | shortest typed path |
| `GET` | `/v1/graphs/{id}/stats` | — | aggregate node/edge counts |
| `GET` | `/v1/graphs/{id}/verify` | — | audit-chain verification |
| `GET` | `/v1/graphs/{id}/snapshot` | — | JSON snapshot export |

### 3.1 Query parameters

| Param | Applies to | Default | Meaning |
|---|---|---|---|
| `node` | descendants, ancestors, blast-radius | *required* | Node id to start from. |
| `expand` | descendants, ancestors, blast-radius | `false` | `true` returns the full id list; `false` returns the aggregate summary only. |
| `limit` | descendants, ancestors, blast-radius | `20` | Max ids returned when `expand=true`. Must be `1..=1000`. |
| `from`, `to` | path | *required* | Endpoint node ids. |

### 3.2 Correspondence with the Phase 1 CLI

The API is a transport rename of CLI surface, not a redesign. Parity is
enforced by test, not by convention.

| API route | CLI command |
|---|---|
| `POST /v1/index` | `codetopo index <repo> --db <db> [--package <p>]` |
| `GET …/descendants?node=` | `codetopo descendants <id> --db <db> [--expand] [--limit N]` |
| `GET …/ancestors?node=` | `codetopo ancestors <id> --db <db> [--expand] [--limit N]` |
| `GET …/blast-radius?node=` | `codetopo blast-radius <id> --db <db> [--expand] [--limit N]` |
| `GET …/path?from=&to=` | `codetopo path <a> <b> --db <db>` |
| `GET …/stats` | `codetopo stats --db <db>` |
| `GET …/verify` | `codetopo verify --db <db>` |
| `GET …/snapshot` | `codetopo snapshot --db <db> --out <f>` |

`{id}` in the route is the *index id* (a UUID, §6), replacing the CLI's
`--db <path>`. Node ids are passed as query parameters, never interpolated into
the path, so node ids containing `/` cannot break routing.

## 4. Auth design

**Bearer API-key middleware.** Every route is wrapped by a layer that:

1. Reads the `Authorization` header.
2. Requires the exact form `Bearer <key_id>:<secret>`.
3. Splits on the first `:` into `(key_id, secret)`.
4. Looks `key_id` up in the keys file, compares the secret in **constant time**.
5. On success, attaches `key_id` to the request extensions for metering (§5).
6. On any failure, short-circuits with `401` and a JSON error body — no handler
   runs, nothing is metered as a success.

**Keys file.** Path from env `CODETOPO_KEYS_FILE`, default `./api_keys`. One key
per line:

```
# key_id:secret
research-agent:s3cr3t-value
billing-bot:another-secret
```

Blank lines and `#` comments are skipped. The file is read once at startup into
an in-memory map; a missing file is a startup error (fail closed — an API with
no keys must not accept anonymous traffic), while an *empty* key set is also a
startup error for the same reason. The file is never re-read per request; key
rotation is a restart, which is acceptable for v1 and keeps the hot path free of
I/O.

**Why a shared-secret map and not JWTs.** The hosted tier needs per-key identity
for metering, and the client population is agents running on a developer's
machine, not third-party OAuth clients. A static key file is the smallest thing
that yields both, with no token-refresh machinery to get wrong. A future tier
can swap the middleware for JWT validation without touching a handler, because
handlers never see the secret — only the `key_id`.

## 5. Metering design

Every request — **including rejected ones** — is appended as exactly one JSON
line to the metering log. Path from env `CODETOPO_METERING_LOG`, default
`./metering.log`. One line per request keeps the log stream-appendable and
greppable without a parser:

```json
{"ts":"2026-09-27T04:31:12.482Z","key_id":"research-agent","method":"GET","path":"/v1/graphs/8f1e…/descendants","status":200,"response_bytes":512,"duration_ms":37}
```

| Field | Type | Notes |
|---|---|---|
| `ts` | RFC 3339 UTC | Server clock, millisecond precision. |
| `key_id` | string | `"anonymous"` when the request failed auth (§4). |
| `method` | string | HTTP method; `"stdio"` for MCP tool calls. |
| `path` | string | Route + query string, secrets never included. |
| `status` | int | HTTP status, or a synthetic `0` for MCP transport failure. |
| `response_bytes` | int | Serialized body length in bytes. |
| `duration_ms` | int | Wall-clock handler time, excluding queueing. |

Writes go through a `Mutex<BufWriter<File>>` and are flushed per line, so a crash
loses at most the in-flight record and a concurrent flush cannot interleave two
records mid-line. Metering failures are **logged and swallowed, never propagated
into the response** — a metering outage must not fail a read-only query.

## 6. Storage design

Data directory from env `CODETOPO_DATA_DIR`, default `./data`.

Each indexing run mints a UUID `index_id` and writes its SQLite database to
`<data_dir>/<index_id>.db`. The db holds the graph *and* its audit chain, exactly
as `codetopo index --db` does — the API adds no storage format of its own.
`{id}` in every route is looked up as `<data_dir>/<id>.db`.

`index_id` is a UUID v4 rather than a repo name for three reasons: a repo can be
re-indexed without clobbering the previous index (agents can compare two states
of one tree), a repo path is not a safe filename, and it leaks neither the
customer's directory layout nor the project name into logs and URLs.

**Concurrency.** SQLite is synchronous and `rusqlite` blocks the thread, as does
extraction. Every handler that touches a db — including `POST /v1/index`, which
walks a whole tree — runs inside `tokio::task::spawn_blocking`. Each request
opens its own connection to the index db rather than sharing one handle, so a
slow query cannot block the server's accept loop or another tenant's request.
Indexing is unbounded in duration and is expected to be; the endpoint carries no
timeout of its own, and a reverse proxy in front of a deployment is responsible
for its own read/write timeouts.

## 7. Error mapping

Uniform JSON error body, one shape everywhere:

```json
{"error": {"code": "not_found", "message": "unknown index id 8f1e…", "status": 404}}
```

| Condition | Status | `code` |
|---|---|---|
| No `Authorization` header, malformed scheme, unknown `key_id`, or wrong secret | `401` | `unauthenticated` |
| Unknown index id (`<data_dir>/<id>.db` does not exist) | `404` | `unknown_index` |
| Unknown node id (valid index, node not in graph) | `404` | `unknown_node` |
| Missing required query param (`node`, `from`, `to`) | `400` | `missing_param` |
| Non-integer or out-of-range `limit` (not `1..=1000`) | `400` | `invalid_param` |
| Malformed JSON body on `POST /v1/index` | `400` | `invalid_body` |
| `repo_path` missing or not a directory | `400` | `invalid_repo_path` |
| Indexing/storage failure | `500` | `internal` |

The `404`-vs-`400` split is deliberate: a *syntactically* valid request that
names something nonexistent is `404`, while a request that is malformed is
`400`. An agent can therefore distinguish "I mistyped the query" from "that
symbol is not in this index" without parsing prose. The distinction is preserved
from the CLI, which likewise rejects an unknown node id rather than returning an
empty list.

## 8. Aggregate-first output discipline

Per `CODE_SCHEMA.md` §6, default responses are counts and summaries; full dumps
require an explicit `expand`.

An id-list endpoint returns the aggregate form by default:

```json
{
  "index_id": "8f1e…",
  "node": "mycrate::net::Client::connect",
  "query": "descendants",
  "total": 137,
  "ids": []
}
```

With `expand=true` the `ids` array is populated, capped at `limit` (default 20,
max 1000) and accompanied by `truncated: true` when `total > ids.len()`:

```json
{
  "index_id": "8f1e…",
  "node": "mycrate::net::Client::connect",
  "query": "descendants",
  "total": 137,
  "ids": ["mycrate::net::Client::send", "…"],
  "truncated": true
}
```

`total` is always the true closure size, so an agent can budget its next query
without ever having asked for the full list. `GET /v1/graphs/{id}/stats` and
`/verify` are aggregates by construction and take no `expand`. `path` returns the
full typed path because a partial path is not a meaningful object. `snapshot` is
an explicit whole-graph export and is the one endpoint that is *expected* to be
large — it is the §7 "JSON as universal export pathway" endpoint, not a query.

## 9. MCP surface

MCP stdio server in `codetopo-mcp`, speaking the same handler layer as the HTTP
routes. Tools exposed:

| Tool | Backs |
|---|---|
| `codetopo_index` | `POST /v1/index` |
| `codetopo_descendants` | `GET …/descendants` |
| `codetopo_ancestors` | `GET …/ancestors` |
| `codetopo_blast_radius` | `GET …/blast-radius` |
| `codetopo_path` | `GET …/path` |
| `codetopo_stats` | `GET …/stats` |
| `codetopo_verify` | `GET …/verify` |
| `codetopo_snapshot` | `GET …/snapshot` |

Tool arguments mirror the HTTP parameters one-for-one (`node`, `expand`,
`limit`, `from`, `to`), so an agent that has learned one surface has learned
both. MCP runs in-process and is not authenticated — it inherits the trust of
the stdio pipe it was spawned on — but it **is** metered, with `method: "stdio"`
(§5). The stdio transport requires `initialize` followed by `tools/list` before
`tools/call` is legal; the test client in §10(c) exercises exactly that sequence.

## 10. Exit criteria

Phase 2 is done when all of the following hold.

**(a) Server boots; index parity.** The server starts and serves
`POST /v1/index` against a fixture repository. The returned `nodes`, `edges`,
`files_indexed`, and `files_failed` are identical to the same values printed by
`codetopo index` and `codetopo stats` on that repository.

**(b) Query round-trip parity.** One HTTP round-trip of
`GET /v1/graphs/{id}/descendants?node=…&expand=true` returns a node set identical
to `codetopo descendants <node> --db <db> --expand` on the same index — same
members, same order, same `total`.

**(c) MCP handshake and a tool call.** A test client script completes the
`initialize` + `tools/list` handshake over stdio, and one `tools/call` of
`codetopo_stats` returns valid JSON.

**(d) Auth and metering.** A request with no `Authorization` header returns
`401` with a JSON body, and the metering log contains entries whose `key_id`
reflects the authenticated key.

**(e) Green build.** `cargo test` is fully green and `cargo clippy` reports zero
warnings.

## 11. Constraints carried from Phase 1

- **License headers.** Every new Rust source file opens with exactly:
  ```rust
  // Copyright (C) 2026 Nrupal Akolkar
  // SPDX-License-Identifier: AGPL-3.0-or-later
  // Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.
  ```
- **Purity.** No crate in this workspace may encode domain-specific logic.
  `codetopo-api` and `codetopo-mcp` add transport, auth, and metering — never
  meaning.
- **Schema is binding.** New response fields are additive (minor); removing or
  renaming one is a major bump. Handlers read through `codetopo-core` queries so
  the CLI, HTTP, and MCP surfaces cannot diverge.
