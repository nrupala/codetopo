<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# codetopo — JavaScript surface

The codetopo Rust core, in-process for Node.js: index a repository
into a queryable code graph, then search, trace, rank, and render it —
deterministically, with every answer the same CODE_SCHEMA JSON the
CLI, HTTP, and MCP surfaces return (parity by construction: this
package is a transport over the core's one computation, not a second
implementation).

This is a **local native library**. It runs in your own Node process,
over your files, and makes no network calls. It is not a client of the
hosted API (no key, no plan, no quotas) and it is not an MCP server.
Indexes it writes are in the shared CLI/store format, so every surface
opens every other surface's indexes.

```js
const { Codetopo, CORE_VERSION, SCHEMA_VERSION } = require('codetopo');

const topo = await Codetopo.index('./my-repo');       // or Codetopo.open(dbPath)
console.log(topo.indexReport);                        // files, nodes, edges, diagnostics
console.log(topo.stats());
const hub = topo.searchSymbols('handle').results[0];
console.log(topo.blastRadius(hub.id));
console.log(await topo.rank({ by: 'pagerank', limit: 10 }));
console.log(topo.topoSort());                         // cycles flagged, never disguised
console.log(topo.snapshot());                         // portable CODE_SCHEMA JSON
console.log(topo.verify());                           // audit chain: {entries, chain_valid, head}
```

## The surface

The twenty operations of the product, camelCase in JS, wire
vocabulary unchanged: `index`, `open` / `fromSnapshot` / `restore`
(the handle lifecycle), `describeSchema`, `stats`, `searchSymbols`,
`getSymbol`, `descendants`, `ancestors`, `blastRadius`, `path`
(`null` when no directed path exists — never a reversed-edge
synthesis), `neighborhood`, `diagnostics`, `rank`, `metrics`,
`communities`, `topoSort`, `render`, `history`, `diff`, `snapshot`,
`verify`.

Heavy operations (`index`, `rank`, `metrics`, `communities`,
`history`) return Promises and run off the event loop. Absent
measurements stay absent — a metric that does not converge answers
`{measured: false, reason}`, never a partial result or a zero.

Strictness is a feature: option objects are validated — an unknown
option key throws an `Error` **naming the key** — and rank's metric
parameter is `by` only, with no synonyms. Errors are `Error` objects
with a stable `code` (`unknown_symbol`,
`unsupported_schema_version`, `invalid_param`, `unknown_index`, …).

`CORE_VERSION` and `SCHEMA_VERSION` are exported from the bound core
itself, so a consumer can state exactly what answered.

## Conformance

The surface earns the codetopo name by reproducing the Rust core's
own answers on the committed fixture battery (oracle goldens, the
quarry known-answer package, the Aarona sibling fixture at post-0.9.1
expectations, lifecycle parity by graph content, determinism):

```sh
npm test   # builds the addon (release) and runs tests/conformance.js
```

## Building

```sh
npm install --ignore-scripts   # @napi-rs/cli (devDependency)
npm run build                  # napi build --platform --release
```

Node ≥ 18. Licence: AGPL-3.0-or-later (the core implementation's
licence; the CODE_SCHEMA specification itself is Apache-2.0).
