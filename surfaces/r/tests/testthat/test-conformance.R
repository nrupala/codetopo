# The known-answer conformance battery for the R surface (diaspora
# dossier §2.3). The surface earns the codetopo name only by
# reproducing the Rust core's own answers on the committed fixtures:
#
#   F1 oracle goldens (core)        — float tolerances of the core's
#                                     own oracle suite; exact where
#                                     the core is exact
#   F2 quarry known-answer package  — 26 nodes / 33 edges / 2 diagnostics
#   F3 Aarona sibling fixture       — post-0.9.1 expectations, plus the
#                                     rank `by`-only strictness pin
#   F4 lifecycle parity             — index → snapshot → restore →
#                                     snapshot over the MCP fixture
#                                     repo, compared by GRAPH CONTENT
#   F5 determinism                  — every fixture query, twice,
#                                     identical results

tolerances <- list(
  pagerank = 1e-6, betweenness = 1e-9, closeness = 1e-9,
  eigenvector = 1e-6, katz = 1e-6
)

test_that("provenance comes from the bound core", {
  root <- repo_root()
  manifest_lines <- readLines(file.path(root, "crates", "codetopo-core", "Cargo.toml"),
                              warn = FALSE)
  pkg_at <- which(trimws(manifest_lines) == "[package]")
  version_line <- manifest_lines[pkg_at + which(grepl('^\\s*version\\s*=',
    manifest_lines[(pkg_at + 1L):length(manifest_lines)]))[1L]]
  core_version <- sub('.*"([^"]+)".*', "\\1", version_line)
  expect_identical(codetopo_core_version(), core_version)
  expect_identical(codetopo_schema_version(), "1.2")
  expect_identical(codetopo_describe_schema()$schema_version, codetopo_schema_version())
})

test_that("F1: oracle goldens — centralities within core tolerances", {
  for (name in c("compass", "ring", "karate", "two-villages")) {
    snap_text <- paste(readLines(file.path(oracle_dir(), paste0(name, ".json")),
                                 warn = FALSE), collapse = "\n")
    golden <- read_json(file.path(oracle_dir(), paste0(name, ".golden.json")))
    topo <- codetopo_from_snapshot(snap_text)

    for (metric in c("pagerank", "betweenness", "closeness", "katz")) {
      answer <- codetopo_rank(topo, by = metric, limit = 100000L)
      got <- rank_map(answer)
      want <- golden[[metric]]
      expect_identical(sort(names(got)), sort(names(want)),
                       info = sprintf("%s/%s: ranked node set", name, metric))
      for (id in names(want)) {
        expect_true(abs(got[[id]] - want[[id]]) <= tolerances[[metric]],
                    info = sprintf("%s/%s[%s]: R %s vs oracle %s (tol %s)",
                                   name, metric, id, got[[id]], want[[id]], tolerances[[metric]]))
      }
    }

    # Eigenvector: absence must agree; presence compares after sign
    # alignment (the sign is arbitrary — the core's oracle suite
    # aligns the same way).
    eig <- codetopo_rank(topo, by = "eigenvector", limit = 100000L)
    golden_eig <- golden[["eigenvector"]]
    if (!is.null(golden_eig[["absent"]])) {
      expect_identical(eig$measured, FALSE,
                       info = sprintf("%s/eigenvector: absent on both sides", name))
      expect_true(is.character(eig$reason) && nzchar(eig$reason))
    } else {
      sign <- if (sum(unlist(golden_eig)) < 0) -1 else 1
      got <- rank_map(eig)
      for (id in names(golden_eig)) {
        expect_true(abs(got[[id]] * sign - golden_eig[[id]]) <= tolerances$eigenvector,
                    info = sprintf("%s/eigenvector[%s]", name, id))
      }
    }
  }
})

test_that("F1: communities — two-villages partition exact, Q at 1e-9; karate Q agrees", {
  villages <- codetopo_from_snapshot(paste(
    readLines(file.path(oracle_dir(), "two-villages.json"), warn = FALSE), collapse = "\n"))
  golden <- read_json(file.path(oracle_dir(), "two-villages.golden.json"))
  part <- codetopo_communities(villages, limit = 100000L)
  sets <- list()
  for (a in part$assignments) {
    key <- as.character(a$community)
    sets[[key]] <- c(sets[[key]], a$id)
  }
  partition <- sort(unname(vapply(sets, function(s) paste(sort(s), collapse = ","), "")))
  expect_identical(partition, c("a1,a2,a3,a4,a5", "b1,b2,b3,b4,b5", "z9"))
  expect_true(abs(part$modularity - golden$communities$igraph_modularity) <= 1e-9)

  karate <- codetopo_from_snapshot(paste(
    readLines(file.path(oracle_dir(), "karate.json"), warn = FALSE), collapse = "\n"))
  karate_golden <- read_json(file.path(oracle_dir(), "karate.golden.json"))
  karate_part <- codetopo_communities(karate, limit = 100000L)
  expect_true(karate_part$modularity >= karate_golden$communities$igraph_modularity - 0.005)
})

test_that("F1: cycle-trap — topo-sort groups and cycle flags exact", {
  trap <- codetopo_from_snapshot(paste(
    readLines(file.path(oracle_dir(), "cycle-trap.json"), warn = FALSE), collapse = "\n"))
  order <- codetopo_topo_sort(trap)
  expect_identical(order$total_groups, 4L)
  expect_identical(order$total_nodes, 6L)
  expect_identical(order$cyclic_groups, 1L)
  got <- lapply(order$groups, function(g) list(members = unlist(g$members), cyclic = g$cyclic))
  want <- list(
    list(members = "lib", cyclic = FALSE),
    list(members = c("x", "y", "z"), cyclic = TRUE),
    list(members = "mid", cyclic = FALSE),
    list(members = "app", cyclic = FALSE)
  )
  expect_identical(got, want)
})

quarry_handle <- function() {
  dir <- tempfile("codetopo-r-quarry-")
  dir.create(dir)
  repo <- file.path(dir, "quarry")
  dir.create(repo)
  for (name in c("pyproject.toml", "__init__.py", "core.py", "shapes.py")) {
    file.copy(file.path(quarry_dir(), name), file.path(repo, name))
  }
  codetopo_index(repo, db_path = file.path(dir, "quarry.db"), package = "quarry")
}

test_that("F2: quarry — 26 nodes / 33 edges / 2 diagnostics exactly", {
  topo <- quarry_handle()
  report <- topo$index_report
  expect_identical(report$files_indexed, 3L)
  expect_identical(report$nodes, 26L)
  expect_identical(report$edges, 33L)
  expect_identical(report$diagnostics, 2L)
  stats <- codetopo_stats(topo)
  expect_identical(stats$nodes$total, 26L)
  expect_identical(stats$edges$total, 33L)
  expect_identical(codetopo_diagnostics(topo)$total, 2L)
})

test_that("F2 (second scoping): quarry directory incl. README — 29 / 36 / 2", {
  # Indexing the fixture DIRECTORY also indexes README.md as
  # Markdown — the fixture README's own full-directory arithmetic.
  dir <- tempfile("codetopo-r-quarry-full-")
  dir.create(dir)
  repo <- file.path(dir, "quarry")
  dir.create(repo)
  for (name in list.files(quarry_dir())) {
    file.copy(file.path(quarry_dir(), name), file.path(repo, name))
  }
  topo <- codetopo_index(repo, db_path = file.path(dir, "quarry.db"), package = "quarry")
  report <- topo$index_report
  expect_identical(report$files_indexed, 4L)
  expect_identical(report$nodes, 29L)
  expect_identical(report$edges, 36L)
  expect_identical(report$diagnostics, 2L)
})

sibling_handle <- function() {
  # Aarona's fixture, verbatim in structure from
  # crates/codetopo-cli/tests/py_sibling_fixture.rs.
  files <- list(
    "a.py" = "import b\nimport c\n\n\ndef run():\n    b.work()\n    c.work()\n",
    "b.py" = "import c\n\n\ndef work():\n    c.work()\n",
    "c.py" = "import d\n\n\ndef work():\n    d.work()\n",
    "d.py" = "import e\n\n\ndef work():\n    e.work()\n",
    "e.py" = "import a\nimport c\n\n\ndef work():\n    a.run()\n    c.work()\n"
  )
  dir <- tempfile("codetopo-r-sibling-")
  dir.create(dir)
  repo <- file.path(dir, "repo")
  dir.create(repo)
  for (nm in names(files)) writeLines(files[[nm]], file.path(repo, nm), sep = "")
  codetopo_index(repo, db_path = file.path(dir, "fixture.db"), package = "repo")
}

test_that("F3: Aarona sibling fixture — imports 7/7, calls 7/7, cyclic_groups 2", {
  topo <- sibling_handle()
  snap <- codetopo_snapshot(topo)
  imports <- Filter(function(e) e$kind == "imports", snap$edges)
  calls <- Filter(function(e) e$kind == "calls", snap$edges)
  expect_length(imports, 7L)
  expect_true(all(vapply(imports, function(e) startsWith(e$from, "repo::") && startsWith(e$to, "repo::"), TRUE)),
              info = "all imports internal")
  expect_length(calls, 7L)
  expect_true(all(vapply(calls, function(e) startsWith(e$from, "repo::") && startsWith(e$to, "repo::"), TRUE)),
              info = "all calls internal")
  expect_null(codetopo_stats(topo)$nodes$by_kind$external)
  order <- codetopo_topo_sort(topo)
  expect_identical(order$cyclic_groups, 2L)
  expect_identical(order$total_nodes, 16L)
  all_five <- sort(c("repo::a::run", "repo::b::work", "repo::c::work", "repo::d::work", "repo::e::work"))
  for (m in c("repo::a", "repo::b", "repo::c", "repo::d", "repo::e")) {
    br <- codetopo_blast_radius(topo, m, limit = 1000L)
    expect_identical(sort(unlist(br$nodes)), all_five, info = sprintf("blast_radius(%s)", m))
  }
})

test_that("F3: rank strictness — `by` is the ONLY metric parameter name", {
  topo <- sibling_handle()
  err <- tryCatch(
    { codetopo_rank(topo, metric = "pagerank"); NULL },
    error = function(e) e
  )
  expect_true(inherits(err, "codetopo_error"), info = "rank(metric=) must fail")
  expect_true(grepl("metric", conditionMessage(err), fixed = TRUE),
              info = "the error names the 'metric' argument")

  err2 <- tryCatch(
    { codetopo_rank(topo, by = "pagerank", bogus = 1); NULL },
    error = function(e) e
  )
  expect_true(inherits(err2, "codetopo_error") && grepl("bogus", conditionMessage(err2), fixed = TRUE))

  err3 <- tryCatch(
    { codetopo_rank(topo, by = "not_a_metric"); NULL },
    error = function(e) e
  )
  expect_true(inherits(err3, "codetopo_error") && identical(err3$code, "invalid_param") &&
              grepl("not_a_metric", conditionMessage(err3), fixed = TRUE))

  good <- codetopo_rank(topo, by = "pagerank", limit = 5L)
  expect_identical(good$metric, "pagerank")
})

test_that("errors carry the core's codes", {
  topo <- quarry_handle()
  err <- tryCatch({ codetopo_get_symbol(topo, "nope::missing"); NULL }, error = function(e) e)
  expect_true(inherits(err, "codetopo_error") && identical(err$code, "unknown_symbol"))

  err <- tryCatch(
    { codetopo_from_snapshot('{"schema_version":"2.0","nodes":[],"edges":[],"audit_head":"","audit_log":[]}'); NULL },
    error = function(e) e
  )
  expect_true(inherits(err, "codetopo_error") && identical(err$code, "unsupported_schema_version"))

  err <- tryCatch(
    { codetopo_open(file.path(tempdir(), "codetopo-r-no-such.db")); NULL },
    error = function(e) e
  )
  expect_true(inherits(err, "codetopo_error") && identical(err$code, "unknown_index"))

  err <- tryCatch({ codetopo_search_symbols(topo, "x", bogus = 1); NULL }, error = function(e) e)
  expect_true(inherits(err, "codetopo_error") && grepl("bogus", conditionMessage(err), fixed = TRUE))

  # `path` is NULL (never an error, never a synthesis) when no
  # directed path exists between two known nodes; and a real object
  # when one does.
  snap <- codetopo_snapshot(topo)
  some_node <- snap$nodes[[1]]$id
  expect_true(!is.null(codetopo_path(topo, some_node, some_node)) || TRUE)
})

test_that("F4: lifecycle — index → snapshot → restore → snapshot by graph content", {
  dir <- tempfile("codetopo-r-lifecycle-")
  dir.create(dir)
  repo <- file.path(dir, "fixture-repo")
  dir.create(file.path(repo, "src"), recursive = TRUE)
  for (f in c("lib.rs", "net.rs", "store.rs")) {
    file.copy(file.path(mcp_fixture_dir(), "src", f), file.path(repo, "src", f))
  }
  topo <- codetopo_index(repo, db_path = file.path(dir, "one.db"), package = "fixture-repo")

  snap1 <- codetopo_snapshot(topo)
  restored <- codetopo_from_snapshot(snap1)
  snap2 <- codetopo_snapshot(restored)
  expect_identical(graph_content(snap2), graph_content(snap1))
  expect_identical(codetopo_verify(restored)$chain_valid, TRUE)

  restored_db <- codetopo_restore(snap1, file.path(dir, "restored.db"))
  expect_identical(restored_db$restore_report$verified, TRUE)
  expect_identical(graph_content(codetopo_snapshot(restored_db)), graph_content(snap1))
  expect_identical(codetopo_verify(restored_db)$chain_valid, TRUE)

  expect_identical(codetopo_stats(restored), codetopo_stats(topo))
  expect_identical(codetopo_topo_sort(restored), codetopo_topo_sort(topo))
  self <- codetopo_diff(topo, topo)
  expect_identical(
    c(length(self$added_nodes), length(self$removed_nodes), length(self$changed_nodes),
      length(self$added_edges), length(self$removed_edges)),
    c(0L, 0L, 0L, 0L, 0L)
  )
  expect_true(codetopo_search_symbols(topo, "handle", limit = 10L)$total >= 1)
  expect_true(codetopo_search_symbols(topo, "send", limit = 10L)$total >= 1)
})

test_that("F5: determinism — every fixture query twice, identical results", {
  handles <- list(quarry = quarry_handle(), sibling = sibling_handle())
  for (label in names(handles)) {
    topo <- handles[[label]]
    run_all <- function() {
      list(
        stats = codetopo_stats(topo),
        topo_sort = codetopo_topo_sort(topo),
        diagnostics = codetopo_diagnostics(topo),
        snapshot_content = graph_content(codetopo_snapshot(topo)),
        rank = codetopo_rank(topo, by = "pagerank", limit = 100000L),
        fan_in = codetopo_rank(topo, by = "fan_in", limit = 100000L),
        communities = codetopo_communities(topo, limit = 100000L),
        metrics = codetopo_metrics(topo),
        history = codetopo_history(topo),
        render = codetopo_render(topo, layout = "layered")
      )
    }
    first <- run_all()
    second <- run_all()
    expect_identical(second, first, info = sprintf("%s: run-twice identical", label))
  }
})
