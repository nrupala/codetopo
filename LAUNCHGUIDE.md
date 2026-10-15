# codetopo — Launch Guide

Structured metadata for MCP directories and marketplace submissions. Fields below mirror the published Official MCP Registry record for `io.github.nrupala/codetopo` and the project README.

## Tagline

Domain-blind code graphs for AI agents: symbols, calls, blast radius, proof-checked snapshots.

## Description

See your codebase the way an agent sees it. codetopo turns a source repository into a queryable code graph — symbols, calls, references, and structure — with JSON answers an AI coding agent can act on directly.

codetopo is a local stdio MCP server written in Rust. It exposes 12 tools: `codetopo_index`, `codetopo_descendants`, `codetopo_ancestors`, `codetopo_blast_radius`, `codetopo_path`, `codetopo_stats`, `codetopo_verify`, `codetopo_snapshot`, `codetopo_search_symbols`, `codetopo_get_symbol`, `codetopo_neighborhood`, and `codetopo_describe_schema`. Indexes stay on your machine; there are no accounts and no telemetry.

## Setup Requirements

- A stable Rust toolchain (install via [rustup](https://rustup.rs)).
- Install the MCP server from crates.io:

```sh
cargo install codetopo-mcp
```

- Client configuration: command `codetopo-mcp` over stdio — no arguments, no environment variables, no API key.

```json
{ "mcpServers": { "codetopo": { "command": "codetopo-mcp" } } }
```

## Category

Developer Tools

## Features

- Queryable code graph: symbols, calls, references, and structure, answered as JSON an agent can act on directly.
- Aggregate-first answers: counts first, full lists only when asked — keeps token budgets and pipes sane.
- Tamper-evident, hash-chained audit log on every index, verifiable with `codetopo_verify`.
- Proof-checked snapshots: portable frozen JSON exports of the whole graph.
- The same engine is also available as a CLI, an HTTP API (`codetopo-server`), and an ACP agent (`codetopo-acp`) alongside the MCP server.
- Live demo: [codetopo.aimlds.org](https://codetopo.aimlds.org) — codetopo's real map of its own repository, refreshed on every push to `main`.

## Getting Started

```sh
# Index a repository (Rust and TypeScript/JavaScript are parsed via tree-sitter)
codetopo index ~/my-project --db graph.db

# Find a symbol's id by name (every surface has this discovery step)
codetopo search connect --db graph.db

# Ask what calls what (the package prefix is your directory's name,
# exactly as indexed — hyphen included)
codetopo descendants my-project::net::Client::connect --db graph.db --expand
```

## Tags

code-graph, code-intelligence, static-analysis, rust, mcp

## Documentation URL

https://aimlds.org/codetopo

## Health Check URL

https://codetopo.aimlds.org

## Listing Metadata

| Field | Value |
|---|---|
| Name | codetopo |
| Registry name | io.github.nrupala/codetopo |
| Version | 0.5.0 |
| Repository | https://github.com/nrupala/codetopo |
| License | AGPL-3.0-or-later |
| Transport | stdio |
| Package | crates.io — codetopo-mcp |
