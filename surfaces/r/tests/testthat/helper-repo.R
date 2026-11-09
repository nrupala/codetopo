# Locate the codetopo repository root (the fixtures live in the repo,
# not in the installed package). Search order: the CODETOPO_REPO_ROOT
# environment variable (CI sets it), then upwards from the working
# directory and from this test file's directory.
repo_root <- function() {
  env <- Sys.getenv("CODETOPO_REPO_ROOT")
  candidates <- c(
    if (nzchar(env)) env,
    getwd(),
    testthat::test_path(),
    file.path(testthat::test_path(), ".."),
    file.path(testthat::test_path(), "..", "..")
  )
  for (cand in candidates) {
    path <- normalizePath(cand, mustWork = FALSE)
    for (i in 0:8) {
      if (file.exists(file.path(path, "CODE_SCHEMA.md")) &&
          dir.exists(file.path(path, "crates", "codetopo-core", "tests", "fixtures", "oracle"))) {
        return(path)
      }
      parent <- dirname(path)
      if (parent == path) break
      path <- parent
    }
  }
  testthat::skip("codetopo repository root not found (set CODETOPO_REPO_ROOT)")
}

oracle_dir <- function() file.path(repo_root(), "crates", "codetopo-core", "tests", "fixtures", "oracle")
quarry_dir <- function() file.path(repo_root(), "crates", "codetopo-extract", "tests", "fixture-py")
mcp_fixture_dir <- function() file.path(repo_root(), "crates", "codetopo-mcp", "tests", "fixture-repo")

read_json <- function(path) {
  jsonlite::fromJSON(paste(readLines(path, warn = FALSE), collapse = "\n"),
                     simplifyVector = FALSE)
}

# Graph content of a snapshot (nodes + edges) in canonical sorted
# form. Snapshot byte/content comparison beyond this is never done:
# snapshots embed wall-clock audit timestamps (the known lifecycle_py
# flake), so parity is asserted on graph content and the audit chain
# is checked through codetopo_verify().
graph_content <- function(snapshot) {
  nodes <- snapshot$nodes[order(vapply(snapshot$nodes, function(n) n$id, ""))]
  edge_keys <- vapply(snapshot$edges, function(e) {
    paste(e$from, e$to, e$kind, if (is.null(e$line)) "" else e$line, sep = "\r")
  }, "")
  edges <- snapshot$edges[order(edge_keys)]
  list(schema_version = snapshot$schema_version, nodes = nodes, edges = edges)
}

rank_map <- function(answer) {
  out <- list()
  for (entry in answer$results) out[[entry$id]] <- entry$value
  out
}
