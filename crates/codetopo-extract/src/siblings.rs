// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The RA profile's two registered statistics siblings, executed.
//!
//! Both siblings are measurements of the MODEL's own arithmetic —
//! the re-evaluation is the workbook's own formulas under a changed
//! binding. No regression, no fitted surface, no external data.
//!
//! - `research-analyst.sensitivity`: for an (output, driver) pair,
//!   perturb the driver by a declared step and report the finite
//!   difference Δoutput/Δdriver and the elasticity. The step rule is
//!   the profile's: ±1% of the driver's measured value by default,
//!   or one declared unit where the driver is a rate (a unitless
//!   driver whose |value| ≤ 1 — a rate or ratio in this dictionary).
//!   A pair whose computes_into path crosses an estimates edge is
//!   reported with its provenance class named; an unmeasured pair is
//!   `not_measured`, never interpolated.
//! - `research-analyst.scenario_spread`: for each output, its
//!   measured value under every scenario snapshot, the range, and
//!   each scenario's signed deviation from the base scenario. An
//!   output unvalued under a scenario is named `not_measured` for
//!   that scenario; the spread is computed over the measured ones
//!   and says which. Never a confidence interval, never a target.
//!
//! The siblings re-evaluate through the same declaration + evaluator
//! the extractor uses, so a sibling number is the model's arithmetic
//! by construction, and its oracle is hand arithmetic over the
//! fixture's printed formulas (RA Regression Contract C-RA4).

use std::collections::BTreeMap;

use codetopo_core::graph::Graph;
use codetopo_core::profile::ProvenanceClass;

use crate::eval::EvalBook;
use crate::ra::{edge_provenance_class, extract_from_sheets_pub, WorkbookDecl};
use crate::xlsx::Sheet;
use crate::ExtractError;

/// One sensitivity measurement for an (output, driver) pair.
#[derive(Debug, Clone, PartialEq)]
pub struct SensitivityEntry {
    /// The output cell id.
    pub output: String,
    /// The driver cell id.
    pub driver: String,
    /// The step applied (+ direction): driver_perturbed − driver_base.
    pub step: f64,
    /// Δoutput/Δdriver at the step; `None` = not_measured.
    pub finite_difference: Option<f64>,
    /// (Δoutput/output)/(Δdriver/driver); `None` = not_measured.
    pub elasticity: Option<f64>,
    /// `not_measured` when either side has no value under the step.
    pub not_measured: bool,
    /// The provenance class the pair's path reports, when the path
    /// crosses a non-truth edge (else `None` — the pair is pure
    /// extracted truth).
    pub provenance_note: Option<ProvenanceClass>,
}

/// Split a node id `{model}::{sheet}::{Ref}` into (sheet, Ref).
fn id_sheet_ref(id: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = id.split("::").collect();
    if parts.len() == 3 {
        Some((parts[1].to_string(), parts[2].to_string()))
    } else {
        None
    }
}

/// Does a directed computes_into path run from `from` to `to`
/// (reader → read direction: the path a perturbation TRAVELS is the
/// reverse — a driver change reaches the cells that read it, which
/// are found by walking computes_into edges BACKWARD from the
/// driver). Returns the path's edges' provenance classes when a
/// path exists.
fn path_classes(
    graph: &Graph,
    driver: &str,
    output: &str,
) -> Option<Vec<ProvenanceClass>> {
    // Walk incoming computes_into edges from the output back to the
    // driver (output reads ... reads driver).
    let profile = graph.profile().clone();
    let mut visited: BTreeMap<String, Vec<ProvenanceClass>> = BTreeMap::new();
    let mut queue: Vec<(String, Vec<ProvenanceClass>)> = vec![(output.to_string(), Vec::new())];
    visited.insert(output.to_string(), Vec::new());
    while let Some((node, classes)) = queue.pop() {
        if node == driver {
            return Some(classes);
        }
        for edge in graph.edges() {
            if edge.to == node
                && (edge.kind.as_str() == "computes_into" || edge.kind.as_str() == "estimates")
                && !visited.contains_key(&edge.from)
            {
                let mut next = classes.clone();
                if let Some(c) = edge_provenance_class(&profile, edge) {
                    next.push(c);
                }
                visited.insert(edge.from.clone(), next.clone());
                queue.push((edge.from.clone(), next));
            }
        }
    }
    None
}

/// Run the sensitivity sibling over an extracted workbook for the
/// given (output cell id, driver cell id) pairs.
///
/// `sheets`/`decl` are the extraction inputs (the sibling re-runs
/// the extractor with the driver overridden); `base` is the base
/// extraction's graph (for base values and path provenance).
pub fn sensitivity(
    sheets: &[Sheet],
    decl: &WorkbookDecl,
    base: &Graph,
    pairs: &[(&str, &str)],
) -> Result<Vec<SensitivityEntry>, ExtractError> {
    let mut out = Vec::new();
    for (output, driver) in pairs {
        let base_output = base
            .get(output)
            .and_then(|n| n.measure("value"))
            .and_then(|v| v.as_f64());
        let base_driver = base
            .get(driver)
            .and_then(|n| n.measure("value"))
            .and_then(|v| v.as_f64());
        let classes = path_classes(base, driver, output);
        let provenance_note = classes.as_ref().and_then(|cs| {
            cs.iter()
                .find(|c| {
                    !matches!(c, ProvenanceClass::ExtractedTruth | ProvenanceClass::SourcedFact)
                })
                .copied()
        });
        let (Some(out_v), Some(drv_v)) = (base_output, base_driver) else {
            out.push(SensitivityEntry {
                output: output.to_string(),
                driver: driver.to_string(),
                step: 0.0,
                finite_difference: None,
                elasticity: None,
                not_measured: true,
                provenance_note,
            });
            continue;
        };
        // The profile's step rule: ±1% of the measured value; one
        // declared unit where the driver is a rate (|value| ≤ 1 and
        // unitless in this dictionary reads as a rate/ratio).
        let is_rate = drv_v.abs() <= 1.0;
        let step = if is_rate { 0.01 } else { drv_v * 0.01 };
        let (dsheet, dref) = id_sheet_ref(driver)
            .ok_or_else(|| ExtractError::Declaration(format!("bad driver id {driver}")))?;
        let mut overrides: BTreeMap<(String, String), f64> = BTreeMap::new();
        overrides.insert((dsheet, dref), drv_v + step);
        let book = EvalBook::from_sheets(sheets).with_overrides(overrides);
        let extraction = extract_from_sheets_pub(sheets, decl, &book)?;
        let perturbed = extraction
            .graph
            .get(output)
            .and_then(|n| n.measure("value"))
            .and_then(|v| v.as_f64());
        match perturbed {
            Some(p) => {
                let fd = (p - out_v) / step;
                let elasticity = if out_v != 0.0 && drv_v != 0.0 {
                    Some(((p - out_v) / out_v) / (step / drv_v))
                } else {
                    None
                };
                out.push(SensitivityEntry {
                    output: output.to_string(),
                    driver: driver.to_string(),
                    step,
                    finite_difference: Some(fd),
                    elasticity,
                    not_measured: false,
                    provenance_note,
                });
            }
            None => out.push(SensitivityEntry {
                output: output.to_string(),
                driver: driver.to_string(),
                step,
                finite_difference: None,
                elasticity: None,
                not_measured: true,
                provenance_note,
            }),
        }
    }
    Ok(out)
}

/// One output's scenario spread.
#[derive(Debug, Clone, PartialEq)]
pub struct ScenarioSpreadEntry {
    /// The output cell id.
    pub output: String,
    /// Per-scenario measured values; a scenario missing here is in
    /// `not_measured`.
    pub values: BTreeMap<String, f64>,
    /// Scenarios under which the output has no value.
    pub not_measured: Vec<String>,
    /// max − min over the measured scenarios; `None` when fewer
    /// than two scenarios measured the output.
    pub range: Option<f64>,
    /// Signed deviation from the base scenario, per measured
    /// scenario (the base's own deviation is 0).
    pub deviation_from_base: BTreeMap<String, f64>,
}

/// Run the scenario_spread sibling: `base` is the base graph;
/// `scenarios` are (name, graph) as the extractor produced them;
/// `base_name` names the base scenario inside the same list (its
/// graph IS the base graph's values).
pub fn scenario_spread(
    base: &Graph,
    scenarios: &[(String, Graph)],
    outputs: &[&str],
    base_name: &str,
) -> Vec<ScenarioSpreadEntry> {
    let mut out = Vec::new();
    for output in outputs {
        let mut values: BTreeMap<String, f64> = BTreeMap::new();
        let mut not_measured: Vec<String> = Vec::new();
        for (name, graph) in scenarios {
            let g = if name == base_name { base } else { graph };
            match g
                .get(output)
                .and_then(|n| n.measure("value"))
                .and_then(|v| v.as_f64())
            {
                Some(v) => {
                    values.insert(name.clone(), v);
                }
                None => not_measured.push(name.clone()),
            }
        }
        let range = if values.len() >= 2 {
            let max = values.values().copied().fold(f64::MIN, f64::max);
            let min = values.values().copied().fold(f64::MAX, f64::min);
            Some(max - min)
        } else {
            None
        };
        let mut deviation_from_base = BTreeMap::new();
        if let Some(base_v) = values.get(base_name) {
            for (name, v) in &values {
                deviation_from_base.insert(name.clone(), v - base_v);
            }
        }
        out.push(ScenarioSpreadEntry {
            output: output.to_string(),
            values,
            not_measured,
            range,
            deviation_from_base,
        });
    }
    out
}
