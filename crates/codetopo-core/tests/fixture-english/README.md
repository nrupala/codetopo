<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# fixture-english — the ENGLISH profile's known-answer fixture

Every expectation in `crates/codetopo-core/tests/
english_profile_fixture.rs` is derived here. A change that moves
a count must move this README, the snapshot, the profile's
conformance oracle, and `docs/english-accuracy.md` in the same
PR — that is the point of a known-answer fixture.

## Source

`paragraph.md` is DESIGN.md section 1 ("Problem / gap") of this
repository, seven sentences, Markdown emphasis stripped,
wording otherwise verbatim (the same text the 2026-11-19 English
spike audited). It was chosen for density: coordination,
appositives, a verbless fragment (S5), pronouns and
cross-sentence coreference (agent S2→S3, edges S4→S6).

## The ground floor (companion output)

`paragraph.snapshot.json`'s ground floor is the verbatim output
of the pinned companion (`tools/english-ingest`, spaCy 3.8.18 /
en_core_web_md 3.8.0) on `paragraph.md`, run with
`--doc-id para --doc-label "DESIGN.md S1 - Problem/gap
(7 sentences)"`, verified triple-for-triple against the spike's
audited graph (every node id, every edge (from, to, kind), every
`dep` attr, every node attr identical; node kinds renamed to the
english dictionary: package→document, file→sentence,
module→predicate, class→entity).

Nodes — 75:

| kind | count | derivation |
|---|---|---|
| document | 1 | the paragraph |
| sentence | 7 | S1–S7 |
| predicate | 22 | every VERB token, plus clause-heading AUX (the copula) |
| entity | 45 | every noun chunk |

Grammar edges — 132, all provenance class `inference`:

| kind | count |
|---|---|
| contains | 74 (1→7 sentences, 7→67 words) |
| subject-of | 13 |
| object-of | 12 |
| clause-of | 12 |
| coordinated-with | 7 |
| appositive-of | 6 |
| complement-of | 4 |
| modifier-of | 4 |

## The upper floor (the reading agent's claims)

Twelve `declared_claim` edges, authored by the spike's reading
agent and carried over unchanged except for the product form:
each carries `claimant: "english-spike-2026-11-19 reading agent"`
and its `basis` verbatim from the spike.

| kind | count | what it claims |
|---|---|---|
| same-entity | 5 | the agent S3→S2 · "it" S2→the agent · "it" S4→codebase-mcp · the edges S6→S4's missing edges · "symbols" S4→"symbols" S4 |
| frames | 3 | S1, S2, S3 each frame the problem S6 answers |
| supports | 3 | S4 and S7 are evidence for S6; S5 elaborates S4 |
| states-thesis-of | 1 | S6 states the paragraph's thesis |

## Totals

**75 nodes · 144 edges · 132 grammar (inference) / 12 claims
(declared_claim).** Accuracy of the ground floor on this text,
measured by eye against the sentences as written: 119/137
relations correct (86.9%) — `docs/english-accuracy.md` is the
full statement, including the three flaw clusters
(parentheticals with hyphenated names; colon/dash clause lists;
coordination labelling) and the S4 inversion the graph faithfully
inherits.
