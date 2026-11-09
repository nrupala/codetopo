# codetopo — the R surface over the codetopo Rust core.
#
# A local library: it runs in your own R session, over your files, and
# makes no network calls. It is not a client of the hosted API and it
# is not an MCP server; it is the same Rust core the CLI, HTTP, and MCP
# surfaces serve, bound in process, answering with the same
# CODE_SCHEMA JSON by construction.
#
# Plain lists in, plain lists out. Every result is the CODE_SCHEMA
# JSON object the other surfaces return, parsed with
# jsonlite::fromJSON(..., simplifyVector = FALSE) — absent
# measurements stay absent (fields are simply not there), never
# coerced to 0.
#
# Strictness is deliberate (the 0.9.1 lesson): every wrapper captures
# `...` and an unknown argument is an error NAMING the argument —
# rank's metric parameter is `by` only, with no synonyms. The surface
# refuses to guess.
#
# The handle: `codetopo_index()` / `codetopo_open()` return an
# environment of class "codetopo_handle" holding the index database
# path (`db_path`); `codetopo_from_snapshot()` returns one holding the
# snapshot JSON text (`snapshot_json`) instead. The Rust layer is
# stateless and loads the frozen graph from those coordinates per
# call, so a handle is cheap to copy, save, and pass around.

# ---------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------

.ct_error <- function(code, message) {
  stop(structure(
    class = c("codetopo_error", "error", "condition"),
    list(message = message, call = NULL, code = code)
  ))
}

# The 0.9.1 discipline: any argument arriving through `...` is unknown
# by definition (every real parameter is named in the signature), and
# unknown arguments fail loudly, naming the argument.
.ct_check_dots <- function(dots, where) {
  if (length(dots) > 0L) {
    bad <- names(dots)
    label <- if (!is.null(bad) && nzchar(bad[[1L]])) bad[[1L]] else "<unnamed>"
    .ct_error(
      "invalid_param",
      sprintf(
        "%s: unknown argument '%s' (this surface refuses to guess; rank's metric argument is 'by' only)",
        where, label
      )
    )
  }
  invisible(NULL)
}

# Parse one native answer. Domain errors cross the boundary as a JSON
# envelope; they are re-raised as R errors carrying the core's code
# (in the `code` field of the condition and prefixed in the message).
.ct_parse <- function(text, where) {
  value <- jsonlite::fromJSON(text, simplifyVector = FALSE)
  err <- value[["__codetopo_error__"]]
  if (!is.null(err)) {
    .ct_error(err$code, sprintf("[%s] %s", err$code, err$message))
  }
  value
}

.ct_raw <- function(text, where) {
  # For provenance strings, which are not JSON answers.
  value <- tryCatch(
    jsonlite::fromJSON(text, simplifyVector = FALSE),
    error = function(e) NULL
  )
  # Only a JSON object can be an error envelope — a bare version
  # string like "1.2" parses as an atomic number and is just itself.
  if (is.list(value) && !is.null(value[["__codetopo_error__"]])) {
    err <- value[["__codetopo_error__"]]
    .ct_error(err$code, sprintf("[%s] %s", err$code, err$message))
  }
  text
}

.ct_new_handle <- function(db_path = NULL, snapshot_json = NULL,
                           index_report = NULL, restore_report = NULL) {
  handle <- new.env(parent = emptyenv())
  handle$db_path <- db_path
  handle$snapshot_json <- snapshot_json
  handle$index_report <- index_report
  handle$restore_report <- restore_report
  class(handle) <- "codetopo_handle"
  handle
}

.ct_coords <- function(handle, where) {
  if (!inherits(handle, "codetopo_handle")) {
    .ct_error(
      "invalid_param",
      sprintf("%s: 'handle' must be a codetopo_handle from codetopo_index(), codetopo_open(), codetopo_from_snapshot(), or codetopo_restore()", where)
    )
  }
  list(
    db = if (is.null(handle$db_path)) "" else handle$db_path,
    snap = if (is.null(handle$snapshot_json)) "" else handle$snapshot_json
  )
}

.ct_limit <- function(limit, where) {
  if (length(limit) != 1L || !is.numeric(limit) || is.na(limit) ||
      limit < 0 || limit != floor(limit)) {
    .ct_error("invalid_param", sprintf("%s: 'limit' must be a non-negative integer", where))
  }
  as.integer(limit)
}

.ct_snapshot_text <- function(snapshot, where) {
  if (is.character(snapshot) && length(snapshot) == 1L) {
    return(snapshot)
  }
  if (is.list(snapshot)) {
    return(jsonlite::toJSON(
      snapshot,
      auto_unbox = TRUE, digits = NA, null = "null", na = "null"
    ))
  }
  .ct_error("missing_param", sprintf("%s: a snapshot (list or JSON string) is required", where))
}

#' @export
print.codetopo_handle <- function(x, ...) {
  if (!is.null(x$db_path)) {
    cat(sprintf("<codetopo_handle> db: %s\n", x$db_path))
  } else {
    cat("<codetopo_handle> snapshot-born\n")
  }
  invisible(x)
}

# ---------------------------------------------------------------------
# Provenance + schema
# ---------------------------------------------------------------------

#' The version of the bound codetopo-core crate.
#'
#' Read from the core crate's own manifest at build time — provenance,
#' not a restated literal.
#' @export
codetopo_core_version <- function(...) {
  .ct_check_dots(list(...), "codetopo_core_version")
  .ct_raw(ct_core_version(), "codetopo_core_version")
}

#' The CODE_SCHEMA version the bound core emits ("1.2").
#' @export
codetopo_schema_version <- function(...) {
  .ct_check_dots(list(...), "codetopo_schema_version")
  .ct_raw(ct_schema_version(), "codetopo_schema_version")
}

#' The schema description every codetopo surface serves.
#' Needs no database.
#' @export
codetopo_describe_schema <- function(...) {
  .ct_check_dots(list(...), "codetopo_describe_schema")
  .ct_parse(ct_describe_schema(), "codetopo_describe_schema")
}

# ---------------------------------------------------------------------
# Lifecycle
# ---------------------------------------------------------------------

#' Index a repository into a queryable code graph.
#'
#' Writes a SQLite index in the shared CLI/store format, so the CLI,
#' MCP, HTTP, and R surfaces open each other's indexes.
#'
#' @param repo_path Local repository path.
#' @param db_path Index database path; defaults to
#'   `file.path(repo_path, "codetopo.db")`.
#' @param package Fallback package name; defaults to the repository
#'   directory's name, as on the other surfaces.
#' @return A `codetopo_handle`; its `index_report` element carries
#'   `files_indexed`, `files_failed`, `nodes`, `edges`, `diagnostics`.
#' @export
codetopo_index <- function(repo_path, db_path = NULL, package = NULL, ...) {
  .ct_check_dots(list(...), "codetopo_index")
  if (!is.character(repo_path) || length(repo_path) != 1L || !nzchar(repo_path)) {
    .ct_error("missing_param", "codetopo_index: a repository path string is required")
  }
  if (is.null(db_path)) db_path <- file.path(repo_path, "codetopo.db")
  report <- .ct_parse(
    ct_index(repo_path, db_path, if (is.null(package)) "" else package),
    "codetopo_index"
  )
  .ct_new_handle(db_path = db_path, index_report = report)
}

#' Open an existing index database (written by any codetopo surface).
#' @param db_path Path to the index database file.
#' @return A `codetopo_handle`.
#' @export
codetopo_open <- function(db_path, ...) {
  .ct_check_dots(list(...), "codetopo_open")
  if (!is.character(db_path) || length(db_path) != 1L || !nzchar(db_path)) {
    .ct_error("missing_param", "codetopo_open: a database path string is required")
  }
  if (!file.exists(db_path)) {
    .ct_error("unknown_index", sprintf("[unknown_index] unknown index: no database file at '%s'", db_path))
  }
  handle <- .ct_new_handle(db_path = db_path)
  # Probe: opening loads the graph, as on the other surfaces, so a
  # corrupt or foreign database fails here, not at first query.
  invisible(codetopo_stats(handle))
  handle
}

#' Build a handle from a snapshot (a list, as returned by
#' `codetopo_snapshot()`, or its JSON string form).
#'
#' A snapshot whose schema major differs from this build's fails with
#' code `unsupported_schema_version` (CODE_SCHEMA §8).
#' @export
codetopo_from_snapshot <- function(snapshot, ...) {
  .ct_check_dots(list(...), "codetopo_from_snapshot")
  text <- .ct_snapshot_text(snapshot, "codetopo_from_snapshot")
  handle <- .ct_new_handle(snapshot_json = text)
  invisible(codetopo_stats(handle)) # validates parse + schema major
  handle
}

#' Restore a snapshot into a database at `db_path`, through the full
#' verification gate (schema major + audit chain), then open it.
#' @return A `codetopo_handle`; its `restore_report` element carries
#'   `nodes`, `edges`, `verified`.
#' @export
codetopo_restore <- function(snapshot, db_path, ...) {
  .ct_check_dots(list(...), "codetopo_restore")
  text <- .ct_snapshot_text(snapshot, "codetopo_restore")
  report <- .ct_parse(ct_restore(text, db_path), "codetopo_restore")
  handle <- codetopo_open(db_path)
  handle$restore_report <- report
  handle
}

# ---------------------------------------------------------------------
# Queries
# ---------------------------------------------------------------------

#' Aggregate-first statistics: totals plus per-kind breakdowns.
#' @export
codetopo_stats <- function(handle, ...) {
  .ct_check_dots(list(...), "codetopo_stats")
  co <- .ct_coords(handle, "codetopo_stats")
  .ct_parse(ct_stats(co$db, co$snap), "codetopo_stats")
}

#' Find symbols by name fragment (case-insensitive) — the discovery
#' step before the exact-id queries.
#' @export
codetopo_search_symbols <- function(handle, query, limit = 20L, ...) {
  .ct_check_dots(list(...), "codetopo_search_symbols")
  co <- .ct_coords(handle, "codetopo_search_symbols")
  .ct_parse(
    ct_search_symbols(co$db, co$snap, as.character(query), .ct_limit(limit, "codetopo_search_symbols")),
    "codetopo_search_symbols"
  )
}

#' One symbol in full: node + incident edges + measurements where
#' measured.
#' @export
codetopo_get_symbol <- function(handle, id, ...) {
  .ct_check_dots(list(...), "codetopo_get_symbol")
  co <- .ct_coords(handle, "codetopo_get_symbol")
  .ct_parse(ct_get_symbol(co$db, co$snap, as.character(id)), "codetopo_get_symbol")
}

.ct_closure_call <- function(handle, relation, id, limit, expand, where, dots) {
  .ct_check_dots(dots, where)
  co <- .ct_coords(handle, where)
  .ct_parse(
    ct_closure(co$db, co$snap, relation, as.character(id), .ct_limit(limit, where), isTRUE(expand)),
    where
  )
}

#' Transitive closure over calls + imports: what `id` depends on.
#' @export
codetopo_descendants <- function(handle, id, limit = 20L, expand = FALSE, ...) {
  .ct_closure_call(handle, "descendants", id, limit, expand, "codetopo_descendants", list(...))
}

#' The reverse closure: what depends on `id`.
#' @export
codetopo_ancestors <- function(handle, id, limit = 20L, expand = FALSE, ...) {
  .ct_closure_call(handle, "ancestors", id, limit, expand, "codetopo_ancestors", list(...))
}

#' Transitive dependents over the breakage-propagating edge kinds.
#' @export
codetopo_blast_radius <- function(handle, id, limit = 20L, expand = FALSE, ...) {
  .ct_closure_call(handle, "blast_radius", id, limit, expand, "codetopo_blast_radius", list(...))
}

#' The shortest directed typed path between two symbols, or `NULL`
#' when no directed path exists — never a reversed-edge synthesis.
#' @export
codetopo_path <- function(handle, from, to, ...) {
  .ct_check_dots(list(...), "codetopo_path")
  co <- .ct_coords(handle, "codetopo_path")
  text <- ct_path(co$db, co$snap, as.character(from), as.character(to))
  if (identical(text, "null")) {
    # Still surface a coded envelope if one came back instead.
    return(NULL)
  }
  .ct_parse(text, "codetopo_path")
}

#' The k-hop subgraph around a symbol, both directions.
#' @export
codetopo_neighborhood <- function(handle, id, depth = 1L, ...) {
  .ct_check_dots(list(...), "codetopo_neighborhood")
  co <- .ct_coords(handle, "codetopo_neighborhood")
  .ct_parse(
    ct_neighborhood(co$db, co$snap, as.character(id), .ct_limit(depth, "codetopo_neighborhood")),
    "codetopo_neighborhood"
  )
}

#' The diagnostics recorded when this index was built, as data.
#' @export
codetopo_diagnostics <- function(handle, ...) {
  .ct_check_dots(list(...), "codetopo_diagnostics")
  co <- .ct_coords(handle, "codetopo_diagnostics")
  .ct_parse(ct_diagnostics(co$db, co$snap), "codetopo_diagnostics")
}

#' Rank symbols by a measurement.
#'
#' The metric parameter is `by` ONLY; any other name (including
#' `metric`) is an error naming the argument. A measure that does not
#' converge on this graph answers `measured: FALSE` with its reason —
#' an absence, never a partial result.
#'
#' @param by One of `fan_in`, `fan_out`, `blast_radius`, `complexity`,
#'   `pagerank`, `betweenness`, `closeness`, `eigenvector`, `katz`.
#' @export
codetopo_rank <- function(handle, by = "blast_radius", limit = 20L, ...) {
  .ct_check_dots(list(...), "codetopo_rank")
  co <- .ct_coords(handle, "codetopo_rank")
  .ct_parse(
    ct_rank(co$db, co$snap, as.character(by), .ct_limit(limit, "codetopo_rank")),
    "codetopo_rank"
  )
}

#' Coupling and cohesion measurements for every type and package,
#' each metric's method stated beside the numbers.
#' @export
codetopo_metrics <- function(handle, ...) {
  .ct_check_dots(list(...), "codetopo_metrics")
  co <- .ct_coords(handle, "codetopo_metrics")
  .ct_parse(ct_metrics(co$db, co$snap), "codetopo_metrics")
}

#' The Leiden partition of the dependency graph (resolution 1.0, the
#' published fixed seed), with modularity Q.
#' @export
codetopo_communities <- function(handle, limit = 20L, ...) {
  .ct_check_dots(list(...), "codetopo_communities")
  co <- .ct_coords(handle, "codetopo_communities")
  .ct_parse(
    ct_communities(co$db, co$snap, .ct_limit(limit, "codetopo_communities")),
    "codetopo_communities"
  )
}

#' The safe change order: SCC-condensed topo-sort; a multi-node group
#' is a cycle, flagged `cyclic: TRUE`, never disguised as an order.
#' @export
codetopo_topo_sort <- function(handle, ...) {
  .ct_check_dots(list(...), "codetopo_topo_sort")
  co <- .ct_coords(handle, "codetopo_topo_sort")
  .ct_parse(ct_topo_sort(co$db, co$snap), "codetopo_topo_sort")
}

#' The layout spec (`layered` v1 / `force` v2) as JSON data, from
#' which SVG or a plot derives.
#' @export
codetopo_render <- function(handle, layout = "layered", ...) {
  .ct_check_dots(list(...), "codetopo_render")
  co <- .ct_coords(handle, "codetopo_render")
  .ct_parse(ct_render(co$db, co$snap, as.character(layout)), "codetopo_render")
}

#' Change-history measurements (hotspots, change coupling) from a
#' local working tree's git history. With no `repo_path` the answer
#' is the unmeasured report with its reason.
#' @export
codetopo_history <- function(handle, repo_path = NULL, since = NULL, min_support = 2L, ...) {
  .ct_check_dots(list(...), "codetopo_history")
  co <- .ct_coords(handle, "codetopo_history")
  .ct_parse(
    ct_history(
      co$db, co$snap,
      if (is.null(repo_path)) "" else as.character(repo_path),
      if (is.null(since)) "" else as.character(since),
      .ct_limit(min_support, "codetopo_history")
    ),
    "codetopo_history"
  )
}

#' The change-impact diff against another handle.
#' @export
codetopo_diff <- function(handle, other, ...) {
  .ct_check_dots(list(...), "codetopo_diff")
  co <- .ct_coords(handle, "codetopo_diff")
  co2 <- .ct_coords(other, "codetopo_diff")
  .ct_parse(ct_diff(co$db, co$snap, co2$db, co2$snap), "codetopo_diff")
}

#' The portable frozen form of the graph: the snapshot, as a list.
#' @export
codetopo_snapshot <- function(handle, ...) {
  .ct_check_dots(list(...), "codetopo_snapshot")
  co <- .ct_coords(handle, "codetopo_snapshot")
  .ct_parse(ct_snapshot(co$db, co$snap), "codetopo_snapshot")
}

#' Audit-chain verification: entry count, validity, head hash.
#' @export
codetopo_verify <- function(handle, ...) {
  .ct_check_dots(list(...), "codetopo_verify")
  co <- .ct_coords(handle, "codetopo_verify")
  .ct_parse(ct_verify(co$db, co$snap), "codetopo_verify")
}
