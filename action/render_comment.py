#!/usr/bin/env python3
# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Render a codetopo GraphDiff (`codetopo diff --json`) as a pull-request
comment a reviewer can read in ten seconds.

Design rules (aggregate first, measurements only, no opinions):
- The headline states the counts and the single largest blast-radius move.
- At most --max-rows symbols are listed, worst |delta| first, then by id
  so two runs over the same diff render byte-identically.
- Every number carries its meaning: "blast radius" = the count of symbols
  reachable from this one over calls+imports edges (codetopo descendants).
- No quality judgements. codetopo measures structure; reviewers decide.

Optional guardrail: --fail-above N exits 2 when any touched symbol's blast
radius after the change exceeds N (the Guardrails tier's first rule).

Usage: render_comment.py diff.json [--max-rows 10] [--fail-above N]
                                    [--repo-url URL] [--sha SHA]
"""

from __future__ import annotations

import argparse
import json
import sys

MARKER = "<!-- codetopo-pr-impact -->"


def _label(node: dict) -> str:
    loc = node.get("loc") or {}
    where = f"{loc.get('file', '?')}:{loc.get('line', '?')}"
    return f"`{node.get('label') or node.get('id')}` ({node.get('kind', '?')}, {where})"


def render(diff: dict, *, max_rows: int = 10, repo_url: str = "", sha: str = "") -> str:
    added = diff.get("added_nodes", [])
    removed = diff.get("removed_nodes", [])
    changed = diff.get("changed_nodes", [])
    added_e = diff.get("added_edges", [])
    removed_e = diff.get("removed_edges", [])
    impact = diff.get("impact", [])

    by_id: dict[str, dict] = {}
    for n in added:
        by_id[n["id"]] = n
    for n in removed:
        by_id.setdefault(n["id"], n)
    for c in changed:
        by_id[c["id"]] = c.get("after") or c.get("before") or {"id": c["id"]}
    status = {n["id"]: "added" for n in added}
    status.update({n["id"]: "removed" for n in removed})
    status.update({c["id"]: "changed" for c in changed})

    rows = sorted(impact, key=lambda e: (-abs(int(e.get("delta", 0))), e.get("id", "")))
    lines = [MARKER, "### codetopo — change impact", ""]

    if not (added or removed or changed or added_e or removed_e):
        lines.append("No structural change: no symbols or call/import edges were added, removed or changed.")
    else:
        lines.append(
            f"**{len(added)}** symbols added · **{len(removed)}** removed · **{len(changed)}** changed · "
            f"**{len(added_e)}** edges added · **{len(removed_e)}** removed"
        )
        if rows:
            top = rows[0]
            sign = "+" if int(top.get("delta", 0)) >= 0 else ""
            lines.append(
                f"Largest blast-radius move: {_label(by_id.get(top['id'], {'id': top['id']}))} "
                f"{top.get('blast_radius_before', 0)} → {top.get('blast_radius_after', 0)} "
                f"({sign}{top.get('delta', 0)})."
            )
        lines += ["", "| Symbol | Change | Blast radius before → after | Δ |", "|---|---|---|---|"]
        for e in rows[:max_rows]:
            d = int(e.get("delta", 0))
            lines.append(
                f"| {_label(by_id.get(e['id'], {'id': e['id']}))} | {status.get(e['id'], 'touched')} | "
                f"{e.get('blast_radius_before', 0)} → {e.get('blast_radius_after', 0)} | {'+' if d >= 0 else ''}{d} |"
            )
        if len(rows) > max_rows:
            lines.append(f"\n…and {len(rows) - max_rows} more touched symbols (full JSON in the workflow artifact).")

    lines += [
        "",
        (
            "<sub>Blast radius = symbols reachable from this one over calls + imports edges. "
            "Measured structure only, no quality judgement. "
            f"{'Head ' + sha[:12] + ' · ' if sha else ''}"
            "[codetopo](https://github.com/nrupala/codetopo) by AIMLDS — free for public repos.</sub>"
        ),
    ]
    return "\n".join(lines) + "\n"


def max_blast_after(diff: dict) -> int:
    return max((int(e.get("blast_radius_after", 0)) for e in diff.get("impact", [])), default=0)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("diff_json")
    ap.add_argument("--max-rows", type=int, default=10)
    ap.add_argument("--fail-above", type=int, default=0, help="guardrail: exit 2 if any blast radius after > N")
    ap.add_argument("--repo-url", default="")
    ap.add_argument("--sha", default="")
    a = ap.parse_args(argv)
    with open(a.diff_json, encoding="utf-8") as f:
        diff = json.load(f)
    sys.stdout.write(render(diff, max_rows=a.max_rows, repo_url=a.repo_url, sha=a.sha))
    if a.fail_above and max_blast_after(diff) > a.fail_above:
        sys.stderr.write(f"guardrail: blast radius {max_blast_after(diff)} exceeds limit {a.fail_above}\n")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
