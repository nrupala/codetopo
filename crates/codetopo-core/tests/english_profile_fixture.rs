// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The ENGLISH profile's known-answer fixture
//! (`tests/fixture-english`).
//!
//! The paragraph is DESIGN.md section 1 of this repository — seven
//! dense technical sentences — parsed by the pinned companion
//! (tools/english-ingest; spaCy 3.8.18 / en_core_web_md 3.8.0) and
//! audited by eye in docs/english-accuracy.md (119/137 relations
//! correct; the audit is the profile's published accuracy
//! statement). The ground floor of `paragraph.snapshot.json` is the
//! companion's own output on `paragraph.md`, verified triple-for-
//! triple against the audited spike graph; the twelve claim edges
//! are the spike reading agent's, each carrying its claimant and
//! basis. Every expectation below is those audited counts. A change
//! that moves a count moves the fixture, the profile's conformance
//! oracle, and docs/english-accuracy.md in the same PR.

use std::collections::BTreeMap;
use std::path::Path;

use codetopo_core::profile::{DomainProfile, ProvenanceClass};
use codetopo_core::render::{layout_scoped, RenderScope};
use codetopo_core::schema::EdgeKind;
use codetopo_core::Snapshot;

const FIXTURE_SNAPSHOT: &str = include_str!("fixture-english/paragraph.snapshot.json");

fn fixture_graph() -> codetopo_core::Graph {
    let snap = Snapshot::from_json(FIXTURE_SNAPSHOT).expect("fixture snapshot parses");
    assert_eq!(snap.profile_id, "english");
    assert_eq!(snap.graph_class, "claims");
    snap.to_graph_checked()
        .expect("an english snapshot restores under the english profile")
}

#[test]
fn english_fixture_counts_are_the_audited_counts() {
    let g = fixture_graph();
    assert_eq!(g.profile().profile_id, "english");

    let mut nodes: BTreeMap<&str, usize> = BTreeMap::new();
    for n in g.nodes() {
        *nodes.entry(n.kind.as_str()).or_insert(0) += 1;
    }
    assert_eq!(nodes.get("document"), Some(&1));
    assert_eq!(nodes.get("sentence"), Some(&7));
    assert_eq!(nodes.get("predicate"), Some(&22));
    assert_eq!(nodes.get("entity"), Some(&45));
    assert_eq!(g.node_count(), 75);
    assert_eq!(g.edge_count(), 144);

    // The two storeys, counted by the profile's own classes:
    // 132 grammar edges ride as inference (74 contains + 58
    // relations), 12 interpretation edges as declared_claim.
    let profile = DomainProfile::english();
    let mut by_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for e in g.edges() {
        let class = profile
            .provenance_class(&e.kind)
            .expect("every fixture kind is declared")
            .as_str()
            .to_string();
        *by_class.entry(class).or_insert(0) += 1;
        *by_kind.entry(e.kind.as_str()).or_insert(0) += 1;
    }
    assert_eq!(by_class.get("inference"), Some(&132));
    assert_eq!(by_class.get("declared_claim"), Some(&12));
    assert_eq!(by_kind.get("contains"), Some(&74));
    assert_eq!(by_kind.get("subject-of"), Some(&13));
    assert_eq!(by_kind.get("object-of"), Some(&12));
    assert_eq!(by_kind.get("clause-of"), Some(&12));
    assert_eq!(by_kind.get("coordinated-with"), Some(&7));
    assert_eq!(by_kind.get("appositive-of"), Some(&6));
    assert_eq!(by_kind.get("complement-of"), Some(&4));
    assert_eq!(by_kind.get("modifier-of"), Some(&4));
    assert_eq!(by_kind.get("same-entity"), Some(&5));
    assert_eq!(by_kind.get("frames"), Some(&3));
    assert_eq!(by_kind.get("supports"), Some(&3));
    assert_eq!(by_kind.get("states-thesis-of"), Some(&1));
}

#[test]
fn every_claim_edge_names_its_claimant_and_basis() {
    // The labelling rule is the product's integrity: an
    // interpretation edge without a named reader and written
    // grounds is a smuggled assertion.
    let g = fixture_graph();
    let profile = DomainProfile::english();
    let mut claims = 0;
    for e in g.edges() {
        if profile.provenance_class(&e.kind) == Some(ProvenanceClass::DeclaredClaim) {
            claims += 1;
            let claimant = e.attrs.get("claimant").and_then(|v| v.as_str());
            let basis = e.attrs.get("basis").and_then(|v| v.as_str());
            assert!(
                claimant.map(|c| !c.trim().is_empty()) == Some(true),
                "claim edge {} -[{}]-> {} names its claimant",
                e.from, e.kind, e.to
            );
            assert!(
                basis.map(|b| !b.trim().is_empty()) == Some(true),
                "claim edge {} -[{}]-> {} states its basis",
                e.from, e.kind, e.to
            );
        }
    }
    assert_eq!(claims, 12);
}

#[test]
fn english_graph_can_never_reexport_as_truths() {
    // The snapshot as authored stamps graph_class "claims"; the
    // engine's section 4.3 computation over the restored graph
    // names the same condition "claims-bearing". Neither word is
    // "truths", and the summary split survives the round trip.
    let g = fixture_graph();
    let out = Snapshot::from_graph(&g, &[]);
    assert_eq!(out.profile_id, "english");
    assert_eq!(out.graph_class, "claims-bearing");
    assert_eq!(out.provenance_summary.get("inference"), Some(&132));
    assert_eq!(out.provenance_summary.get("declared_claim"), Some(&12));
}

#[test]
fn identity_edges_traverse_both_ways_and_directed_kinds_do_not() {
    let g = fixture_graph();
    // The spike's named case: S4's "no edges" (s4:e8) and S6's
    // "the edges" (s6:e1) are one entity; the same-entity edge
    // points anaphorically (s6:e1 -> s4:e8). A directed-only walk
    // answered "no path" from S4 to S6 — the connection exists and
    // the direction hid it. Identity is directionless, so both
    // directions now answer, each as a single same-entity step.
    for (a, b) in [("s4:e8", "s6:e1"), ("s6:e1", "s4:e8")] {
        let steps = g
            .path(a, b)
            .unwrap_or_else(|| panic!("identity path {a} -> {b} exists"));
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].kind, EdgeKind::new("same-entity"));
    }
    // Directed kinds stay directed: states-thesis-of runs
    // s6 -> para, and no path runs para -> s6 (containment is
    // never a path step, and the claim edges point the other way).
    assert!(g.path("s6", "para").is_some());
    assert!(g.path("para", "s6").is_none());
    // A grammar edge does not reverse either: the thesis
    // sentence's subject reaches its predicate ("reason" is the
    // subject of "is", s6:v5), not the reverse.
    assert!(g.path("s6:e0", "s6:v5").is_some());
    assert!(g.path("s6:v5", "s6:e0").is_none());
}

#[test]
fn structure_render_clusters_words_under_their_sentence() {
    let g = fixture_graph();
    let spec = layout_scoped(&g, RenderScope::Structure);
    // Every node placed exactly once — the clustered scope keeps
    // the whole graph (no roll-up).
    assert_eq!(spec.nodes.len(), 75);
    let pos: BTreeMap<&str, (u32, u32)> =
        spec.nodes.iter().map(|n| (n.id.as_str(), (n.x, n.y))).collect();
    // The document roots the map.
    assert_eq!(pos["para"].1, 0);
    // Seven sentence bands: distinct columns, all on the row
    // under the document.
    let mut band_of: BTreeMap<String, u32> = BTreeMap::new();
    for i in 1..=7 {
        let (x, y) = pos[format!("s{i}").as_str()];
        assert_eq!(y, 1, "sentence s{i} sits under the document");
        band_of.insert(format!("s{i}"), x);
    }
    let distinct: std::collections::BTreeSet<u32> = band_of.values().copied().collect();
    assert_eq!(distinct.len(), 7, "each sentence gets its own band");
    assert_eq!(spec.width, 7, "seven one-column bands");
    // Every word stands in its own sentence's column, below it.
    let profile = DomainProfile::english();
    for e in g.edges() {
        if profile.is_containment(&e.kind) && e.from.starts_with('s') && e.from.len() == 2 {
            let (sx, sy) = pos[e.from.as_str()];
            let (wx, wy) = pos[e.to.as_str()];
            assert_eq!(wx, sx, "{} stands under {}", e.to, e.from);
            assert!(wy > sy, "{} stands below {}", e.to, e.from);
        }
    }
    // The map is taller than the ribbon it replaces: the deepest
    // sentence (S2, twenty-one children) sets the height.
    assert_eq!(spec.height, 23);
    // Deterministic, like every layout here.
    assert_eq!(spec, layout_scoped(&g, RenderScope::Structure));
}

#[test]
fn unknown_profiles_are_refused_by_name() {
    let mut snap = Snapshot::from_json(FIXTURE_SNAPSHOT).expect("fixture parses");
    snap.profile_id = "klingon".to_string();
    let err = snap
        .to_graph_checked()
        .expect_err("an unknown profile is refused, never read under another dictionary");
    assert!(err.contains("klingon"), "{err}");
    assert!(err.contains("english"), "the refusal names what is embedded: {err}");
}

#[test]
fn fixture_paragraph_file_is_the_snapshot_source() {
    // The companion's input ships beside the snapshot: seven
    // sentences, one per line, wording identical to DESIGN.md
    // section 1 (emphasis stripped, as the spike recorded).
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-english");
    let text = std::fs::read_to_string(dir.join("paragraph.md")).expect("paragraph ships");
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 7);
    assert!(lines[0].starts_with("Every existing code-visualization tool"));
    assert!(lines[5].starts_with("Our entire reason to exist is the edges."));
}
