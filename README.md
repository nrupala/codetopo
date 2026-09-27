# codetopo

Code structure as queryable data, built for agents first — a Rust library that
turns source repositories into graph representations agents can explore, query,
and reason over, with JSON as the universal export pathway.

**Status: Phase 1 (graph core) in progress.** Workspace scaffold and toolchain
only; crate implementations are being built by the Phase 1 workstreams.

## Crate layout

| Crate | Purpose |
|---|---|
| `codetopo-core` | Graph model: nodes, edges, traversal, serialization |
| `codetopo-extract` | Language extraction via tree-sitter (Rust, TypeScript) |
| `codetopo-store` | SQLite persistence + tamper-evident audit log (trust primitive) |
| `codetopo-cli` | `codetopo` CLI: scan, build, and query code graphs |

## Build

Requires a stable Rust toolchain (install via rustup):

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --workspace
```

## License

Core is **AGPL-3.0-or-later** (see `LICENSES/AGPL-3.0.txt`).
The code schema spec (`CODE_SCHEMA.md`) is **Apache-2.0** (see
`LICENSES/Apache-2.0.txt`).

Owned by Nrupal Akolkar · Built with Muse by Meta.
