# codetopo

codetopo indexes a repository into a deterministic graph of its code -
packages, modules, files, classes, functions, calls, imports, and
reads/writes - and answers structural questions about it over MCP. It is
read-only by construction: indexed code is parsed, never executed.

## Prerequisite

This extension launches the `codetopo-mcp` binary over stdio, so install it
first and make sure it is on your PATH:

```sh
cargo install codetopo-mcp
```

Prebuilt binaries for Linux, macOS, and Windows are also attached to each
GitHub release: https://github.com/nrupala/codetopo/releases

## Using the tools

Start a session by indexing the repository you want to ask about with
`codetopo_index`, then query the graph:

- `codetopo_search_symbols`, `codetopo_get_symbol` - find and inspect symbols
- `codetopo_neighborhood` - the code around a symbol
- `codetopo_descendants`, `codetopo_ancestors` - dependency closures
- `codetopo_blast_radius` - what a change to a symbol would affect
- `codetopo_path` - how two symbols connect
- `codetopo_stats` - graph size and diagnostics
- `codetopo_snapshot`, `codetopo_verify` - proof-checked snapshots
- `codetopo_describe_schema` - the graph schema
