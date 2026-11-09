\name{codetopo}
\alias{codetopo}
\alias{codetopo_index}
\alias{codetopo_open}
\alias{codetopo_from_snapshot}
\alias{codetopo_restore}
\alias{codetopo_stats}
\alias{codetopo_search_symbols}
\alias{codetopo_get_symbol}
\alias{codetopo_descendants}
\alias{codetopo_ancestors}
\alias{codetopo_blast_radius}
\alias{codetopo_path}
\alias{codetopo_neighborhood}
\alias{codetopo_diagnostics}
\alias{codetopo_rank}
\alias{codetopo_metrics}
\alias{codetopo_communities}
\alias{codetopo_topo_sort}
\alias{codetopo_render}
\alias{codetopo_history}
\alias{codetopo_diff}
\alias{codetopo_snapshot}
\alias{codetopo_verify}
\alias{codetopo_describe_schema}
\alias{codetopo_core_version}
\alias{codetopo_schema_version}
\title{Code Topology Analysis from R}
\description{
An R surface over the codetopo Rust core (extendr). Index a repository
with \code{codetopo_index()} (or open an existing index with
\code{codetopo_open()}), then query the returned \code{codetopo_handle}
with the \code{codetopo_*} functions. All answers are CODE_SCHEMA JSON
produced by the same library the CLI, HTTP, and MCP surfaces serve.
}
\details{
Every function captures \code{...} and errors naming any unknown
argument. Rank's metric argument is \code{by} only. Absent
measurements stay absent, never coerced to 0. Errors carry the core's
stable codes in the condition's \code{code} field.
}
\examples{
\dontrun{
topo <- codetopo_index("path/to/repo")
codetopo_stats(topo)
codetopo_rank(topo, by = "pagerank", limit = 10)
}
}
