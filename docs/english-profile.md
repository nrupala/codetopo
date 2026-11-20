<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# The ENGLISH profile — grammar measured, meaning labelled

Codetopo's third profile (`profiles/english.profile.json`) graphs
English text. It is a **document profile, not a code language**:
it is not counted among the nineteen extracted code languages,
its graphs are **claims-class**, and nothing in it is ever
presented as truth-class. This page is the profile's contract.

## The two storeys

**Ground floor — measured grammar.** Inside a sentence, English
grammar is formal structure: the dependency parse names the
predicate (the verb), its subject, its objects, its complements
and modifiers — the noun / verb / action triad, extracted by a
named parser (spaCy 3.8.18 / en_core_web_md 3.8.0) by stated
mapping rules. Those edges are measurements, and they carry a
measured error: on dense technical prose the parser scores
**86.9%** of relations correct (119/137 on the conformance
fixture; flawless on the plain sentences of the same text). The
full audit — every flaw, clustered and named — is the profile's
published accuracy statement: [english-accuracy.md](english-accuracy.md).
Because a statistical parser's output is a measurement with an
error rate and not a rule-derived truth, grammar edges ride as
provenance class **inference**, each stamped with the parser
identity (`method`) and the exact spaCy relation it came from
(`dep`). The number travels with the label, or the label would
lie.

**Upper floor — labelled claims.** What a paragraph *means* —
which mentions are the same entity, which sentence states the
thesis, which sentences frame or support it — is not in the
grammar, and no mechanical source extracts it today (the parser
has no coreference at all). That layer enters an english graph
only as **declared_claim** edges (`same-entity`, `frames`,
`supports`, `states-thesis-of`), each carrying:

- `claimant` — the reading agent's name, and
- `basis` — the grounds, in words.

An interpretation edge without both is not a claim, it is a
smuggled assertion, and the profile's well-formedness rules
refuse it wherever edges are admitted. The label is not
decoration; it is the product's integrity. The day it comes off,
the product is lying.

## The pieces

- **The companion** (`tools/english-ingest`) produces the ground
  floor: text or Markdown in, an english snapshot out. It emits
  grammar edges only — the claim kinds do not exist in its
  mapping table — and it enforces the parser pin at startup,
  because the accuracy statement is a measurement *of that
  parser*. Claims are added afterwards, by a reader, labelled.
- **The engine** treats english graphs like any profile's:
  snapshots naming `profile_id: "english"` restore under the
  english dictionary; the query surface (`symbol`,
  `neighborhood`, `rank`, `path`, `stats`) answers over both
  storeys, with provenance class visible per edge kind.
- **`path` and identity.** `same-entity` is declared *symmetric*
  in the profile: identity points anaphorically (a later mention
  back to an earlier one), and a directed-only walk answered "no
  path" between two mentions of one entity — a connection that
  exists, hidden by direction. Path queries therefore traverse
  symmetric kinds both ways. Every other kind stays directed, in
  every profile: a path is still never synthesized by reversing
  an edge that asserts a direction. CODE declares no symmetric
  kinds; its paths are unchanged.
- **The render.** The structure scope of an english graph
  clusters each sentence's words in a band under their sentence
  (the profile declares `render.structure_cluster`), instead
  of the flat per-depth row that made the spike's renders
  ribbons. In the SVG, the two storeys read differently as they
  do in the data: grammar edges are solid, claim edges are
  dashed amber. A claim never borrows the measured storey's
  line.

## The graph-class words

English snapshots stamp `graph_class: "claims"`. The core's
generic section 4.3 computation names the same condition
"claims-bearing" when a graph is re-exported through
`Snapshot::from_graph` (its vocabulary is truths /
claims-bearing, computed from edge classes). Both words refuse
the same thing: an english graph contains inference and
declared-claim edges, so it is never — under either word —
truth-class.

## What this profile is not

- Not a code language, and not counted as one anywhere.
- Not a truth source: the ground floor is ~87% reliable on hard
  prose, and the upper floor is somebody's reading, labelled as
  such. Both numbers and both labels are the product.
- Not an understanding engine. The parser measures grammar the
  way the engine measures code; the understanding in the claim
  layer is the reader's. *The agent thinks, the engine
  measures* — English gives that law a second storey; it does
  not repeal it.

## Conformance

The known-answer fixture is `crates/codetopo-core/tests/
fixture-english/` (the paragraph, the two-storey snapshot, the
derivation README), proven by `crates/codetopo-core/tests/
english_profile_fixture.rs`: 75 nodes (1 document, 7 sentences,
22 predicates, 45 entities), 144 edges (132 grammar, 12 claims).
A change that moves a count moves the fixture, the profile's
conformance oracle, and the accuracy statement in the same PR.
