// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The Research Analyst extractor: a financial-model workbook in
//! (XLSX bytes + a declaration), an RA-profile graph out.
//!
//! The declaration (`codetopo.ra-decl/1`) is what makes extraction a
//! rule rather than a guess: it names the model, its sheets, every
//! node's cell, kind, label, period and unit, the sources behind
//! hard-coded inputs, the articulation identities with their
//! tolerances, the scenario bindings, and the estimate / reference
//! edges. A workbook cell the declaration does not account for is an
//! extraction error — the graph must reproduce the workbook exactly,
//! so silence is not an option (RA Regression Contract C-RA1).
//!
//! Emission follows `profiles/ra.profile.json`:
//! - model → statements → cells, contained by `contains` edges;
//! - a formula cell's reads become `computes_into` edges
//!   (reader → read), derived from the formula TEXT by
//!   [`crate::eval::formula_reads`] — an edge exists even when the
//!   value cannot be evaluated;
//! - `value` is recomputed by [`crate::eval`] from the workbook's
//!   own formulas; a cell whose formula cannot be evaluated carries
//!   NO value (`not_measured`, never zero);
//! - `formula` is the stored formula text, whitespace-collapsed,
//!   WITH the leading `=` (the profile's rule);
//! - `unit` is read from the cell's number format or the
//!   declaration — never inferred from the label;
//! - hard-coded inputs the declaration marks as sourced get a
//!   `sourced_from` edge to a `source::` boundary stub carrying the
//!   mandatory `source` + `as_of` attrs (the builder refuses the
//!   edge, loudly, when one is missing — R2);
//! - the three articulation identities EXECUTE over the emitted
//!   graph's measured totals (format gap F-2, closed here): a
//!   failure is a [`Diagnostic::ArticulationIdentity`] carrying both
//!   sides' values; the values are never repaired;
//! - a formula reference to a cell that does not exist is a
//!   formula_resolution finding ([`Diagnostic::UnresolvedCall`], the
//!   profile's `unresolved_call` slot) and the reading cell's value
//!   is absent;
//! - two cells the declaration presents as THE per-share (or equity)
//!   value are a [`Diagnostic::MultiDefinition`] naming both (the
//!   profile's `single_valuation_output` rule);
//! - scenario bindings become `assumes` edges, and each scenario is
//!   re-evaluated with its overrides so every scenario's graph
//!   carries its own measured values.
//!
//! Analysis-only: this module measures a model's structure and
//! arithmetic. It contains no recommendation, score, or verdict
//! about any security, and no market data of its own — prices enter
//! only as `source::` boundary nodes named by a declaration.

use std::collections::BTreeMap;
use std::sync::Arc;

use codetopo_core::diag::Diagnostic;
use codetopo_core::emit::RawSymbol;
use codetopo_core::graph::{Graph, GraphBuilder};
use codetopo_core::profile::{DomainProfile, ProvenanceClass};
use codetopo_core::schema::{Edge, EdgeKind, NodeKind};
use serde_json::{json, Map, Value};

use crate::eval::{formula_reads, EvalBook, EvalError};
use crate::xlsx::{read_workbook, Sheet};
use crate::ExtractError;

// ---------------------------------------------------------------------------
// The declaration.
// ---------------------------------------------------------------------------

/// A parsed `codetopo.ra-decl/1` declaration.
#[derive(Debug, Clone)]
pub struct WorkbookDecl {
    /// The model id — the graph root and every cell id's prefix.
    pub model_id: String,
    /// The artefact's file name (locations name it).
    pub artifact: String,
    /// The model's display label.
    pub label: String,
    /// Default absolute tolerance for identities, in currency units.
    pub tolerance: f64,
    /// Declared sheets, in workbook order.
    pub sheets: Vec<SheetDecl>,
    /// Articulation identities to execute.
    pub identities: Vec<IdentityDecl>,
    /// Scenario nodes + bindings (empty for a single-case model).
    pub scenarios: Vec<ScenarioDecl>,
    /// Estimate edges (driver/parameter → cell) with their mandatory
    /// provenance; `error` absent means the edge MUST be refused.
    pub estimates: Vec<EstimateDecl>,
    /// Reference edges; `claimant` present = a declared claim,
    /// absent = a found reference.
    pub references: Vec<ReferenceDecl>,
}

/// One declared sheet.
#[derive(Debug, Clone)]
pub struct SheetDecl {
    /// The sheet's workbook name (must match the XLSX sheet name).
    pub name: String,
    /// The statement node's label.
    pub label: String,
    /// Column holding row captions (e.g. `A`); caption cells are not
    /// nodes and need no declaration.
    pub label_column: String,
    /// Declared cells by reference.
    pub cells: BTreeMap<String, CellDecl>,
}

/// One declared cell.
#[derive(Debug, Clone)]
pub struct CellDecl {
    /// Profile node kind: line_item | driver | parameter | output.
    pub kind: String,
    /// The node's label.
    pub label: String,
    /// The period measure (line_item/output only), exactly as printed.
    pub period: Option<String>,
    /// The unit measure; when absent, the cell's number format is read.
    pub unit: Option<String>,
    /// The source behind a hard-coded input.
    pub source: Option<SourceDecl>,
    /// `per_share_value` / `equity_value` when the model presents this
    /// output as THE value of that kind (the single_valuation_output
    /// rule counts these).
    pub presented_as: Option<String>,
}

/// A declared source for one cell.
#[derive(Debug, Clone)]
pub struct SourceDecl {
    /// The boundary stub id (`source::...`).
    pub stub: String,
    /// The stub's label.
    pub label: String,
    /// The `source` provenance attr (e.g. a filing accession).
    pub source: String,
    /// The `as_of` provenance attr; ABSENT ON PURPOSE in refusal
    /// fixtures — the edge must then be refused by the builder.
    pub as_of: Option<String>,
}

/// One articulation identity, executed over measured totals.
#[derive(Debug, Clone)]
pub struct IdentityDecl {
    /// The profile identity name (`balance_sheet_equation`, ...).
    pub name: String,
    /// The full well-formedness name, for reporting.
    pub rule_name: String,
    /// Left side total: a cell ref (`sheet!B12`) — its measured value.
    pub lhs: String,
    /// Right side total: a cell ref, or a SIGNED SUM of terms
    /// (`sheet!B8+sheet!B9`, `bs!B11+is!C10-cf!C7`) evaluated from
    /// measured values.
    pub rhs: String,
    /// Absolute tolerance; defaults to the declaration's tolerance.
    pub tolerance: Option<f64>,
}

/// One scenario: a node plus the driver/parameter cells it rebinds.
#[derive(Debug, Clone)]
pub struct ScenarioDecl {
    /// Scenario name (`base`, `bull`, `bear`, ...).
    pub name: String,
    /// Display label.
    pub label: String,
    /// Bindings: `"sheet!Ref"` → replacement value.
    pub binds: BTreeMap<String, f64>,
}

/// One estimates edge declaration.
#[derive(Debug, Clone)]
pub struct EstimateDecl {
    /// From cell: `"sheet!Ref"` (a driver or parameter).
    pub from: String,
    /// To cell: `"sheet!Ref"` (a line_item or output).
    pub to: String,
    /// The `method` attr.
    pub method: String,
    /// The `window` attr.
    pub window: String,
    /// The `error` attr; absent = the edge must be refused.
    pub error: Option<String>,
}

/// One references edge declaration.
#[derive(Debug, Clone)]
pub struct ReferenceDecl {
    /// From cell: `"sheet!Ref"`.
    pub from: String,
    /// To cell: `"sheet!Ref"`.
    pub to: String,
    /// The `claimant` attr; present = declared_claim per edge.
    pub claimant: Option<String>,
}

fn decl_err(msg: impl Into<String>) -> ExtractError {
    ExtractError::Declaration(msg.into())
}

fn req_str<'a>(obj: &'a Map<String, Value>, key: &str, ctx: &str) -> Result<&'a str, ExtractError> {
    obj.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| decl_err(format!("{ctx}: missing string field '{key}'")))
}

fn opt_str(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Parse a declaration from its JSON text.
pub fn parse_decl(text: &str) -> Result<WorkbookDecl, ExtractError> {
    let root: Value = serde_json::from_str(text)
        .map_err(|e| decl_err(format!("declaration is not valid JSON: {e}")))?;
    let obj = root
        .as_object()
        .ok_or_else(|| decl_err("declaration root is not an object"))?;
    let format = obj.get("format").and_then(Value::as_str).unwrap_or("");
    if format != "codetopo.ra-decl/1" {
        return Err(decl_err(format!(
            "declaration format must be codetopo.ra-decl/1, got '{format}'"
        )));
    }
    let model_id = req_str(obj, "model_id", "declaration")?.to_string();
    let artifact = req_str(obj, "artifact", "declaration")?.to_string();
    let label = opt_str(obj, "label").unwrap_or_else(|| model_id.clone());
    let tolerance = obj
        .get("tolerance")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| decl_err("declaration: missing numeric field 'tolerance'"))?;

    let mut sheets = Vec::new();
    for sv in obj
        .get("sheets")
        .and_then(Value::as_array)
        .ok_or_else(|| decl_err("declaration: 'sheets' must be an array"))?
    {
        let so = sv
            .as_object()
            .ok_or_else(|| decl_err("declaration: sheet entry is not an object"))?;
        let name = req_str(so, "name", "sheet")?.to_string();
        let mut cells = BTreeMap::new();
        for (reference, cv) in so
            .get("cells")
            .and_then(Value::as_object)
            .ok_or_else(|| decl_err(format!("sheet {name}: 'cells' must be an object")))?
        {
            let co = cv
                .as_object()
                .ok_or_else(|| decl_err(format!("sheet {name} cell {reference}: not an object")))?;
            let source = match co.get("source") {
                Some(sv) => {
                    let so2 = sv.as_object().ok_or_else(|| {
                        decl_err(format!("sheet {name} cell {reference}: source not an object"))
                    })?;
                    Some(SourceDecl {
                        stub: req_str(so2, "stub", "source")?.to_string(),
                        label: opt_str(so2, "label").unwrap_or_default(),
                        source: req_str(so2, "source", "source")?.to_string(),
                        as_of: opt_str(so2, "as_of"),
                    })
                }
                None => None,
            };
            cells.insert(
                reference.clone(),
                CellDecl {
                    kind: req_str(co, "kind", "cell")?.to_string(),
                    label: req_str(co, "label", "cell")?.to_string(),
                    period: opt_str(co, "period"),
                    unit: opt_str(co, "unit"),
                    source,
                    presented_as: opt_str(co, "presented_as"),
                },
            );
        }
        sheets.push(SheetDecl {
            name: name.clone(),
            label: opt_str(so, "label").unwrap_or(name),
            label_column: opt_str(so, "label_column").unwrap_or_else(|| "A".to_string()),
            cells,
        });
    }

    let mut identities = Vec::new();
    for iv in obj.get("identities").and_then(Value::as_array).cloned().unwrap_or_default() {
        let io = iv
            .as_object()
            .ok_or_else(|| decl_err("declaration: identity entry is not an object"))?;
        identities.push(IdentityDecl {
            name: req_str(io, "name", "identity")?.to_string(),
            rule_name: opt_str(io, "rule_name").unwrap_or_default(),
            lhs: req_str(io, "lhs", "identity")?.to_string(),
            rhs: req_str(io, "rhs", "identity")?.to_string(),
            tolerance: io.get("tolerance").and_then(|v| v.as_f64()),
        });
    }

    let mut scenarios = Vec::new();
    for sv in obj.get("scenarios").and_then(Value::as_array).cloned().unwrap_or_default() {
        let so = sv
            .as_object()
            .ok_or_else(|| decl_err("declaration: scenario entry is not an object"))?;
        let mut binds = BTreeMap::new();
        for (k, v) in so
            .get("binds")
            .and_then(Value::as_object)
            .ok_or_else(|| decl_err("declaration: scenario 'binds' must be an object"))?
        {
            binds.insert(
                k.clone(),
                v.as_f64()
                    .ok_or_else(|| decl_err(format!("scenario bind {k}: not a number")))?,
            );
        }
        scenarios.push(ScenarioDecl {
            name: req_str(so, "name", "scenario")?.to_string(),
            label: opt_str(so, "label").unwrap_or_default(),
            binds,
        });
    }

    let mut estimates = Vec::new();
    for ev in obj.get("estimates").and_then(Value::as_array).cloned().unwrap_or_default() {
        let eo = ev
            .as_object()
            .ok_or_else(|| decl_err("declaration: estimates entry is not an object"))?;
        estimates.push(EstimateDecl {
            from: req_str(eo, "from", "estimates")?.to_string(),
            to: req_str(eo, "to", "estimates")?.to_string(),
            method: req_str(eo, "method", "estimates")?.to_string(),
            window: req_str(eo, "window", "estimates")?.to_string(),
            error: opt_str(eo, "error"),
        });
    }

    let mut references = Vec::new();
    for rv in obj.get("references").and_then(Value::as_array).cloned().unwrap_or_default() {
        let ro = rv
            .as_object()
            .ok_or_else(|| decl_err("declaration: references entry is not an object"))?;
        references.push(ReferenceDecl {
            from: req_str(ro, "from", "references")?.to_string(),
            to: req_str(ro, "to", "references")?.to_string(),
            claimant: opt_str(ro, "claimant"),
        });
    }

    Ok(WorkbookDecl {
        model_id,
        artifact,
        label,
        tolerance,
        sheets,
        identities,
        scenarios,
        estimates,
        references,
    })
}

// ---------------------------------------------------------------------------
// Extraction.
// ---------------------------------------------------------------------------

/// The result of one workbook extraction: the base graph plus one
/// graph per declared scenario (base graph first-class; scenario
/// graphs carry the same structure with their own measured values).
pub struct RaExtraction {
    /// The graph under the workbook's own (base) values.
    pub graph: Graph,
    /// Diagnostics of the base build, in deterministic order.
    pub diagnostics: Vec<Diagnostic>,
    /// (scenario name, graph) for each declared scenario.
    pub scenarios: Vec<(String, Graph)>,
}

/// The RA profile, parsed from the same JSON the repo ships.
pub fn ra_profile() -> Result<Arc<DomainProfile>, ExtractError> {
    let text = include_str!("../../../profiles/ra.profile.json");
    DomainProfile::from_json(text)
        .map(Arc::new)
        .map_err(|e| decl_err(format!("the embedded RA profile does not validate: {e}")))
}

/// Build a RawSymbol with empty name-lists and no optional fields —
/// tenant nodes state their edges explicitly (see
/// `GraphBuilder::add_node`), so the CODE name-list fields stay empty.
/// `loc_file` names the artefact the node lives in (the profile's
/// location record: model-level nodes name the artefact, cells name
/// the sheet) and travels into the node's Loc, so duplicate and
/// diagnostic locations name real places.
fn node(
    id: String,
    kind: &str,
    label: String,
    loc_file: String,
    line: u32,
    column: Option<u32>,
    measures: BTreeMap<String, Value>,
) -> RawSymbol {
    RawSymbol {
        id,
        kind: NodeKind::new(kind),
        label,
        line,
        column,
        defines: Vec::new(),
        refs: Vec::new(),
        writes: Vec::new(),
        end_line: None,
        signature: None,
        decisions: None,
        measures,
        loc_file,
    }
}

/// Split a `"sheet!Ref"` declaration reference.
fn split_ref(s: &str) -> Result<(String, String), ExtractError> {
    let (sheet, reference) = s
        .rsplit_once('!')
        .ok_or_else(|| decl_err(format!("cell reference '{s}' is not sheet-qualified")))?;
    Ok((sheet.to_string(), reference.to_string()))
}

/// The node id of a cell: `{model}::{sheet}::{Ref}`.
fn cell_id(model: &str, sheet: &str, reference: &str) -> String {
    format!("{model}::{sheet}::{reference}")
}

/// The currency a number format names, if any: `$`/USD → USD,
/// `€` → EUR, `£` → GBP. Read from the artefact, never inferred.
fn currency_of_format(format: &str) -> Option<&'static str> {
    if format.contains('$') || format.contains("USD") {
        Some("USD")
    } else if format.contains('€') || format.contains("EUR") {
        Some("EUR")
    } else if format.contains('£') || format.contains("GBP") {
        Some("GBP")
    } else {
        None
    }
}

/// Extract a workbook (XLSX bytes) under a declaration into
/// RA-profile graphs.
pub fn extract_workbook(
    bytes: &[u8],
    decl: &WorkbookDecl,
) -> Result<RaExtraction, ExtractError> {
    let sheets = read_workbook(bytes)?;
    extract_from_sheets(&sheets, decl, &EvalBook::from_sheets(&sheets))
}

/// The sheet-driven core of [`extract_workbook`], shared with the
/// scenario pass: `sheets` give structure and formulas; `book`
/// gives the values (base book, or a scenario-overridden one).
/// (Public within the crate's RA surface: the statistics siblings
/// re-evaluate through this same entry, so a sibling number is the
/// extractor's own arithmetic, never a second evaluator.)
pub fn extract_from_sheets_pub(
    sheets: &[Sheet],
    decl: &WorkbookDecl,
    book: &EvalBook,
) -> Result<RaExtraction, ExtractError> {
    extract_from_sheets(sheets, decl, book)
}

fn extract_from_sheets(
    sheets: &[Sheet],
    decl: &WorkbookDecl,
    book: &EvalBook,
) -> Result<RaExtraction, ExtractError> {
    let profile = ra_profile()?;

    // The declaration's sheets must be exactly the workbook's sheets.
    let wb_names: Vec<&str> = sheets.iter().map(|s| s.name.as_str()).collect();
    let decl_names: Vec<&str> = decl.sheets.iter().map(|s| s.name.as_str()).collect();
    if wb_names != decl_names {
        return Err(decl_err(format!(
            "declaration sheets {decl_names:?} do not match workbook sheets {wb_names:?}"
        )));
    }

    // Every non-empty workbook cell must be declared (or be a label
    // caption in the sheet's label column). Anything else is an
    // extraction error: the graph reproduces the workbook exactly.
    for (sheet, sdecl) in sheets.iter().zip(decl.sheets.iter()) {
        let label_col = crate::xlsx::parse_ref(&format!("{}1", sdecl.label_column))
            .map(|(_, c)| c)
            .unwrap_or(1);
        for cell in &sheet.cells {
            if sdecl.cells.contains_key(&cell.reference) {
                continue;
            }
            if cell.column == label_col && cell.formula.is_none() {
                continue; // a row caption, not a node
            }
            return Err(decl_err(format!(
                "sheet {} cell {} is not declared (kind, label, period, unit are declaration facts, never guessed)",
                sheet.name, cell.reference
            )));
        }
        // Every declared cell must exist in the workbook.
        for reference in sdecl.cells.keys() {
            if !sheet.cells.iter().any(|c| &c.reference == reference) {
                return Err(decl_err(format!(
                    "sheet {} declares cell {} which the workbook does not contain",
                    sheet.name, reference
                )));
            }
        }
    }

    let graph = build_graph(sheets, decl, book, &profile)?;
    let mut scenarios = Vec::new();
    for scenario in &decl.scenarios {
        let mut overrides: BTreeMap<(String, String), f64> = BTreeMap::new();
        for (key, value) in &scenario.binds {
            overrides.insert(split_ref(key)?, *value);
        }
        let sbook = EvalBook::from_sheets(sheets).with_overrides(overrides);
        let sgraph = build_graph(sheets, decl, &sbook, &profile)?;
        scenarios.push((scenario.name.clone(), sgraph.0));
    }
    Ok(RaExtraction {
        graph: graph.0,
        diagnostics: graph.1,
        scenarios,
    })
}

/// Evaluate every cell and build one graph. Returns (graph,
/// diagnostics).
fn build_graph(
    sheets: &[Sheet],
    decl: &WorkbookDecl,
    book: &EvalBook,
    profile: &Arc<DomainProfile>,
) -> Result<(Graph, Vec<Diagnostic>), ExtractError> {
    // ---- Evaluate: literals directly; formulas via the evaluator.
    let mut values: BTreeMap<(String, String), f64> = BTreeMap::new();
    let mut eval_errors: BTreeMap<(String, String), EvalError> = BTreeMap::new();
    for ((sheet, reference), result) in book.evaluate_all() {
        match result {
            Ok(v) => {
                values.insert((sheet, reference), v);
            }
            Err(e) => {
                eval_errors.insert((sheet, reference), e);
            }
        }
    }
    for sheet in sheets {
        for cell in &sheet.cells {
            if cell.formula.is_none() {
                if let Some(n) = cell.number {
                    values.insert((sheet.name.clone(), cell.reference.clone()), n);
                }
            }
        }
    }

    let mut builder = GraphBuilder::with_profile(profile.clone());
    let model_id = decl.model_id.clone();

    // ---- Nodes: model, statements, period nodes, cells.
    builder.add_node(node(model_id.clone(), "model", decl.label.clone(), decl.artifact.clone(), 0, None, BTreeMap::new()));
    let mut period_ids: BTreeMap<String, String> = BTreeMap::new();
    for sdecl in &decl.sheets {
        for cell in sdecl.cells.values() {
            if let Some(period) = &cell.period {
                period_ids
                    .entry(period.clone())
                    .or_insert_with(|| format!("{model_id}::period::{period}"));
            }
        }
    }
    for (period, id) in &period_ids {
        builder.add_node(node(id.clone(), "period", period.clone(), decl.artifact.clone(), 0, None, BTreeMap::new()));
        builder.add_contains(&model_id, id);
    }
    for (sheet, sdecl) in sheets.iter().zip(decl.sheets.iter()) {
        let sid = format!("{model_id}::{}", sheet.name);
        builder.add_node(node(sid.clone(), "statement", sdecl.label.clone(), decl.artifact.clone(), 1, None, BTreeMap::new()));
        builder.add_contains(&model_id, &sid);
        for (reference, cell_decl) in &sdecl.cells {
            let cell = sheet
                .cells
                .iter()
                .find(|c| &c.reference == reference)
                .expect("declared cell existence checked above");
            let mut measures: BTreeMap<String, Value> = BTreeMap::new();
            if let Some(f) = &cell.formula {
                let collapsed = f.split_whitespace().collect::<Vec<_>>().join(" ");
                measures.insert("formula".to_string(), json!(format!("={collapsed}")));
            }
            if let Some(v) = values.get(&(sheet.name.clone(), reference.clone())) {
                measures.insert("value".to_string(), json!(v));
            }
            let unit = cell_decl
                .unit
                .clone()
                .or_else(|| {
                    cell.number_format
                        .as_deref()
                        .and_then(currency_of_format)
                        .map(str::to_string)
                });
            if let Some(u) = unit {
                measures.insert("unit".to_string(), json!(u));
            }
            if let Some(p) = &cell_decl.period {
                measures.insert("period".to_string(), json!(p));
            }
            builder.add_node(node(
                cell_id(&model_id, &sheet.name, reference),
                &cell_decl.kind,
                cell_decl.label.clone(),
                sheet.name.clone(),
                cell.row,
                Some(cell.column),
                measures,
            ));
            builder.add_contains(&sid, &cell_id(&model_id, &sheet.name, reference));
        }
    }

    // ---- Boundary stubs + sourced_from edges.
    for sdecl in &decl.sheets {
        for (reference, cell_decl) in &sdecl.cells {
            if let Some(src) = &cell_decl.source {
                builder.add_boundary_stub(&src.stub, &src.label);
                let mut attrs = Map::new();
                attrs.insert("source".to_string(), json!(src.source));
                if let Some(as_of) = &src.as_of {
                    attrs.insert("as_of".to_string(), json!(as_of));
                }
                builder.add_typed_edge(Edge {
                    from: cell_id(&model_id, &sdecl.name, reference),
                    to: src.stub.clone(),
                    kind: EdgeKind::new("sourced_from"),
                    line: None,
                    attrs,
                });
            }
        }
    }

    // ---- computes_into edges from the formula texts.
    for (sheet_name, reference, formula) in book.formula_cells() {
        let from_id = cell_id(&model_id, &sheet_name, &reference);
        match formula_reads(&formula, &sheet_name) {
            Ok(reads) => {
                for (rsheet, rref) in reads {
                    let to_id = cell_id(&model_id, &rsheet, &rref);
                    if book.cell_exists(&rsheet, &rref) {
                        builder.add_typed_edge(Edge {
                            from: from_id.clone(),
                            to: to_id,
                            kind: EdgeKind::new("computes_into"),
                            line: None,
                            attrs: Map::new(),
                        });
                    } else {
                        // formula_resolution: the reference names a
                        // cell the workbook does not contain. The
                        // finding is recorded; no edge is invented.
                        builder.record_diagnostic(Diagnostic::UnresolvedCall {
                            from: from_id.clone(),
                            target: format!("{rsheet}!{rref}"),
                            line: crate::xlsx::parse_ref(&reference).map(|(r, _)| r).unwrap_or(0),
                        });
                    }
                }
            }
            Err(_) => {
                // An unparseable formula yields no reads; its value
                // is already absent via eval_errors.
            }
        }
    }

    // ---- SINGLE VALUATION OUTPUT (profile well-formedness): a
    // presented_as tag carried by two cells is a multi-definition
    // finding naming both. Checked over the declaration — the model's
    // own presentation — in deterministic (sheet, ref) order.
    {
        let mut presented: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();
        for sdecl in &decl.sheets {
            for (reference, cell_decl) in &sdecl.cells {
                if let Some(tag) = &cell_decl.presented_as {
                    presented
                        .entry(tag.as_str())
                        .or_default()
                        .push((sdecl.name.clone(), reference.clone()));
                }
            }
        }
        for (tag, cells) in &presented {
            if cells.len() > 1 {
                let locs: Vec<codetopo_core::schema::Loc> = cells
                    .iter()
                    .map(|(sheet, reference)| codetopo_core::schema::Loc {
                        file: format!("{}:{sheet}", decl.artifact),
                        line: crate::xlsx::parse_ref(reference).map(|(r, _)| r).unwrap_or(0),
                        column: crate::xlsx::parse_ref(reference).map(|(_, c)| c),
                    })
                    .collect();
                builder.record_diagnostic(Diagnostic::MultiDefinition {
                    symbol: format!("{model_id}::presented_as::{tag}"),
                    locations: locs,
                });
            }
        }
    }

    // ---- articulates pairs: one directed edge per declared
    // identity, joining the statements its two sides live on.
    for identity in &decl.identities {
        let (lsheet, _) = split_ref(&identity.lhs)?;
        let (rsheet, _) = split_ref(identity.rhs.split(['+', '-']).next().unwrap_or(""))?;
        if lsheet != rsheet {
            let mut attrs = Map::new();
            attrs.insert("identity".to_string(), json!(identity.name));
            attrs.insert(
                "tolerance".to_string(),
                json!(identity.tolerance.unwrap_or(decl.tolerance)),
            );
            builder.add_typed_edge(Edge {
                from: format!("{model_id}::{lsheet}"),
                to: format!("{model_id}::{rsheet}"),
                kind: EdgeKind::new("articulates"),
                line: None,
                attrs,
            });
        }
    }

    // ---- Scenario nodes + assumes edges.
    for scenario in &decl.scenarios {
        let sid = format!("{model_id}::scenario::{}", scenario.name);
        builder.add_node(node(
            sid.clone(),
            "scenario",
            decl.artifact.clone(),
            if scenario.label.is_empty() {
                scenario.name.clone()
            } else {
                scenario.label.clone()
            },
            0,
            None,
            BTreeMap::new(),
        ));
        builder.add_contains(&model_id, &sid);
        for key in scenario.binds.keys() {
            let (bsheet, bref) = split_ref(key)?;
            builder.add_typed_edge(Edge {
                from: sid.clone(),
                to: cell_id(&model_id, &bsheet, &bref),
                kind: EdgeKind::new("assumes"),
                line: None,
                attrs: Map::new(),
            });
        }
    }

    // ---- estimates edges (R2 refuses any missing its error attr).
    for est in &decl.estimates {
        let (fsheet, fref) = split_ref(&est.from)?;
        let (tsheet, tref) = split_ref(&est.to)?;
        let mut attrs = Map::new();
        attrs.insert("method".to_string(), json!(est.method));
        attrs.insert("window".to_string(), json!(est.window));
        if let Some(error) = &est.error {
            attrs.insert("error".to_string(), json!(error));
        }
        builder.add_typed_edge(Edge {
            from: cell_id(&model_id, &fsheet, &fref),
            to: cell_id(&model_id, &tsheet, &tref),
            kind: EdgeKind::new("estimates"),
            line: None,
            attrs,
        });
    }

    // ---- references edges.
    for r in &decl.references {
        let (fsheet, fref) = split_ref(&r.from)?;
        let (tsheet, tref) = split_ref(&r.to)?;
        let mut attrs = Map::new();
        if let Some(claimant) = &r.claimant {
            attrs.insert("claimant".to_string(), json!(claimant));
        }
        builder.add_typed_edge(Edge {
            from: cell_id(&model_id, &fsheet, &fref),
            to: cell_id(&model_id, &tsheet, &tref),
            kind: EdgeKind::new("references"),
            line: None,
            attrs,
        });
    }

    let (graph, mut diagnostics) = builder.build();

    // ---- F-2 EXECUTION: the articulation identities over the
    // emitted graph's measured values. A failure is reported with
    // both sides; nothing is repaired.
    for identity in &decl.identities {
        let tol = identity.tolerance.unwrap_or(decl.tolerance);
        let lhs = identity_side(&identity.lhs, &graph, &model_id);
        let rhs = identity_rhs(&identity.rhs, &graph, &model_id);
        if let (Some(l), Some(r)) = (lhs, rhs) {
            if (l - r).abs() > tol {
                diagnostics.push(Diagnostic::ArticulationIdentity {
                    identity: identity.name.clone(),
                    lhs: l,
                    rhs: r,
                    tolerance: tol,
                });
            }
        }
        // A side with no measured value cannot be judged: the
        // identity is neither passed nor failed — the absence is
        // already carried by the cells' missing values.
    }

    Ok((graph, diagnostics))
}

/// The right side of an identity: a SIGNED sum of cell terms
/// (`sheet!B8+sheet!B9`, `bs!B11+is!C10-cf!C7`). The rollforward
/// identity subtracts by definition (dividends, buybacks), so the
/// grammar carries signs; a term whose cell has no measured value
/// makes the whole side unmeasured (the identity is then neither
/// passed nor failed — see the caller).
fn identity_rhs(rhs: &str, graph: &Graph, model_id: &str) -> Option<f64> {
    let mut terms: Vec<(f64, &str)> = Vec::new();
    let mut sign = 1.0f64;
    let mut start = 0usize;
    for (i, c) in rhs.char_indices() {
        if c == '+' || c == '-' {
            let t = rhs[start..i].trim();
            if !t.is_empty() {
                terms.push((sign, t));
            }
            sign = if c == '+' { 1.0 } else { -1.0 };
            start = i + 1;
        }
    }
    let t = rhs[start..].trim();
    if !t.is_empty() {
        terms.push((sign, t));
    }
    let mut total = 0.0f64;
    for (s, t) in terms {
        total += s * identity_side(t, graph, model_id)?;
    }
    Some(total)
}

/// One side of an identity: a single cell's measured value.
fn identity_side(term: &str, graph: &Graph, model_id: &str) -> Option<f64> {
    let (sheet, reference) = term.rsplit_once('!')?;
    let node = graph.get(&cell_id(model_id, sheet, reference))?;
    node.measure("value")?.as_f64()
}

// ---------------------------------------------------------------------------
// The filed-statements entry (C-RA3's graph source).
// ---------------------------------------------------------------------------

/// One filed series: the exact statement series an RA-F4-class
/// capture fed the RA engine — field name → per-period values
/// (missing = `None`, never zero) — plus the per-field source
/// concepts for fields the series itself drops.
#[derive(Debug, Clone)]
pub struct FiledSeries {
    /// Model id (e.g. the ticker's model name).
    pub model_id: String,
    /// The artefact label (the filing set).
    pub label: String,
    /// Period labels, oldest → newest (`FY2023`, `FY2024`).
    pub periods: Vec<String>,
    /// Period end dates (the `as_of` of every sourced value).
    pub period_ends: Vec<String>,
    /// The boundary stub id (the filing accession set).
    pub source_stub: String,
    /// The stub's label.
    pub source_label: String,
    /// The `source` attr value (e.g. `sec-edgar:companyfacts`).
    pub source_name: String,
    /// Field rows: (field name, statement sheet, label, per-period
    /// values). Sheets: `is`, `bs`, `cf`.
    pub fields: Vec<(String, String, String, Vec<Option<f64>>)>,
    /// Unit measure for the filed values (from the filing).
    pub unit: String,
}

/// Build the RA graph of a filed series: every filed value a
/// line_item input under a `sourced_from` edge to the filing stub,
/// statements per period, the three articulation identities
/// executed (per period) over the filed totals.
pub fn extract_filed_series(input: &FiledSeries) -> Result<RaExtraction, ExtractError> {
    let profile = ra_profile()?;
    let mut builder = GraphBuilder::with_profile(profile);
    let model_id = input.model_id.clone();

    builder.add_node(node(model_id.clone(), "model", input.label.clone(), input.label.clone(), 0, None, BTreeMap::new()));
    builder.add_boundary_stub(&input.source_stub, &input.source_label);

    // Statement nodes per (sheet, period): `{model}::{sheet}::{period}`.
    let sheet_labels = [
        ("is", "Income statement"),
        ("bs", "Balance sheet"),
        ("cf", "Cash-flow statement"),
    ];
    for (si, period) in input.periods.iter().enumerate() {
        let period_node = format!("{model_id}::period::{period}");
        builder.add_node(node(period_node.clone(), "period", period.clone(), input.label.clone(), 0, None, BTreeMap::new()));
        builder.add_contains(&model_id, &period_node);
        for (sheet, slabel) in &sheet_labels {
            let sid = format!("{model_id}::{sheet}::{period}");
            builder.add_node(node(
                sid.clone(),
                "statement",
                format!("{slabel} {period}"),
                input.label.clone(),
                1,
                None,
                BTreeMap::new(),
            ));
            builder.add_contains(&model_id, &sid);
        }
        let _ = si;
    }

    // Field cells: `{model}::{field}::{period}`.
    for (field, sheet, label, series) in &input.fields {
        for (pi, period) in input.periods.iter().enumerate() {
            let Some(Some(v)) = series.get(pi) else { continue };
            let id = format!("{model_id}::{field}::{period}");
            let mut measures: BTreeMap<String, Value> = BTreeMap::new();
            measures.insert("value".to_string(), json!(v));
            measures.insert("unit".to_string(), json!(input.unit));
            measures.insert("period".to_string(), json!(period));
            builder.add_node(node(id.clone(), "line_item", label.clone(), input.label.clone(), 0, None, measures));
            builder.add_contains(&format!("{model_id}::{sheet}::{period}"), &id);
            let mut attrs = Map::new();
            attrs.insert("source".to_string(), json!(input.source_name));
            attrs.insert("as_of".to_string(), json!(input.period_ends[pi]));
            builder.add_typed_edge(Edge {
                from: id,
                to: input.source_stub.clone(),
                kind: EdgeKind::new("sourced_from"),
                line: None,
                attrs,
            });
        }
    }

    // articulates edges between the per-period statements:
    // net-income rollforward joins is→bs, cash articulation cf→bs.
    // (The balance-sheet equation has both sides on ONE statement;
    // statement-to-itself edges are not part of the dictionary's
    // articulation shape — the identity executes below.)
    for period in &input.periods {
        for (identity, from_sheet, to_sheet) in [
            ("net_income_rollforward", "is", "bs"),
            ("cash_articulation", "cf", "bs"),
        ] {
            let mut attrs = Map::new();
            attrs.insert("identity".to_string(), json!(identity));
            attrs.insert("tolerance".to_string(), json!(0.01));
            builder.add_typed_edge(Edge {
                from: format!("{model_id}::{from_sheet}::{period}"),
                to: format!("{model_id}::{to_sheet}::{period}"),
                kind: EdgeKind::new("articulates"),
                line: None,
                attrs,
            });
        }
    }

    let (graph, mut diagnostics) = builder.build();

    // F-2 execution over the filed totals (tolerance 0.01 currency
    // units — filed statements articulate exactly; the tolerance is
    // the fixture's declared rounding allowance).
    let val = |field: &str, period: &str| -> Option<f64> {
        graph
            .get(&format!("{model_id}::{field}::{period}"))
            .and_then(|n| n.measure("value"))
            .and_then(|v| v.as_f64())
    };
    for period in &input.periods {
        // balance_sheet_equation: assets = liabilities + equity.
        if let (Some(a), Some(l), Some(e)) = (
            val("total_assets", period),
            val("total_liabilities", period),
            val("total_equity", period),
        ) {
            if (a - (l + e)).abs() > 0.01 {
                diagnostics.push(Diagnostic::ArticulationIdentity {
                    identity: "balance_sheet_equation".to_string(),
                    lhs: a,
                    rhs: l + e,
                    tolerance: 0.01,
                });
            }
        }
        // net_income_rollforward: ΔRE = NI − dividends − buybacks.
        if let Some(pi) = input.periods.iter().position(|p| p == period) {
            if pi > 0 {
                let prev = &input.periods[pi - 1];
                if let (Some(re_now), Some(re_prev), Some(ni), Some(div), Some(bb)) = (
                    val("retained_earnings", period),
                    val("retained_earnings", prev),
                    val("net_income", period),
                    val("dividends_paid", period),
                    val("share_buybacks", period),
                ) {
                    let rhs = re_prev + ni - div - bb;
                    if (re_now - rhs).abs() > 0.01 {
                        diagnostics.push(Diagnostic::ArticulationIdentity {
                            identity: "net_income_rollforward".to_string(),
                            lhs: re_now,
                            rhs,
                            tolerance: 0.01,
                        });
                    }
                }
            }
        }
        // cash_articulation: closing cash = opening cash + OCF +
        // investing + financing (the filed section totals).
        if let Some(pi) = input.periods.iter().position(|p| p == period) {
            if pi > 0 {
                let prev = &input.periods[pi - 1];
                if let (Some(close), Some(open), Some(ocf), Some(inv), Some(fin)) = (
                    val("cash", period),
                    val("cash", prev),
                    val("operating_cash_flow", period),
                    val("investing_cash_flow", period),
                    val("financing_cash_flow", period),
                ) {
                    let rhs = open + ocf + inv + fin;
                    if (close - rhs).abs() > 0.01 {
                        diagnostics.push(Diagnostic::ArticulationIdentity {
                            identity: "cash_articulation".to_string(),
                            lhs: close,
                            rhs,
                            tolerance: 0.01,
                        });
                    }
                }
            }
        }
    }

    Ok(RaExtraction {
        graph,
        diagnostics,
        scenarios: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Per-edge provenance (format gap F-1's pinned resolution, executable).
// ---------------------------------------------------------------------------

/// The provenance class of ONE edge under the profile, applying the
/// RA profile's per-edge rule for `references`: an edge carrying a
/// `claimant` attr is a declared claim; a `references` edge without
/// one is a found reference — extracted truth. Every other kind
/// answers its kind's declared class. (The snapshot's graph class
/// counts KIND classes, the engine's two-value §4.3 vocabulary; this
/// function is the per-edge answer an agent reports when its walk
/// crosses the edge — spec §4.3's "reports provenance_class per
/// edge".)
pub fn edge_provenance_class(profile: &DomainProfile, edge: &Edge) -> Option<ProvenanceClass> {
    if edge.kind == EdgeKind::new("references") {
        return Some(if edge.attrs.contains_key("claimant") {
            ProvenanceClass::DeclaredClaim
        } else {
            ProvenanceClass::ExtractedTruth
        });
    }
    profile.provenance_class(&edge.kind)
}
