<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# codetopo — failure and abuse analysis (delivery-system artifact)

**Filed:** 2026-11-10, at the v0.9.1 arming, from the Gate 17
certification pack's probed reality
(`~/workspace/release-gate-runs/codetopo-v091-2026-11-10/`). The
attacker-side view lives in `docs/threat-model.md`; this file is the
failure-side view: how the product breaks, what happens when it does,
and what it must never do quietly.

## Failure modes

| # | Failure | Behaviour | Evidence / where it is owned |
|---|---------|-----------|------------------------------|
| F1 | Bad bind address, unusable key store, data dir that cannot be created | **Fail-fast at startup** — the process refuses to boot rather than serve an API that cannot authenticate (`main.rs` startup doctrine; bad-bind case is unit-tested). | `crates/codetopo-server/src/main.rs`, `config.rs` |
| F2 | Internal error while serving | The client receives a fixed generic message; operator detail goes to stderr only — no internals in the error body. | `error.rs`, read in full at the gate |
| F3 | Unknown or extra request parameters | `400 invalid_param` naming the offending key and the declared set — the standing /rank lesson: a parameter is never silently normalised into another. Generalised across every `/v1` route in 0.9.1. | Probed live at the gate (`rank?metric=`, `stats?bogus=1`) |
| F4 | Oversize request body | 413 refusal before handler work. (Known cosmetic issue F-2 of the certification: the 413 body is the framework's plain text, not the API's flat JSON shape — status correct.) | Probed live at the gate |
| F5 | Wrong method on a known route | 405. | Probed live at the gate |
| F6 | Quota exhausted | `429 query_quota_exceeded` naming plan, limit and month; other keys unaffected. The refusal count reconciles exactly with the metering log. | Proven live at the gate with a seeded log |
| F7 | Plan caps exceeded (indexes, files) | Refused pre-ingest with 429 — before work is spent, never after a partial index. | `handlers.rs` pre-ingest checks; pinned by the `limits`/`tenancy` suites |
| F8 | Archive or tree that cannot be indexed | The ingest fails as an error, not a partial graph: traversal entries refused 400; an unusable repository path answers `invalid_repo_path`. | Probed live at the gate (traversal fixture) |
| F9 | A question the ask path cannot answer | Refused in plain words, with the answerable questions named — never answered by guessing. Out-of-scope classes (weather, edit requests) probed at the gate. | `ask.rs`; probe transcripts in the pack |
| F10 | Key store holds only a legacy flat file at startup | Its entries are imported when the store holds no active key, so an existing deployment keeps working — a designed migration, not a silent drop. | `config.rs` / `keys.rs` bootstrap |
| F11 | Hosted server unreachable / not started | Callers see connection failure; the server never half-serves: the listener binds only after config, key store and tenancy are prepared. | `main.rs` `run()` ordering |

## Abuse shapes (summary — full treatment in the threat model)

Guessed or stolen keys (hashed store, constant-time compare,
identical 401s) · cross-tenant reads (404, no existence leak) ·
clone-URL SSRF and embedded credentials (https-only, credentials
refused; the clone is the only egress) · archive path traversal
(refused pre-extraction) · hosted keys naming host paths (refused;
self-hosted keys only) · quota/noisy-neighbour abuse (per-key
metering and caps) · parameter smuggling (flat 400) · prompt
injection against the ask path (no model exists to inject into).

## The never-quietly list

A failure that must always be loud, never absorbed: serving without
a usable key store (F1) · answering a question the engine did not
measure (F9) · accepting a parameter the contract does not declare
(F3) · writing outside the extraction root (threat model U4) ·
exposing one tenant's index to another (U2) · printing a secret
after issuance (key administration prints once, list never).
