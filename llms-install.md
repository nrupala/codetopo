# codetopo MCP Server — Installation Guide for AI Agents

This file is written for AI coding agents (Cline, Cursor, Claude Code, and
other MCP clients) to install and configure the codetopo MCP server
automatically. Follow the steps in order.

## What this server does

codetopo turns a source repository into a queryable code graph — symbols,
calls, imports, and structure — and answers structural questions as JSON:
what calls this function, what breaks if it changes, how two modules
connect. The MCP server (`codetopo-mcp`) is a stdio JSON-RPC server
exposing 17 tools over the same engine as the codetopo CLI and HTTP API.
Indexes are local SQLite databases; nothing leaves the machine.

## Prerequisites

- A stable Rust toolchain — install via [rustup](https://rustup.rs) if
  `cargo --version` fails. The server is installed from source with
  Cargo; there is no prebuilt package yet.
- `git` on `PATH` (Cargo fetches the repository with it).
- No API key, account, or credentials are required.

## Installation

### 1. Install the server binary

```sh
cargo install --git https://github.com/nrupala/codetopo codetopo-mcp
```

The package must be named: the repository contains several binary
packages, and a bare install URL fails. This command installs only the
MCP server. (The project README's longer command additionally installs
the `codetopo` CLI, the `codetopo-server` HTTP API, and the
`codetopo-acp` agent.)

The first build compiles from source and can take several minutes. The
binary lands in `~/.cargo/bin/codetopo-mcp` — make sure `~/.cargo/bin`
is on the `PATH` your MCP client sees.

### 2. Configure the MCP client

Add the server to the client's MCP settings:

```json
{
  "mcpServers": {
    "codetopo": {
      "command": "codetopo-mcp"
    }
  }
}
```

That is the whole configuration. The server takes **no command-line
arguments, no environment variables, and no config file** by design —
an MCP client spawns it with this fixed command line, and every tool
argument (repository path, database path, node id) travels inside the
JSON-RPC calls.

## Verify the installation

1. Call `codetopo_describe_schema` — it needs no database and returns
   the node/edge kinds and query vocabulary. A structured answer means
   the server is up.
2. Call `codetopo_index` with the path of a repository to index (Rust
   and TypeScript/JavaScript are parsed via tree-sitter). It creates
   the graph database and reports files indexed, nodes, and edges.
3. Call `codetopo_stats` against that index — totals should match the
   index report.

## The 17 tools

| Tool | What it answers |
|---|---|
| `codetopo_index` | Index a repository into a codetopo graph database |
| `codetopo_search_symbols` | Find symbols by name — the discovery step before exact-id queries |
| `codetopo_get_symbol` | One node in full: kind, location, tier, defines and refs |
| `codetopo_descendants` | Nodes transitively reachable from a node over calls and imports |
| `codetopo_ancestors` | Nodes from which a node is transitively reachable |
| `codetopo_blast_radius` | What breaks if a node changes: dependents, overriders, implementers |
| `codetopo_path` | Shortest typed path between two nodes, or why there is none |
| `codetopo_neighborhood` | The subgraph within a few hops of a node, both directions |
| `codetopo_stats` | Node and edge totals for an index, with per-kind breakdowns |
| `codetopo_verify` | Verify the index's tamper-evident (hash-chained) audit log |
| `codetopo_snapshot` | Export the whole graph as a portable snapshot bound to the audit head |
| `codetopo_diagnostics` | The diagnostics recorded at index time — dependency cycles, multi-definitions, unresolved calls — as data |
| `codetopo_rank` | Top symbols by fan-in, fan-out, blast-radius size, or cyclomatic complexity — counts with the method stated |
| `codetopo_diff` | Compare two indexes or snapshots: added, removed, and changed nodes and edges, plus blast-radius movement |
| `codetopo_metrics` | Coupling and cohesion per type and package: CBO, RFC, LCOM, TCC, ATFD, WMC; Ca, Ce, instability |
| `codetopo_history` | Hotspots (change frequency × complexity) and change coupling from a local tree's git history; answers "not measured" without one |
| `codetopo_describe_schema` | The schema this server speaks; needs no database |

## Notes

- Aggregate-first output: tools return counts before full lists unless
  asked to expand, which keeps token budgets sane.
- MCP answers are identical to the HTTP API answers by construction —
  both call the same `codetopo-core` queries.
- License: AGPL-3.0-or-later. Repository:
  https://github.com/nrupala/codetopo — issues and questions go there.
