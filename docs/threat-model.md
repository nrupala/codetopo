<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# codetopo — threat model

**Version:** v0.9.1 (release commit `5d5bfaf4`)
**Written:** 2026-11-10, at the v0.9.1 arming, by the gate lane.
**Method:** derived from the product's actual code and from the
Gate 17 certification pack's probed reality
(`~/workspace/release-gate-runs/codetopo-v091-2026-11-10/` — live probe
transcripts in `evidence/probe-output.txt` / `probe-output2.txt`).
This file names the model; the agent-mode THREAT-MODEL gate check
grades its quality at each full gate run.

## What the product is, for threat purposes

A deterministic code-structure measurement engine served three ways:
a hosted HTTP API (`codetopo-server`, bearer-key auth, per-key
tenancy and quotas), a local MCP server (`codetopo-mcp`, stdio, on
the caller's own machine), and local CLI/library surfaces. Buyers pay
through Stripe-hosted links; card data never touches this product.
The API's answers are measured graph data — 0.9.1's ask path contains
no language model.

## Assets

- **Hosted API keys** — the only credential the API knows. The store
  (`keys.db`) holds `SHA-256(salt ‖ secret)` hashes, never secrets.
- **Tenant index data** — one database per index in the data dir;
  a tenant's repository structure is their confidential material.
- **The operator's host** — the server process, its data dir, and
  key administration (a CLI subcommand) all live behind the host's
  shell boundary.
- **Metering integrity** — the append-only metering log is the
  quota truth; corrupting it corrupts billing fairness.
- **The served pages** (`/`, `/ask`) — the public front door; the
  ask page handles a visitor's pasted key in the browser.

## Trust boundaries

1. **Internet → server.** Everything crosses at the one auth wrapper
   (`auth.rs`). Unauthenticated surface is exactly two pages: `/`
   and `/ask`, public by design. There is no admin endpoint, no
   health endpoint, no registration or OAuth surface.
2. **Tenant ↔ tenant.** Enforced in-process by the tenancy check on
   every graph route: an index belongs to the key that created it.
3. **Server → outside world.** The server's only egress is the
   ingest git clone — the product's declared function. Nothing else
   in the server initiates network connections.
4. **Archive bytes → filesystem.** Uploaded archives are extracted
   under a per-ingest root; entries are untrusted input.
5. **Host shell → key store.** Key administration is a subcommand,
   not an endpoint: minting a key requires shell access to the host,
   which is already the trust boundary for the data dir itself
   (module docs, `crates/codetopo-server/src/main.rs`).
6. **Caller machine → MCP binary.** The MCP surface is stdio on the
   caller's machine; its trust boundary is that machine, not ours.
7. **Buyer → Stripe.** Payment happens entirely on Stripe-hosted
   pages (payments-via-processor).

## Use cases beside their abuse cases

| # | Use case | Abuse case | The product's answer (evidence) |
|---|----------|-----------|----------------------------------|
| U1 | A keyholder calls `/v1` routes with their key | Stolen or guessed key; credential stuffing against the wrapper | Secrets are hashed at rest (`SHA-256(salt ‖ secret)`), compared in constant time, with a dummy-hash burn for unknown ids so timing does not enumerate (`keys.rs`, read at the gate). No-key and wrong-key both answer `401 {"error":"unauthenticated"}` — identical body, no enumeration signal (probed live). |
| U2 | A keyholder reads their own index | Key A reads key B's index (IDOR / cross-tenant) | Tenancy check per route; a cross-tenant read answers `404 unknown_index` — no existence leak (probed live). A traversal-shaped id also answers 404. |
| U3 | A tenant indexes a repository by URL | SSRF: clone URL aimed at internal/metadata targets; credentials embedded in the URL | The clone is the server's only egress, https-only, and embedded credentials are refused (`ingest.rs`, read in full at the gate). Residual: redirect/rewrite behaviour of the underlying git client is not separately probed — named, not hidden. |
| U4 | A tenant uploads an archive of their code | Zip-slip: entries like `../escape.txt` write outside the extraction root | Entry paths that escape the root are refused `400` before extraction (probed live: refused, no file escaped). |
| U5 | A hosted tenant names a repo path | Tenant reaches the host filesystem by naming `repo_path` | `repo_path` is refused to hosted keys ("available only to self-hosted keys") — naming host paths is the operator's own power (probed live). |
| U6 | A keyholder works within their plan | Quota exhaustion / noisy neighbour: one key burns the service for all | Per-key metering from an append-only log; the monthly query quota refuses `429 query_quota_exceeded` naming plan, limit and month (proven live at the 10,000 hosted bound; a second key was unaffected). File/index caps (50,000 / 250,000 per index; 25 / 100 active) are enforced pre-ingest, before work is spent (`limits.rs`, `handlers.rs`). |
| U7 | Anyone calls the API | Oversize or malformed bodies; unknown parameters silently reinterpreted | A 5 MB body answers 413; unknown parameters on any `/v1` route answer `400 invalid_param` naming the declared set — nothing is silently normalised (probed live: `rank?metric=` refused, `rank?by=` served). Internal failures render a fixed generic client message; operator detail goes to stderr only (`error.rs`). |
| U8 | A visitor uses the `/ask` page | Prompt injection / a model talked into leaking or acting | There is no model in the 0.9.1 ask path: routing is the deterministic ACP intent classifier, answers are measured data plus a templated summary, and out-of-scope questions (weather, edit requests) are refused in plain words (probed live; `ask.rs` carries no HTTP client and no model reference). The page's script escapes all rendered values (`esc()`) before inserting returned data into the DOM. |
| U9 | An operator administers keys | Remote attacker mints or revokes keys | No network path exists: `keys issue|rotate|revoke|list` is a host-shell subcommand against the same store (boundary 5). `keys list` never prints a secret or a hash. |
| U10 | A buyer subscribes via a payment link | Card data exposure; a lookalike checkout | Checkout is Stripe-hosted via the two live links published in `README.md`; this product never receives card data (compliance set: payments-via-processor). Descriptor/live-mode truth is the agent-mode PAY-DESCRIPTOR check at the gate. |
| U11 | An agent drives the MCP binary locally | A hostile repository or tool argument abuses the caller's machine | The MCP tools are thin adapters over the same engine/store code as the API; the catalogue (`schemas/mcp-tools.json`) is extracted from the live binary. The binary runs with the caller's own privileges on the caller's machine — stated plainly so no one mistakes it for a hosted sandbox. |
| U12 | Anyone fetches the public pages | The front door enumerates the protected surface | The pages describe the API and its auth in plain words (that is their job) but expose no key material, no index data, and no route that answers without a key. The unauthenticated inventory is exactly the two pages (adjudicated at the gate). |

## Named gaps (not modelled away)

- **TLS and network posture of the production deployment** are the
  hosting site's, not this artifact's; the Kali-bench leg that would
  grade them was NOT RUN at the 0.9.1 gate and is owed at the RC
  deploy act (certification pack, NOT RUN item 4).
- **Git-clone redirect behaviour** (U3 residual) has no dedicated
  probe in this version's gate evidence.
- **Rate shape:** the product bounds usage per key per month, not
  per minute; a single key making expensive-but-in-quota requests is
  bounded by the 30 s hosted query timeout and the plan caps, and
  abuse inside those bounds is an operator/metering matter.
- **The ask page** takes the visitor's key into a form field in
  their own browser and sends it only to this API — the key is never
  stored by the page; a shared-machine browser remains the
  visitor's own boundary.
