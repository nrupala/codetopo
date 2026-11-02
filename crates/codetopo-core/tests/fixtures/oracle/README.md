# Oracle fixtures — known-answer graphs for the analytics engine

Each fixture is a snapshot JSON (the §3 record) plus a committed
golden (`<name>.golden.json`) produced from the identical §3
projection by the validation oracle (networkx; `tests/oracle/`).
The Rust suite (`tests/oracle_goldens.rs`) compares the engine
against the goldens at the SPEC §6 tolerances, and — for `compass`
— against the hand arithmetic published here. The tests assert the
hand values, not implementation output.

## compass — the centrality known-answer fixture (RA1)

Eight nodes, nine `calls` edges:

```
a -> b, a -> c, b -> d, c -> d,   (a diamond)
d -> e, e -> f,                   (e is the bridge between the halves)
f -> g,                           (a tail)
b -> h                            (a leaf)
```

### Betweenness, by hand

All-pairs shortest paths (directed; endpoints excluded). The only
branching is at the diamond: a→d, a→e, a→f, a→g each have two
shortest paths (via b, via c), splitting their credit evenly. Raw
counts of paths through each internal node:

| node | pairs carried | raw |
|---|---|---|
| b | a→d ½, a→e ½, a→f ½, a→g ½, a→h 1 | 3 |
| c | a→d ½, a→e ½, a→f ½, a→g ½ | 2 |
| d | a→e, a→f, a→g, b→e, b→f, b→g, c→e, c→f, c→g (1 each) | 9 |
| e | a→f, a→g, b→f, b→g, c→f, c→g, d→f, d→g (1 each) | **8** |
| f | a→g, b→g, c→g, d→g, e→g (1 each) | 5 |
| a, g, h | — | 0 |

Normalized (directed): divide by (n−1)(n−2) = 7·6 = 42. The bridge
node **e measures 8/42 ≈ 0.190476** — the value the fixture exists
to pin: degree counts cannot see e (fan-in 1, fan-out 1); only the
path structure makes it the second-most important node here.

### Closeness, by hand (Wasserman–Faust)

Distances follow paths *to* the node (the directed convention both
oracles and the engine use). reachable−1 = number of ancestors,
Σd = sum of their distances:

| node | ancestors (distance) | (r)/(n−1) × (r)/(Σd) | value |
|---|---|---|---|
| a | — | 0 | 0 |
| b | a(1) | (1/7)(1/1) | 0.142857 |
| c | a(1) | (1/7)(1/1) | 0.142857 |
| d | b(1), c(1), a(2) | (3/7)(3/4) | 0.321429 |
| e | d(1), b(2), c(2), a(3) | (4/7)(4/8) | 0.285714 |
| f | e(1), d(2), b(3), c(3), a(4) | (5/7)(5/13) | 0.274725 |
| g | f(1), e(2), d(3), b(4), c(4), a(5) | (6/7)(6/19) | 0.270677 |
| h | b(1), a(2) | (2/7)(2/3) | 0.190476 |

Ordering asserted by the tests: **d > e > f > g > h > b = c > a**.

### PageRank ordering, hand-derived

Rank flows along edges and pools in the sinks. g collects f's whole
share each round; f collects e's; the diamond splits a's share
between b and c, which is why they tie and why h (fed by half of b)
sits below d (fed by all of b and c). Iterating the method
(damping 0.85, uniform start) to its fixed point gives the ordering
the tests assert: **g > f > e > d > h > b = c > a**, sum exactly 1.

### Eigenvector: absent, by hand

The compass graph is a DAG; its adjacency matrix is nilpotent and
has no dominant eigenvector. The power iteration collapses, and the
correct answer is the absence (`did not converge`) — asserted
against both the engine and the oracle, which independently refuse
(PowerIterationFailedConvergence).

## two-villages — the communities known-answer fixture (RA2)

Two 5-cliques (`a1..a5`, `b1..b5`) joined by one bridge edge
(`a1-b1`), plus one isolated node (`z9`). 11 nodes, 21 edges.
Planted partition: the two cliques + the singleton. Leiden must
recover it exactly at γ = 1.0.

### Modularity of the planted partition, by hand

m = 21. In each clique: internal weight l = 10 (all pairs); degree
sum d = 21 (the bridge endpoint has degree 5 — 4 clique edges + the
bridge — the other four have degree 4). The singleton contributes 0
(l = 0, d = 0).

Q = Σ_c [ l_c/m − (d_c/2m)² ] = 2 × [ 10/21 − (21/42)² ]
  = 2 × [ 0.476190 − 0.25 ] = **0.452381** (0.452380952…)

The tests assert the engine's Q against this hand value and against
networkx's modularity of the igraph partition (the §6 formula
cross-check), both at 1e-9.

## karate — the literature's standard case (RA2)

Zachary's karate club: 34 nodes, 78 edges, as shipped by networkx's
`karate_club_graph`. No planted partition is asserted. The rules
(SPEC §6): Q(engine) ≥ Q(igraph) − 0.005 and NMI(engine, igraph)
≥ 0.9, where the golden records igraph's Leiden partition and its
networkx modularity (Q ≈ 0.4198 at generation time).

## cycle-trap — the topo-sort fixture (RA3)

A planted 3-cycle (`x -> y -> z -> x`) inside a DAG: `app -> mid`,
`mid -> lib`, `mid -> x`, `z -> lib`. The expected answer
(asserted exactly by `tests/topo_sort_fixture.rs`, after the order
property is verified mechanically over every edge):

1. `lib` — everything downstream needs it first
2. `x, y, z` — one group, `cyclic: true`
3. `mid`
4. `app`

## ring — the eigenvector/katz fixture (RA1)

Four nodes, five edges: `w -> x, x -> y, y -> z, z -> w, x -> z` —
a directed cycle with a chord, so the spectral measures exist.

Betweenness by hand (normalized by (4−1)(4−2) = 6): the shortest
paths give raw counts w 3 (y→x, z→x, z→y), x 3 (w→y, w→z, z→y),
z 3 (x→w, y→w, y→x), y 0 — so w = x = z = 0.5, y = 0.0, matching
the golden exactly.
