<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ENGLISH profile — accuracy statement

This is the english profile's published accuracy statement. It is
a conformance artifact: the profile's face
(`profiles/english.profile.json`) quotes its headline figure, the
fixture test (`crates/codetopo-core/tests/english_profile_fixture.rs`)
pins the graph it was measured on, and any change to the parser
pin or the fixture re-measures and updates this statement in the
same PR. A change that moves a count moves this statement — that
is the rule, not a courtesy.

**Parser:** spaCy 3.8.18, model en_core_web_md 3.8.0 (pinned; the
companion in `tools/english-ingest` refuses to run under any other
versions). **Method:** every non-punctuation dependency relation
in the parse of the fixture paragraph (137) was checked by eye
against the sentence as written. Borderline relations that
preserve the true connectivity under a loose label were counted
correct and are named below — the count is not padded either way.

**Fixture:** DESIGN.md section 1 of this repository, seven
sentences chosen for density (`crates/codetopo-core/tests/
fixture-english/paragraph.md`). First measured 2026-11-19
(ENGLISH-SPIKE-2026-11-19), when the paragraph was graphed by hand
from this parser's output; the product fixture's ground floor is
the companion's own output on the same text, verified
triple-for-triple against that audited graph.

## Score by sentence

| Sentence | Relations | Wrong | Accuracy | Character |
|---|---|---|---|---|
| S1 | 20 | 2 | 90.0% | main clause perfect; the colon list slips |
| S2 | 35 | 5 | 85.7% | main clause perfect; the colon list of questions collapses |
| S3 | 12 | 1 | 91.7% | one coordination-chain convention |
| S4 | 22 | 7 | **68.2%** | catastrophic: subject destroyed, main verb inverted |
| S5 | 8 | 2 | 75.0% | verbless fragment: list items labelled appositive, not coordinated |
| S6 | 8 | 0 | **100%** | the thesis sentence parses flawlessly |
| S7 | 32 | 1 | 96.9% | parenthetical handled; the arrow chain's labels are mush, its connectivity intact |
| **Total** | **137** | **18** | **86.9%** | |

## Where the flaws sit — three clusters, no others

1. **Parentheticals + hyphenated names (S4, 7 of the 18).**
   "(Apache-2.0)" after "codebase-mcp" breaks the sentence: the
   name is shattered, the licence string becomes the head of the
   subject phrase, and "proved" — the main verb — is demoted to
   an adjective on "agents", leaving "want" as ROOT. The graph
   inherits this whole: S4's subject entity node is literally the
   chunk "codebase-mcp (Apache-2.0) proved agents". One
   punctuation habit of technical writing (name + parenthetical
   licence) is enough to invert a sentence's meaning in the graph.
2. **Colon/dash lists of clauses (S2, 4 of its 5; S1, both of
   its 2).** When a sentence enumerates — "answers: what calls
   this, what breaks…", "eyes: diagrams to glance at, trees to
   browse, waterfalls to scan" — the parser attaches list members
   to their neighbours instead of to the list's head. The main
   clauses on both sides of the list are perfect; the list itself
   is where structure is lost.
3. **Coordination labelling (S3, S5; S4 partly).** Coordinated
   items are chained (each conjunct attached to the previous) or
   mislabelled appositive. Meaning is recoverable by closure, but
   the relation as stated is wrong — and a graph that stores the
   stated relation stores the wrong one.

## Borderline, counted correct, named here

- S7 "typed entity schema": "typed" is read as a verb taking
  "schema" as its object; the attachment is right, the framing is
  not.
- S7's pipeline "schema → surface → aggregation" is folded into
  a tree whose labels are unreliable while the walkable chain
  survives intact.
- S1 "existing" is POS-tagged VERB; its relation (modifier of
  "tool") is right.

## What the parser does not even attempt

- **Coreference: zero mechanical help.** en_core_web_md has no
  coreference component. Every cross-sentence identity in an
  english graph is a reading agent's `same-entity` claim edge —
  in the fixture, 12 claim edges, 0 of them parser-derived.
- **The fragment S5 has no verb.** The parser crowns the first
  noun as ROOT, which is a reasonable storage decision and a
  false statement about the sentence: nothing in S5 predicates
  anything. A graph that doesn't mark "this sentence has no
  predicate" presents a fragment as a claim with a subject — the
  essence layer must carry that mark, and today nothing produces
  it mechanically.

## The honest summary

"Without flaws" would be 137/137. Measured: **119/137 (86.9%)**,
and the 18 flaws are not random noise — they concentrate exactly
where technical English does its real work (names with
parentheticals, enumerated lists). On plain sentences the parser
is at or near flawless (S6 100%, S3 91.7%, S7 96.9%); on the
estate's actual writing style it is not. That is why the
profile's grammar edges ride as class **inference**, never
extracted truth, why this number is printed on the profile's
face, and why no english graph is ever truth-class: a RULE edge
from this parser is a measurement that is ~87% reliable on this
kind of text, and the label stays honest only while the number
travels with it.
