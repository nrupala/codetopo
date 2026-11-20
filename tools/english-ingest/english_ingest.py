#!/usr/bin/env python3
# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
#
"""english_ingest — the Codetopo ENGLISH profile's ingest companion.

Reads a text or Markdown file, parses it with the pinned dependency
parser (spaCy 3.8.18 / en_core_web_md 3.8.0), and emits a Codetopo
snapshot (profile_id "english") holding the GROUND FLOOR ONLY:
document / sentence / predicate / entity nodes and grammar edges
(contains, subject-of, object-of, complement-of, modifier-of,
clause-of, appositive-of, coordinated-with), every edge stamped
with the parser identity in its `method` attr and the exact spaCy
relation in its `dep` attr where the mapping came from one.

THE COMPANION NEVER EMITS A CLAIM EDGE. The claim kinds of the
english profile (same-entity, frames, supports, states-thesis-of)
do not exist in this file's mapping table: there is no code path
here that can produce one. Claim edges are added afterwards by a
reading agent, in the snapshot or through the surfaces, each
carrying that agent's name in `claimant` and its grounds in
`basis`. The two storeys are separate because they are different
kinds of knowledge: the parser measures grammar (at the measured
accuracy printed on the profile's face — docs/english-accuracy.md);
a reader interprets meaning. Merging the two in one tool would
smuggle interpretation in under the measurement's authority.

The parser pin is enforced, not advisory: the accuracy statement
on the profile's face is a measurement OF THIS PARSER on the
conformance fixture. Running under any other parser or model
version would emit graphs whose `method` stamps lie about the
measurement behind them, so this tool refuses and names the
versions it found.

Usage:
    python3 english_ingest.py INPUT.md [--out SNAPSHOT.json]
                              [--doc-id ID] [--doc-label LABEL]

Output defaults to stdout. Exit codes: 0 ok, 2 refusal (wrong
parser, unreadable input, self-check failure).
"""

import argparse
import json
import re
import sys

SPACY_PIN = "3.8.18"
MODEL_NAME = "en_core_web_md"
MODEL_PIN = "3.8.0"
METHOD = f"spacy {SPACY_PIN} / {MODEL_NAME} {MODEL_PIN}"

# The grammar kind table — the ONLY edge kinds this tool can emit.
# Claim kinds are absent by construction (see the module docstring).
GRAMMAR_KINDS = {
    "contains",
    "subject-of",
    "object-of",
    "complement-of",
    "modifier-of",
    "clause-of",
    "appositive-of",
    "coordinated-with",
}
CLAIM_KINDS = {"same-entity", "frames", "supports", "states-thesis-of"}

CLAUSE_DEPS = {"relcl", "acl", "advcl", "ccomp", "xcomp", "parataxis", "conj"}


def refuse(msg):
    print(f"english_ingest: refusal: {msg}", file=sys.stderr)
    sys.exit(2)


def load_parser():
    try:
        import spacy
    except ImportError:
        refuse("spaCy is not installed (see requirements.txt in this directory)")
    if spacy.__version__ != SPACY_PIN:
        refuse(
            f"spaCy {spacy.__version__} found; the english profile's accuracy "
            f"statement is measured on spaCy {SPACY_PIN}. Install the pin "
            f"(requirements.txt) or re-measure the fixture before changing it."
        )
    try:
        nlp = spacy.load(MODEL_NAME)
    except OSError:
        refuse(
            f"model {MODEL_NAME} is not installed "
            f"(python3 -m spacy download {MODEL_NAME})"
        )
    found = nlp.meta.get("version")
    if found != MODEL_PIN:
        refuse(
            f"{MODEL_NAME} {found} found; the accuracy statement is measured "
            f"on {MODEL_PIN}. Install the pin or re-measure before changing it."
        )
    return nlp


def clean_markdown(text):
    """Strip Markdown furniture, keeping a per-line map.

    Emphasis and code markers, heading/list/quote markers and link
    syntax are removed; wording is otherwise verbatim (the spike's
    treatment of its source). Returns (cleaned_text, line_starts)
    where line_starts[i] is the char offset of source line i+1.
    """
    out_lines = []
    for line in text.split("\n"):
        line = re.sub(r"^\s{0,3}#{1,6}\s+", "", line)
        line = re.sub(r"^\s*>\s?", "", line)
        line = re.sub(r"^\s*[-*+]\s+", "", line)
        line = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", line)
        line = line.replace("`", "").replace("*", "").replace("_", " ")
        out_lines.append(line)
    cleaned = "\n".join(out_lines)
    starts = []
    off = 0
    for line in out_lines:
        starts.append(off)
        off += len(line) + 1
    return cleaned, starts


def line_of(offset, starts):
    line = 1
    for i, s in enumerate(starts):
        if s <= offset:
            line = i + 1
        else:
            break
    return line


def segment_and_parse(nlp, cleaned, starts):
    """Segment the document into sentences, then parse each sentence
    on its own (the spike's method — parsing is sentence-local, and
    per-sentence parses are what the fixture was audited against).
    Returns a list of sentence dicts in the spike's parse shape."""
    doc = nlp(cleaned)
    sentences = []
    for n, sent in enumerate(doc.sents, 1):
        text = sent.text.strip()
        if not text:
            continue
        line = line_of(sent.start_char + (len(sent.text) - len(sent.text.lstrip())), starts)
        sdoc = nlp(text)
        toks = [
            {
                "i": t.i,
                "text": t.text,
                "lemma": t.lemma_,
                "pos": t.pos_,
                "dep": t.dep_,
                "head": t.head.i,
            }
            for t in sdoc
        ]
        chunks = [
            {
                "text": c.text,
                "root": c.root.text,
                "root_dep": c.root.dep_,
                "start": c.start,
                "end": c.end,
            }
            for c in sdoc.noun_chunks
        ]
        sentences.append({"n": len(sentences) + 1, "text": text, "line": line,
                          "tokens": toks, "noun_chunks": chunks})
    return sentences


def build_graph(sentences, doc_id, doc_label, source_name):
    nodes, edges = [], []

    def node(nid, kind, label, line, tier, attrs=None):
        nodes.append({
            "id": nid, "kind": kind, "label": label,
            "loc": {"file": source_name, "line": line},
            "tier": tier, "defines": [], "refs": [], "attrs": attrs or {},
        })

    def edge(frm, to, kind, **attrs):
        a = {"method": METHOD}
        a.update(attrs)
        edges.append({"from": frm, "to": to, "kind": kind, "attrs": a})

    node(doc_id, "document", doc_label, 1, "core")

    pred_of_token = {}   # (sent_n, token_i) -> predicate node id
    ent_of_chunk = {}    # (sent_n, chunk_idx) -> entity node id

    for s in sentences:
        n = s["n"]
        sid = f"s{n}"
        node(sid, "sentence", f"S{n}", s["line"], "core")
        edge(doc_id, sid, "contains")
        # predicates: every VERB, plus AUX that heads a clause (ROOT
        # or has nsubj/attr dependents — the copula in "what's the
        # shortest path")
        aux_heads = {t["head"] for t in s["tokens"] if t["dep"] in ("nsubj", "attr")}
        for t in s["tokens"]:
            if t["pos"] == "VERB" or (t["pos"] == "AUX" and t["i"] in aux_heads):
                pid = f"s{n}:v{t['i']}"
                neg = any(
                    c["dep"] == "neg" and c["head"] == t["i"] for c in s["tokens"]
                )
                pred_of_token[(n, t["i"])] = pid
                node(pid, "predicate", t["text"] + (" (negated)" if neg else ""),
                     s["line"], "optional", {"verb": t["lemma"], "negated": neg})
                edge(sid, pid, "contains")
        # entities: noun chunks
        for ci, c in enumerate(s["noun_chunks"]):
            eid = f"s{n}:e{ci}"
            ent_of_chunk[(n, ci)] = eid
            node(eid, "entity", c["text"], s["line"], "optional",
                 {"chunk_root": c["root"], "root_dep": c["root_dep"]})
            edge(sid, eid, "contains")

    def chunk_of_root(n, tok_i):
        for (sn, ci), eid in ent_of_chunk.items():
            if sn == n:
                c = sentences[n - 1]["noun_chunks"][ci]
                for t in sentences[n - 1]["tokens"]:
                    if t["i"] == tok_i and c["start"] <= t["i"] < c["end"]:
                        return eid
        return None

    for s in sentences:
        n = s["n"]
        toks = {t["i"]: t for t in s["tokens"]}
        for ci, c in enumerate(s["noun_chunks"]):
            eid = ent_of_chunk[(n, ci)]
            root_tok = None
            for t in s["tokens"]:
                if c["start"] <= t["i"] < c["end"] and not (
                    c["start"] <= t["head"] < c["end"]
                ):
                    root_tok = t
                    break
            if root_tok is None:
                continue
            dep, head = root_tok["dep"], root_tok["head"]
            head_pred = pred_of_token.get((n, head))
            head_ent = chunk_of_root(n, head)
            if dep in ("nsubj", "nsubjpass") and head_pred:
                edge(eid, head_pred, "subject-of", dep=dep)
            elif dep == "dobj" and head_pred:
                edge(eid, head_pred, "object-of", dep=dep)
            elif dep == "attr" and head_pred:
                edge(eid, head_pred, "complement-of", dep=dep)
            elif dep == "pobj":
                # object of a preposition; the prep's head is verb or noun
                prep = toks[head]
                gp = pred_of_token.get((n, prep["head"]))
                ge = chunk_of_root(n, prep["head"])
                if gp:
                    edge(eid, gp, "object-of", dep=dep, via=prep["text"])
                elif ge:
                    edge(eid, ge, "modifier-of", dep=dep, via=prep["text"])
            elif dep == "appos" and head_ent:
                edge(eid, head_ent, "appositive-of", dep=dep)
            elif dep == "conj":
                if head_ent:
                    edge(eid, head_ent, "coordinated-with", dep=dep)
                elif head_pred:
                    edge(eid, head_pred, "object-of", dep=dep,
                         note="conjunct object")
            elif dep == "nmod" and head_ent:
                edge(eid, head_ent, "modifier-of", dep=dep)
        # verb-verb clause structure
        for t in s["tokens"]:
            pid = pred_of_token.get((n, t["i"]))
            if not pid or t["dep"] == "ROOT":
                continue
            hp = pred_of_token.get((n, t["head"]))
            he = chunk_of_root(n, t["head"])
            if t["dep"] == "conj" and hp:
                edge(pid, hp, "coordinated-with", dep="conj")
            elif t["dep"] in CLAUSE_DEPS and hp:
                edge(pid, hp, "clause-of", dep=t["dep"])
            elif t["dep"] in ("relcl", "acl") and he:
                edge(pid, he, "clause-of", dep=t["dep"])

    return nodes, edges


def main():
    ap = argparse.ArgumentParser(description="Codetopo ENGLISH ingest companion")
    ap.add_argument("input", help="text or Markdown file to parse")
    ap.add_argument("--out", help="snapshot output path (default: stdout)")
    ap.add_argument("--doc-id", default="doc", help="document node id")
    ap.add_argument("--doc-label", default=None, help="document node label")
    args = ap.parse_args()

    try:
        with open(args.input, encoding="utf-8") as f:
            raw = f.read()
    except OSError as e:
        refuse(f"cannot read {args.input}: {e}")

    nlp = load_parser()
    cleaned, starts = clean_markdown(raw)
    sentences = segment_and_parse(nlp, cleaned, starts)
    if not sentences:
        refuse("no sentences found in the input")

    import os
    source_name = os.path.basename(args.input)
    label = args.doc_label or f"{source_name} ({len(sentences)} sentences)"
    nodes, edges = build_graph(sentences, args.doc_id, label, source_name)

    # Self-check: the companion's whole honesty claim rests on this —
    # no emitted edge may be a claim kind, and every emitted kind
    # must be a declared grammar kind of the english profile.
    for e in edges:
        if e["kind"] in CLAIM_KINDS or e["kind"] not in GRAMMAR_KINDS:
            refuse(
                f"self-check failed: companion produced a {e['kind']} edge — "
                f"claim edges are a reading agent's to add, never this tool's"
            )

    snap = {
        "schema_version": "2.0",
        "profile_id": "english",
        "profile_version": "1.0.0",
        "graph_class": "claims",
        "provenance_summary": {"inference": len(edges)},
        "nodes": nodes,
        "edges": edges,
        "audit_head": "",
        "audit_log": [],
        "diagnostics": [],
    }
    text = json.dumps(snap, indent=1)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(text + "\n")
    else:
        print(text)
    kinds = {}
    for nd in nodes:
        kinds[nd["kind"]] = kinds.get(nd["kind"], 0) + 1
    print(
        f"english_ingest: {len(nodes)} nodes {kinds}, {len(edges)} grammar "
        f"edges, 0 claim edges (claims are added by a reading agent, "
        f"labelled — never by this tool)",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
