//! RA profile probe (domain-profile build, Stage 3 — RA profile authoring).
//!
//! Proves, against the engine's own machinery, the five behaviours the RA
//! profile's conformance plan (C-RA6/C-RA8 family) rests on:
//!   1. `profiles/ra.profile.json` parses and validates under
//!      `DomainProfile::from_json` + `validate()` (the engine's validator);
//!   2. a builder under the RA profile constructs a synthetic model graph
//!      (the `add_file` structural synthesis carries the model root; tenant
//!      edges enter via `add_typed_edge`);
//!   3. R2 enforcement: a `sourced_from` edge missing `as_of` and an
//!      `estimates` edge missing `error` are REFUSED with
//!      `MissingProvenance` diagnostics and never enter the graph, while
//!      fully-provenanced twins of the same kinds DO enter;
//!   4. the §4.3 graph class over the built graph is `tainted_claims`
//!      (a `references` edge is present) and the provenance summary counts
//!      every class actually present;
//!   5. core measures transfer: `blast_radius` of the WACC parameter over
//!      the profile's breakage set reaches exactly the cells that read it
//!      plus the output, and `path` returns the formula chain step for step.
//!
//! This is a probe, not the extractor: the synthetic model is hand-built
//! here so the mechanism is exercised before the RA extractor stage exists.

use codetopo_core::{
    Diagnostic, DomainProfile, Edge, EdgeKind, FileSymbols, GraphBuilder, NodeKind, RawSymbol,
    Snapshot,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

const RA_PROFILE_JSON: &str = include_str!("../../../profiles/ra.profile.json");

fn ra_profile() -> Arc<DomainProfile> {
    let p = DomainProfile::from_json(RA_PROFILE_JSON)
        .expect("ra.profile.json must parse under DomainProfile::from_json");
    assert_eq!(p.profile_id, "research-analyst");
    let v = p.validate();
    assert!(
        v.is_ok(),
        "ra.profile.json must validate under the engine validator: {:?}",
        v.err()
    );
    Arc::new(p)
}

fn sym(id: &str, kind: &str, line: u32, measures: &[(&str, Value)]) -> RawSymbol {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    for (k, v) in measures {
        m.insert(k.to_string(), v.clone());
    }
    RawSymbol {
        id: id.to_string(),
        kind: NodeKind::new(kind),
        label: id.rsplit("::").next().unwrap_or(id).to_string(),
        line,
        column: None,
        defines: Vec::new(),
        refs: Vec::new(),
        writes: Vec::new(),
        end_line: None,
        signature: None,
        decisions: None,
        measures: m,
    }
}

fn edge(from: &str, to: &str, kind: &str, attrs: &[(&str, Value)]) -> Edge {
    let mut map = Map::new();
    for (k, v) in attrs {
        map.insert(k.to_string(), v.clone());
    }
    Edge {
        from: from.to_string(),
        to: to.to_string(),
        kind: EdgeKind::new(kind),
        line: None,
        attrs: map,
    }
}

/// The synthetic model: package = the model root `probe-model`; the
/// "module" is the valuation sheet (the builder's structural middle layer —
/// see the test's note); symbols are the sheet's cells.
fn probe_builder() -> GraphBuilder {
    let profile = ra_profile();
    let mut b = GraphBuilder::with_profile(profile);
    // NOTE (probe scaffolding, named honestly): the core builder's
    // structural layer is package -> module -> file -> symbols, hardcoded
    // to the Module/File kinds. Under the RA profile the package slot IS
    // the model root (containment.root_kind = model), and the module/file
    // layers ride along as structural scaffolding kinds the RA dictionary
    // does not declare. The RA extractor stage must either generalize this
    // synthesis or drive the builder the same way this probe does; the
    // profile's own containment chain (model -> statement -> cells) is
    // carried by the contains edges added below via add_typed_edge.
    let fs = FileSymbols {
        package: "probe-model".to_string(),
        file: "valuation".to_string(),
        module_id: "probe-model::valuation".to_string(),
        symbols: vec![
            sym("probe-model::valuation", "statement", 1, &[]),
            sym(
                "probe-model::valuation::C5",
                "driver",
                5,
                &[
                    ("value", json!(0.08)),
                    ("formula", json!("0.08")),
                    ("unit", json!("rate")),
                ],
            ),
            sym(
                "probe-model::is::B10",
                "line_item",
                10,
                &[("value", json!(120.0)), ("period", json!("FY2025"))],
            ),
            sym(
                "probe-model::is::B12",
                "line_item",
                12,
                &[
                    ("value", json!(129.6)),
                    ("formula", json!("=B10*(1+'valuation'!C5)")),
                    ("period", json!("FY2026E")),
                ],
            ),
            sym(
                "probe-model::valuation::C2",
                "parameter",
                2,
                &[("value", json!(0.09)), ("formula", json!("0.09"))],
            ),
            sym(
                "probe-model::valuation::C20",
                "output",
                20,
                &[
                    ("value", json!(42.50)),
                    ("formula", json!("=C18/C19")),
                    ("unit", json!("USD")),
                ],
            ),
            // The boundary source as a declared symbol: add_typed_edge
            // requires both endpoints to exist, and the public builder
            // API has no boundary-stub materialization entry point (the
            // ensure_external path is private to build()'s import/call
            // resolution — Finding F-3). The RA extractor stage registers
            // source stubs the same way until the builder grows one.
            sym("source::stooq:aapl.us", "source", 0, &[]),
        ],
        imports: Vec::new(),
        calls: Vec::new(),
        heritage: Vec::new(),
    };
    let diags = b.add_file(fs);
    assert!(
        diags.is_empty(),
        "structural synthesis must be diagnostic-free, got: {diags:?}"
    );
    b
}

#[test]
fn ra_profile_parses_and_validates() {
    let _ = ra_profile();
}

#[test]
fn ra_builder_enforces_provenance_and_transfers_measures() {
    let mut b = probe_builder();

    // --- the dependency chain: output reads WACC; projected revenue reads
    // the growth driver and current revenue.
    let d = b.add_typed_edge(edge(
        "probe-model::valuation::C20",
        "probe-model::valuation::C2",
        "computes_into",
        &[("cell", json!("C20"))],
    ));
    assert!(
        d.is_empty(),
        "computes_into must be admitted cleanly: {d:?}"
    );
    let d = b.add_typed_edge(edge(
        "probe-model::is::B12",
        "probe-model::valuation::C5",
        "computes_into",
        &[],
    ));
    assert!(d.is_empty(), "{d:?}");
    let d = b.add_typed_edge(edge(
        "probe-model::is::B12",
        "probe-model::is::B10",
        "computes_into",
        &[],
    ));
    assert!(d.is_empty(), "{d:?}");

    // --- R2 refusal 1: sourced_from without as_of is refused by name.
    let d = b.add_typed_edge(edge(
        "probe-model::is::B10",
        "source::stooq:aapl.us",
        "sourced_from",
        &[("source", json!("stooq:aapl.us:daily-close"))],
    ));
    assert_eq!(d.len(), 1, "exactly one refusal diagnostic: {d:?}");
    match &d[0] {
        Diagnostic::MissingProvenance { missing, kind, .. } => {
            assert_eq!(missing, &vec!["as_of".to_string()]);
            assert_eq!(*kind, EdgeKind::new("sourced_from"));
        }
        other => panic!("expected MissingProvenance, got {other:?}"),
    }

    // --- the same edge, fully provenanced, IS admitted and enters.
    let d = b.add_typed_edge(edge(
        "probe-model::is::B10",
        "source::stooq:aapl.us",
        "sourced_from",
        &[
            ("source", json!("stooq:aapl.us:daily-close")),
            ("as_of", json!("2025-09-27")),
        ],
    ));
    assert!(d.is_empty(), "provenanced sourced_from must enter: {d:?}");

    // --- R2 refusal 2: estimates without error is refused by name.
    let d = b.add_typed_edge(edge(
        "probe-model::valuation::C5",
        "probe-model::is::B12",
        "estimates",
        &[
            ("method", json!("management guidance, FY2026 outlook")),
            ("window", json!("FY2026")),
        ],
    ));
    assert_eq!(d.len(), 1, "{d:?}");
    match &d[0] {
        Diagnostic::MissingProvenance { missing, .. } => {
            assert_eq!(missing, &vec!["error".to_string()]);
        }
        other => panic!("expected MissingProvenance, got {other:?}"),
    }
    // --- the fully-provenanced estimates twin IS admitted.
    let d = b.add_typed_edge(edge(
        "probe-model::valuation::C5",
        "probe-model::is::B12",
        "estimates",
        &[
            ("method", json!("management guidance, FY2026 outlook")),
            ("window", json!("FY2026")),
            ("error", json!("±2pp")),
        ],
    ));
    assert!(d.is_empty(), "provenanced estimates edge must enter: {d:?}");

    // --- a judgmental references edge carrying its claimant.
    let d = b.add_typed_edge(edge(
        "probe-model::valuation::C20",
        "probe-model::is::B12",
        "references",
        &[("claimant", json!("analyst:house-view"))],
    ));
    assert!(d.is_empty(), "{d:?}");

    let (graph, diags) = b.build();
    // Diagnostics of the whole build: exactly the two R2 refusals
    // (returned at offer time above and accumulated). Nothing else.
    let refusals = diags
        .iter()
        .filter(|x| matches!(x, Diagnostic::MissingProvenance { .. }))
        .count();
    assert_eq!(refusals, 2, "exactly the two R2 refusals: {diags:?}");
    assert_eq!(diags.len(), 2, "no other diagnostics: {diags:?}");

    // Exactly ONE sourced_from edge entered the graph: the provenanced
    // twin. The refused one never did.
    assert_eq!(graph.edge_count_by_kind(EdgeKind::new("sourced_from")), 1);
    assert_eq!(graph.edge_count_by_kind(EdgeKind::new("estimates")), 1);
    assert_eq!(graph.edge_count_by_kind(EdgeKind::new("computes_into")), 3);
    // The boundary node exists, under the profile's boundary kind.
    let stub = graph.get("source::stooq:aapl.us").expect("boundary node");
    assert_eq!(stub.kind, NodeKind::new("source"));

    // §4.3 as the engine implements it (graph_class_of): the class is
    // `claims-bearing` whenever any edge's class is neither
    // extracted_truth nor sourced_fact — here both an estimate and a
    // declared claim are present. (The founding spec's prose names the
    // same condition "tainted"; the engine's two-value vocabulary is
    // truths | claims-bearing, recorded here as Finding F-4: the spec's
    // finer tainted_estimate / tainted_claims distinction is NOT what
    // the shipped mechanism computes.) The summary counts EVERY edge by
    // class, containment included: contains edges are 9 here (package→
    // module, module→file, file→each of 7 symbols), so extracted_truth
    // = 9 contains + 3 computes_into = 12.
    let snap = Snapshot::from_graph(&graph, &[]);
    assert_eq!(snap.graph_class, "claims-bearing");
    assert_eq!(snap.provenance_summary.get("extracted_truth"), Some(&12));
    assert_eq!(snap.provenance_summary.get("sourced_fact"), Some(&1));
    assert_eq!(snap.provenance_summary.get("estimate"), Some(&1));
    assert_eq!(snap.provenance_summary.get("declared_claim"), Some(&1));

    // The profile's provenance table is what push_edge enforces.
    let prof = ra_profile();
    let need = prof.provenance_required_attrs(&EdgeKind::new("sourced_from"));
    assert_eq!(need, vec!["source", "as_of"]);
    let need_est = prof.provenance_required_attrs(&EdgeKind::new("estimates"));
    assert_eq!(need_est, vec!["method", "window", "error"]);

    // Core transfer: blast radius of the WACC parameter over the profile's
    // breakage set {computes_into, articulates} = exactly its reader.
    let br = graph.blast_radius("probe-model::valuation::C2");
    assert_eq!(br, vec!["probe-model::valuation::C20".to_string()]);

    // The audit trail: path from the projected-revenue cell to the growth
    // driver is the single formula step.
    let p = graph
        .path("probe-model::is::B12", "probe-model::valuation::C5")
        .expect("formula path must exist");
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].kind, EdgeKind::new("computes_into"));

    // Measured fields ride the nodes: value present where measured,
    // absent (not zero) where not — the statement node carries no value.
    let wacc = graph.get("probe-model::valuation::C2").expect("wacc node");
    assert_eq!(wacc.measure("value"), Some(json!(0.09)));
    let stmt = graph.get("probe-model::valuation").expect("statement node");
    assert_eq!(stmt.measure("value"), None);
}
