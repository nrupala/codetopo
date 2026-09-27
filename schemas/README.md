# codetopo schemas

Machine-readable contracts for the two agent-facing surfaces of `codetopo`,
kept separate from the server that implements them so client generators,
proxies, and test harnesses can consume them without building the workspace.

| File | Contract | Transport |
|---|---|---|
| [`openapi.yaml`](openapi.yaml) | HTTP query API (`codetopo-server`) | HTTP/JSON, OpenAPI 3.1.0 |
| [`mcp-tools.json`](mcp-tools.json) | MCP tool catalogue (`codetopo-mcp`) | stdio, JSON-RPC 2.0 |
| [`LICENSE-APACHE`](LICENSE-APACHE) | Licence for this directory | — |

Both files are generated-where-possible and checked against the running
servers. They are not hand-written prose pretending to be a contract.

## `openapi.yaml`

Describes `POST /v1/index` plus seven reads under `/v1/graphs/{id}/…`, all
behind bearer auth, all read-only except the index call.

### Endpoints

| Method | Path | Answers |
|---|---|---|
| `POST` | `/v1/index` | Index a repository tree, return a new index id. |
| `GET` | `/v1/graphs/{id}/descendants` | What this node affects. |
| `GET` | `/v1/graphs/{id}/ancestors` | What depends on this node. |
| `GET` | `/v1/graphs/{id}/blast-radius` | What a failure here would take with it. |
| `GET` | `/v1/graphs/{id}/path` | Shortest typed path between two nodes. |
| `GET` | `/v1/graphs/{id}/stats` | Aggregate node and edge counts. |
| `GET` | `/v1/graphs/{id}/verify` | Audit-chain verification for the index. |
| `GET` | `/v1/graphs/{id}/snapshot` | Portable frozen export of the whole graph. |

### Authentication

```http
Authorization: Bearer <key_id>:<secret>
```

One opaque token: the key id and its secret joined by a colon. Keys live in
the file named by `CODETOPO_KEYS_FILE` (default `./api_keys`).

Every failure mode — missing header, malformed header, unknown key, wrong
secret — returns the *same* `401` with a constant message and a
`WWW-Authenticate: Bearer realm="codetopo"` challenge, so a prober learns
nothing from the response. A rejected request is not metered, because
metering records accepted work and should not be a way to flood the log.

Scheme matching is case-sensitive: `bearer` is rejected.

### Errors

Flat, and the status lives only in the HTTP status line:

```json
{ "error": "missing_param", "message": "node is required" }
```

`error` is a stable code. `message` is prose for a human and may change
without notice. The nine codes:

| Code | Status | Meaning |
|---|---|---|
| `invalid_body` | 400 | Body is not valid JSON for this endpoint. |
| `invalid_repo_path` | 400 | `repo_path` does not name a readable directory. |
| `missing_param` | 400 | Parameter absent, or empty after trimming. |
| `invalid_param` | 400 | Parameter present but unparseable or out of range. |
| `unauthenticated` | 401 | Credential missing, malformed, or wrong. |
| `unknown_index` | 404 | No index with that id. |
| `unknown_node` | 404 | The query endpoint's node is not in the graph. |
| `no_path` | 404 | No path connects the two nodes. |
| `internal` | 500 | Unexpected server-side failure. |

### Response shape

Closure responses are aggregate-first: `total` is the full size of the
answer and arrives before any list, so a caller can decide whether it can
afford the list from a single integer.

```json
{ "index_id": "…", "kind": "descendants", "total": 137, "truncated": true, "nodes": ["…"] }
```

* `limit` (default `20`, max `1000`) caps the default list.
* `expand=true` lifts the cap and returns full node objects instead of ids.
* `truncated` is true exactly when `!expand && total > limit`.

Every list is deterministically ordered — adjacency is sorted by
`(neighbor id, edge-kind rank)` at build time — so traversals are
reproducible across runs and machines.

### Server configuration

| Variable | Default | Effect |
|---|---|---|
| `CODETOPO_BIND` | `127.0.0.1:8080` | Listen address. Loopback by default. |
| `CODETOPO_DATA_DIR` | `./data` | Root for index storage. |
| `CODETOPO_KEYS_FILE` | `./api_keys` | Bearer credentials. |
| `CODETOPO_METERING_LOG` | `./metering.log` | Accepted-request log. |

An empty value is treated as unset. The default bind is loopback because the
API holds a full source-code index and is meant to sit behind an SSH tunnel
or a mesh peer, not to listen on a public interface.

### Conventions, and where the spec departs from the design doc

Both are recorded in full in `openapi.yaml` itself, under `info.description`.
The short version: errors are flat rather than nested (the design's §7 nests
them under an `error` object with a `status` field), and `expand` wins over
`limit` where the design's §3.1 contradicts itself (it says `expand=true`
returns the full id list and, in the same table, that `limit` caps ids
returned when `expand=true`). CLI parity was used as the tie-breaker.

## `mcp-tools.json`

The verbatim `result.tools` payload of a JSON-RPC `tools/list` call against
`codetopo-mcp` over stdio. Seven tools:

`codetopo_index`, `codetopo_descendants`, `codetopo_ancestors`,
`codetopo_blast_radius`, `codetopo_path`, `codetopo_stats`,
`codetopo_verify`

Regenerate with:

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' \
  | cargo run -q -p codetopo-mcp
```

### How the two surfaces line up

Every MCP tool has an HTTP twin except one:

| MCP tool | HTTP equivalent |
|---|---|
| `codetopo_index` | `POST /v1/index` |
| `codetopo_descendants` | `GET /v1/graphs/{id}/descendants` |
| `codetopo_ancestors` | `GET /v1/graphs/{id}/ancestors` |
| `codetopo_blast_radius` | `GET /v1/graphs/{id}/blast-radius` |
| `codetopo_path` | `GET /v1/graphs/{id}/path` |
| `codetopo_stats` | `GET /v1/graphs/{id}/stats` |
| `codetopo_verify` | `GET /v1/graphs/{id}/verify` |
| — | `GET /v1/graphs/{id}/snapshot` |

The asymmetry is deliberate and runs in both directions. The MCP surface is
not wrapped by auth because a stdio server is reached by spawning the
process, which is its own authorisation; the HTTP surface is. Conversely,
the full-graph export is a bulk endpoint that would blow past an agent's
context window if returned as a tool result, so it is HTTP-only.

One more deliberate difference: MCP tools take `db_path` and read the index
straight off disk, so the caller chooses which graph to answer from. HTTP
clients get `id` in the path and are always confined to the one graph the
server loaded at startup.

## Licence

This directory is Apache-2.0 (see `LICENSE-APACHE`). The rest of the
`codetopo` workspace is AGPL-3.0-or-later. `schemas/` is the only Apache-2.0
part, so tooling built on these contracts does not inherit the copyleft.
