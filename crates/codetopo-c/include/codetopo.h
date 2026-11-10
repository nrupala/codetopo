/* Copyright (C) 2026 2351485 ALBERTA INC.
 * SPDX-License-Identifier: AGPL-3.0-or-later
 * Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
 *
 * codetopo — C ABI foundation.
 *
 * This header is the stable C surface every other language face stands
 * on: R (extendr / .Call), Julia (ccall), C and C++ (directly or via
 * cxx), and any other FFI caller. It is hand-written and pinned by
 * tests in the codetopo-c crate: a test fails if a function declared
 * here is not an exported symbol of the built cdylib, or if an export
 * appears that is not declared here.
 *
 * BOUNDARY SHAPE
 *   The boundary is CODE_SCHEMA JSON strings (CODE_SCHEMA section 7:
 *   JSON is the universal pathway). Every answer this library returns
 *   is the same JSON the CLI, HTTP, and MCP surfaces produce for the
 *   same question — parity is by construction, because the queries are
 *   served by the same codetopo-cli library builders those surfaces
 *   use. There are no C structs mirroring the graph schema; the schema
 *   travels as JSON and evolves under CODE_SCHEMA versioning, not
 *   under this header's ABI.
 *
 * OWNERSHIP
 *   - Every `char *` this library returns (query/index answers and
 *     error strings) is heap-allocated for the caller and MUST be
 *     released with codetopo_free_string(). Passing any other pointer
 *     to codetopo_free_string() is undefined behaviour.
 *   - The two exceptions are codetopo_version() and
 *     codetopo_schema_version(): they return pointers to static
 *     strings owned by the library. Do NOT free them; they stay valid
 *     for the life of the process.
 *   - A `codetopo_graph *` returned by codetopo_open_db() or
 *     codetopo_load_snapshot() is owned by the caller and MUST be
 *     released with codetopo_graph_free(). Handles are not shared
 *     between threads for concurrent queries; use one handle per
 *     thread (queries take the handle by const pointer and never
 *     mutate it, but the guarantee is per-handle, not global).
 *   - Input strings are borrowed for the duration of the call only;
 *     the library copies whatever it keeps.
 *
 * ERRORS
 *   Fallible functions return NULL on failure and, when `err_out` is
 *   not NULL, store a heap-allocated JSON object there:
 *
 *       {"code": "<stable code>", "message": "<human-readable detail>"}
 *
 *   which the caller frees with codetopo_free_string(). On success
 *   *err_out is set to NULL. Error codes reuse the vocabulary of the
 *   existing surfaces: "unknown_symbol" (a node id is not in the
 *   graph), "unknown_index" (no index database at the path),
 *   "unsupported_schema_version" (CODE_SCHEMA section 8 major
 *   mismatch), "invalid_param" (a missing/invalid argument, an
 *   unknown operation, or — for rank — any parameter other than `by`
 *   or `limit`, named in the message), "no_path", "io_error",
 *   "store_error", and "internal" (including a caught panic: no panic
 *   ever unwinds across this boundary).
 *
 * OPERATIONS (codetopo_query's `op` argument)
 *   describe_schema, stats, search_symbols, get_symbol, descendants,
 *   ancestors, blast_radius, path, neighborhood, diagnostics, rank,
 *   metrics, communities, topo_sort, render, history, diff, snapshot,
 *   verify. Indexing and opening are the dedicated entry points
 *   codetopo_index / codetopo_open_db / codetopo_load_snapshot below,
 *   not query ops. An unknown op is an "invalid_param" error, never a
 *   silent default.
 *
 *   Query arguments are a JSON object (`args_json`; NULL or "" means
 *   "{}"). Per op:
 *     describe_schema  {} — needs no graph data.
 *     stats            {}
 *     search_symbols   {"query": "...", "limit": 20}
 *     get_symbol       {"node": "<id>"}
 *     descendants      {"node": "<id>", "limit": 20, "expand": false}
 *     ancestors        {"node": "<id>", "limit": 20, "expand": false}
 *     blast_radius     {"node": "<id>", "limit": 20, "expand": false}
 *     path             {"from": "<id>", "to": "<id>"}
 *     neighborhood     {"node": "<id>", "depth": 1}
 *     diagnostics      {} — store-persisted diagnostics of a DB-backed
 *                        handle; a snapshot-backed handle answers an
 *                        empty list (snapshots carry no diagnostics).
 *     rank             {"by": "pagerank", "limit": 20} — `by` is the
 *                        ONLY metric parameter name; any other key is
 *                        an error naming the key.
 *     metrics          {}
 *     communities      {"limit": 20}
 *     topo_sort        {}
 *     render           {"layout": "layered" | "force"}
 *     history          {"repo_path": "...", "since": "...",
 *                        "min_support": 2} — repo_path optional; with
 *                        none, history answers measured:false + reason.
 *     diff             exactly one other source: {"other_db_path": ...}
 *                        or {"other_snapshot_json": "..."} or
 *                        {"other_snapshot_path": "..."}; the handle is
 *                        the "before" graph, the other source "after".
 *     snapshot         {} — the portable Snapshot JSON of the handle's
 *                        graph, bound to its audit head.
 *     verify           {} — the audit chain of a DB-backed handle, or
 *                        the audit log carried by a snapshot-backed one.
 */

#ifndef CODETOPO_H
#define CODETOPO_H

#ifdef __cplusplus
extern "C" {
#endif

/** Opaque handle to a frozen codetopo graph. */
typedef struct codetopo_graph codetopo_graph;

/** Version of the codetopo core this library was built against
 *  (e.g. "0.9.1"). Static string — do NOT free. */
const char *codetopo_version(void);

/** CODE_SCHEMA version this build speaks (e.g. "1.2"), taken from the
 *  core's SCHEMA_VERSION constant. Static string — do NOT free. */
const char *codetopo_schema_version(void);

/** Open the index database at `db_path` and hold its graph.
 *  Returns NULL + err_out ("unknown_index") when no database exists
 *  at the path. Caller frees the handle with codetopo_graph_free(). */
codetopo_graph *codetopo_open_db(const char *db_path, char **err_out);

/** Load a graph from a Snapshot JSON string (the portable form the
 *  `snapshot` op and every other surface produce). A schema major
 *  mismatch fails with "unsupported_schema_version".
 *  Caller frees the handle with codetopo_graph_free(). */
codetopo_graph *codetopo_load_snapshot(const char *snapshot_json, char **err_out);

/** Release a graph handle. NULL is a no-op. */
void codetopo_graph_free(codetopo_graph *graph);

/** Index the repository at `repo_path` into the database at
 *  `db_path` (created/replaced, in the CLI/store format every surface
 *  opens). `package` may be NULL, in which case the repository
 *  directory name is used, as on the other surfaces.
 *  Returns the index report as a JSON string
 *  ({"package", "files_indexed", "files_failed", "nodes", "edges",
 *  "diagnostics"}) — caller frees with codetopo_free_string(). */
char *codetopo_index(const char *repo_path, const char *db_path, const char *package, char **err_out);

/** Run one query operation (see the operation list above) against a
 *  graph handle. `args_json` is a JSON object string (NULL = "{}").
 *  Returns the answer as a JSON string — caller frees with
 *  codetopo_free_string(). NULL + err_out on failure. */
char *codetopo_query(const codetopo_graph *graph, const char *op, const char *args_json, char **err_out);

/** Release a string returned by this library (an answer or an error
 *  string). NULL is a no-op. Must NOT be called on the static strings
 *  from codetopo_version() / codetopo_schema_version(). */
void codetopo_free_string(char *s);

#ifdef __cplusplus
}
#endif

#endif /* CODETOPO_H */
