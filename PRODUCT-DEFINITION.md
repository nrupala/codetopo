<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# codetopo — product definition (delivery-system artifact)

**Filed:** 2026-11-10, at the v0.9.1 arming. This record formalises,
for the delivery system, the definition the product has carried since
its design study (`DESIGN.md`, decisions locked 2026-09-26) and its
public description (`README.md`). It changes no product behaviour.

## What it is

codetopo measures the structure of a codebase — symbols, calls,
imports, communities, cycles — and answers structural questions over
that measurement deterministically: closures, blast radius, shortest
paths, rankings, diffs, renders. It is served as a hosted API, a
local MCP server, a CLI, and language surfaces (C ABI, JavaScript,
R, Julia, C/C++). Its answers are measured data, not model opinion:
**the agent thinks, the engine measures.**

## Can / should / will-not

- **Can:** index a repository into a structural graph and answer the
  twenty chartered measurement questions over it, identically from
  every surface (one engine, one schema — CODE_SCHEMA).
- **Should:** be the structure layer other agents build on — the
  schema is an input for downstream systems, licensed so third-party
  agents can build against the contract (Apache-2.0 for the spec,
  SDKs and API/tool contract; AGPL-3.0 for the product).
- **Will-not:** call a language model (0.9.1's ask path is
  deterministic machinery, no model), hold card data (payments are
  processor-hosted), expose a tenant's index to another tenant, or
  administer keys over the network (key administration is a host
  subcommand by design).

## Buyers and price

Agent builders and engineering teams who need structural truth about
code they are changing. Hosted tiers, settled 2026-11-09: **Team
US$19/month per key** (25 active indexes, 10,000 queries/month,
50,000 files per index) and **Guardrails US$49/month per key**
(100 / 50,000 / 250,000). The local tools stay free and open source;
a hosted tier buys the operated service.

## Data management

- **Tenant data:** one database per index in the server's data dir,
  owned by exactly one key; deleted with its index; indexes may carry
  an operator-set lifetime (`CODETOPO_INDEX_TTL_SECS`).
- **Credentials:** the key store holds `SHA-256(salt ‖ secret)`
  hashes only; a secret is printed once, at issuance, by the host
  subcommand, and cannot be shown again.
- **Usage truth:** an append-only metering log per server; quota
  decisions read from it; `keys list` shows a tenant's position
  exactly as the server computes it.
- **Personal information:** the artifact itself holds no accounts
  and no card data; buyer identity lives with the payment processor
  (compliance set: privacy, payments-via-processor, accessibility).
