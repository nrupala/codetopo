<!--
SPDX-FileCopyrightText: 2026 2351485 ALBERTA INC.

SPDX-License-Identifier: AGPL-3.0-or-later
-->

# english-ingest — the ENGLISH profile's ingest companion

Placement decision: the parser lives **outside** the engine, on
purpose. Codetopo's law is *the agent thinks, the engine measures*;
a statistical parser is neither — it is the instrument that
produces the english profile's ground floor, and it belongs in a
companion the same way the RA workbook extractor's precedents put
domain instruments at the repo's edge rather than in the core. The
repo has no Rust-side NLP stack and adding one to the workspace
would couple the engine's release to a model file; a pinned Python
tool under `tools/` keeps the boundary exact: the engine consumes
snapshots, the companion produces them, and neither pretends to be
the other.

## What it does

Reads a text or Markdown file and emits a Codetopo snapshot
(`profile_id: "english"`) holding the **ground floor only**:

- nodes: one `document`, one `sentence` per sentence, one
  `predicate` per clause-heading verb, one `entity` per noun chunk;
- edges: `contains` plus the grammar relations (`subject-of`,
  `object-of`, `complement-of`, `modifier-of`, `clause-of`,
  `appositive-of`, `coordinated-with`), each stamped with the
  parser identity in `method` and the exact spaCy dependency
  relation in `dep` where the mapping came from one.

Markdown furniture (headings, emphasis, links, list markers) is
stripped before parsing; wording is otherwise verbatim, and node
locations point at the source file's lines.

## What it never does

**It never emits a claim edge.** The english profile's claim kinds
(`same-entity`, `frames`, `supports`, `states-thesis-of`) do not
exist in the companion's mapping table — there is no code path that
can produce one, and a self-check refuses the output if that ever
changes. Claim edges are added afterwards by a reading agent, each
carrying the agent's name in `claimant` and its grounds in
`basis`. Interpretation smuggled in under a measurement's
authority is the one dishonest shape this profile exists to avoid.

## The pin is enforced

The profile's published accuracy (119/137 relations on the
conformance fixture; `docs/english-accuracy.md`) is a measurement
of **spaCy 3.8.18 / en_core_web_md 3.8.0**. The companion checks
both versions at startup and refuses under any other, naming what
it found. To change the parser: re-run the fixture audit by eye,
update the accuracy statement and the profile's face in the same
change, then change the pin.

## Use

```sh
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
.venv/bin/python -m spacy download en_core_web_md
.venv/bin/python english_ingest.py path/to/text.md --out snapshot.json
```

The snapshot restores through any Codetopo surface that loads
snapshots (`codetopo restore`, the server's snapshot pathway, the
core API). The graph it produces is claims-class: its grammar
edges are `inference`-class measurements with the accuracy above,
and it carries no interpretation at all until a reader adds some —
labelled.
