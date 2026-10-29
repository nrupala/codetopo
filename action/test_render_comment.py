# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Tests for the PR-impact comment renderer (pure Python, no codetopo build needed)."""

import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).parent
sys.path.insert(0, str(HERE))

from render_comment import MARKER, max_blast_after, render


def node(i, kind="function", file="src/lib.rs", line=1):
    return {"id": i, "kind": kind, "label": i.split("::")[-1], "loc": {"file": file, "line": line}}


DIFF = {
    "added_nodes": [node("pkg::new_fn", line=10)],
    "removed_nodes": [node("pkg::old_fn", line=20)],
    "changed_nodes": [{"id": "pkg::core", "before": node("pkg::core", line=5), "after": node("pkg::core", line=6)}],
    "added_edges": [{"from": "pkg::new_fn", "to": "pkg::core", "kind": "calls"}],
    "removed_edges": [],
    "impact": [
        {"id": "pkg::new_fn", "blast_radius_before": 0, "blast_radius_after": 3, "delta": 3},
        {"id": "pkg::core", "blast_radius_before": 12, "blast_radius_after": 40, "delta": 28},
        {"id": "pkg::old_fn", "blast_radius_before": 2, "blast_radius_after": 0, "delta": -2},
    ],
}


def test_headline_and_worst_first():
    out = render(DIFF)
    assert out.startswith(MARKER)
    assert "**1** symbols added · **1** removed · **1** changed · **1** edges added" in out
    assert "Largest blast-radius move: `core`" in out and "12 → 40 (+28)" in out
    rows = [ln for ln in out.splitlines() if ln.startswith("| `")]
    assert [r.split("`")[1] for r in rows] == ["core", "new_fn", "old_fn"]


def test_deterministic():
    assert render(DIFF) == render(json.loads(json.dumps(DIFF)))


def test_no_change():
    empty = {k: [] for k in DIFF}
    assert "No structural change" in render(empty)


def test_max_rows_truncates():
    out = render(DIFF, max_rows=1)
    assert "…and 2 more touched symbols" in out


def test_guardrail_exit_code(tmp_path):
    p = tmp_path / "d.json"
    p.write_text(json.dumps(DIFF))
    ok = subprocess.run([sys.executable, str(HERE / "render_comment.py"), str(p), "--fail-above", "50"], check=False)
    bad = subprocess.run([sys.executable, str(HERE / "render_comment.py"), str(p), "--fail-above", "39"],
                         capture_output=True, text=True, check=False)
    assert ok.returncode == 0 and bad.returncode == 2 and max_blast_after(DIFF) == 40
