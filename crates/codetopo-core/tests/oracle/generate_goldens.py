#!/usr/bin/env python3
"""Oracle golden generator for the Rust analytics engine (SPEC §6).

Reads a fixture graph (snapshot JSON) from ../fixtures/oracle/, builds
the SPEC §3 analytics projection from it (dependency edges = every
kind except `contains`; external stubs and their edges excluded;
simple directed graph: parallel edges collapse, self-loops dropped),
runs the reference measures in networkx with the parameters the
method texts fix, and writes the committed golden next to the fixture
as <name>.golden.json.

Python is the validation oracle only: it never ships and is not
needed in CI — the Rust suite compares against these committed
goldens. Regenerating a golden is a deliberate, reviewed act: the PR
that changes one cites the oracle versions and the reason.

Usage: generate_goldens.py <fixture-name> [<fixture-name> ...]
Environment: the pinned venv (requirements.txt beside this script).
"""
import json
import platform
import sys
from pathlib import Path

import igraph as ig
import networkx as nx
import numpy

HERE = Path(__file__).resolve().parent
FIXTURES = HERE.parent / "fixtures" / "oracle"

DEPENDENCY_KINDS = {
    "imports", "calls", "inherits", "implements",
    "defines", "reads", "writes", "references",
}


def projection(fixture):
    nodes = sorted(
        n["id"] for n in fixture["nodes"]
        if n["kind"] != "external" and not n["id"].startswith("external::")
    )
    keep = set(nodes)
    edges = set()
    for e in fixture["edges"]:
        if e["kind"] not in DEPENDENCY_KINDS:
            continue
        if e["from"] in keep and e["to"] in keep and e["from"] != e["to"]:
            edges.add((e["from"], e["to"]))
    g = nx.DiGraph()
    g.add_nodes_from(nodes)
    g.add_edges_from(sorted(edges))
    return g


def measure(fn, g):
    try:
        return fn(g)
    except (nx.PowerIterationFailedConvergence, ZeroDivisionError) as exc:
        return {"absent": f"{type(exc).__name__}: oracle did not converge"}


def communities(fixture):
    """The communities cross-check (SPEC §6): python-igraph Leiden on
    the undirected multiplicity-weighted projection, with that
    partition's modularity computed by networkx. The Rust side is
    judged by partition quality + agreement (planted recovery on
    planted fixtures; Q and NMI on karate), never label identity."""
    nodes = sorted(
        n["id"] for n in fixture["nodes"]
        if n["kind"] != "external" and not n["id"].startswith("external::")
    )
    keep = set(nodes)
    idx = {node: i for i, node in enumerate(nodes)}
    weights = {}
    for e in fixture["edges"]:
        if e["kind"] not in DEPENDENCY_KINDS:
            continue
        a, b = e["from"], e["to"]
        if a in keep and b in keep and a != b:
            key = (min(a, b), max(a, b))
            weights[key] = weights.get(key, 0) + 1
    edges = sorted(weights)
    g = ig.Graph(
        n=len(nodes),
        edges=[(idx[a], idx[b]) for a, b in edges],
        directed=False,
    )
    g.es["weight"] = [weights[e] for e in edges]
    if edges:
        part = g.community_leiden(
            objective_function="modularity",
            weights="weight",
            resolution=1.0,
        )
        membership = {nodes[i]: int(part.membership[i]) for i in range(len(nodes))}
        und = nx.Graph()
        und.add_nodes_from(nodes)
        und.add_edges_from([(a, b, {"weight": w}) for (a, b), w in weights.items()])
        groups = {}
        for node, label in membership.items():
            groups.setdefault(label, set()).add(node)
        q = nx.community.modularity(und, groups.values(), weight="weight")
    else:
        membership = {node: i for i, node in enumerate(nodes)}
        q = None
    return {
        "igraph_partition": membership,
        "igraph_modularity": q,
        "igraph_version": ig.__version__,
    }


def main():
    for name in sys.argv[1:]:
        fixture = json.loads((FIXTURES / f"{name}.json").read_text())
        g = projection(fixture)
        golden = {
            "fixture": name,
            "oracle": {
                "networkx": nx.__version__,
                "numpy": numpy.__version__,
                "python": platform.python_version(),
                "generator": "tests/oracle/generate_goldens.py",
            },
            "parameters": {
                "pagerank": "alpha=0.85, tol=1e-6, max_iter=100",
                "betweenness": "normalized=True, endpoints=False, exact",
                "closeness": "wf_improved=True (networkx default)",
                "eigenvector": "max_iter=100, tol=1e-6",
                "katz": "alpha=0.1, beta=1.0, max_iter=100, tol=1e-6, normalized=True",
            },
            "nodes": sorted(g.nodes),
            "pagerank": measure(lambda g: nx.pagerank(g, alpha=0.85, tol=1e-6, max_iter=100), g),
            "betweenness": measure(lambda g: nx.betweenness_centrality(g, normalized=True, endpoints=False), g),
            "closeness": measure(lambda g: nx.closeness_centrality(g), g),
            "eigenvector": measure(lambda g: nx.eigenvector_centrality(g, max_iter=100, tol=1e-6), g),
            "katz": measure(lambda g: nx.katz_centrality(g, alpha=0.1, beta=1.0, max_iter=100, tol=1e-6, normalized=True), g),
            "communities": communities(fixture),
        }
        out = FIXTURES / f"{name}.golden.json"
        out.write_text(json.dumps(golden, indent=2, sort_keys=True) + "\n")
        print(f"wrote {out}")


if __name__ == "__main__":
    main()
