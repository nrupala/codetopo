// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Change-history measurements — Phase B, part 4: hotspots and change
//! coupling (CODE_SCHEMA §9). History is a **local-only** input
//! (ruling B0.1): the core computation is a pure function over
//! transactions the caller supplies (a transaction is the set of
//! files one commit touched, already mapped to graph file paths).
//! Core never shells out to git and never invents history: a surface
//! with no history to supply answers with an unmeasured report —
//! `measured: false` and the reason — never zeros.
//!
//! The two measurements, per CODE_SCHEMA §9:
//!
//! - **Hotspots** (Tornhill, *Your Code as a Crime Scene*, 2015): per
//!   file, the change frequency over the stated window and the file's
//!   total complexity (the sum of its functions' cyclomatic
//!   complexities, B2) — both factors reported, and their product.
//!   A file whose callables were not all measured carries no
//!   complexity factor and no product.
//! - **Change coupling** (Zimmermann et al., 2005): per file pair,
//!   support (transactions containing both) and confidence
//!   (support / transactions containing either) — reported as the
//!   pair counts themselves, never collapsed to a bare score. Pairs
//!   below the caller's minimum support are not reported.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::graph::Graph;
use crate::schema::NodeKind;

/// The published method of every history measurement, keyed by field
/// name; the surfaces state these beside the numbers.
pub const HISTORY_METHODS: &[(&str, &str)] = &[
    (
        "hotspot",
        "hotspot (Tornhill 2015): per file, changes = the number of \
         transactions (commits, merge commits excluded) in the stated \
         window that touched the file; complexity = the sum of the \
         cyclomatic complexities of the file's functions and methods, \
         absent unless every one was measured; the reported score is \
         changes x complexity, with both factors always shown",
    ),
    (
        "change_coupling",
        "change coupling (Zimmermann et al. 2005): per file pair, \
         support = transactions containing both files; confidence = \
         support / transactions containing either file; pairs below the \
         stated minimum support are not reported",
    ),
];

/// One file's hotspot measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileHotspot {
    pub file: String,
    /// Transactions in the window that touched the file.
    pub changes: usize,
    /// Sum of the file's callables' complexities; `None` unless every
    /// callable in the file was measured (or the file has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<u32>,
    /// changes × complexity; `None` exactly when complexity is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<u64>,
}

/// One change-coupled file pair, with the counts the confidence
/// stands on — never a bare score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoupledFiles {
    pub a: String,
    pub b: String,
    /// Transactions containing both files.
    pub support: usize,
    /// Transactions containing either file (the confidence denominator).
    pub either: usize,
    /// support / either.
    pub confidence: f64,
}

/// A history report. `measured: false` (with `reason`) is the answer
/// of every surface that has no history to measure — hosted indexes
/// carry no repository history (B0.1), and a tree without recorded
/// history is answered the same way on local surfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryReport {
    pub measured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Where the transactions came from, as the caller states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The window the transactions span, as the caller states it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    /// Transactions measured (after mapping to graph files; a
    /// transaction touching no indexed file does not count).
    pub transactions: usize,
    /// The minimum support the coupling list was filtered by.
    pub min_support: usize,
    /// Hotspots: score descending (unscored files last), then changes
    /// descending, then file ascending.
    pub hotspots: Vec<FileHotspot>,
    /// Coupled pairs: support descending, then (a, b) ascending.
    pub coupling: Vec<CoupledFiles>,
}

impl HistoryReport {
    /// The unmeasured answer: no history, stated reason, no numbers.
    pub fn not_measured(reason: &str, min_support: usize) -> Self {
        HistoryReport {
            measured: false,
            reason: Some(reason.to_string()),
            source: None,
            window: None,
            transactions: 0,
            min_support,
            hotspots: Vec::new(),
            coupling: Vec::new(),
        }
    }
}

/// The history input a local surface supplies: where the transactions
/// came from, the window they span, and the transactions themselves —
/// each a set of graph file paths (the caller maps repository paths
/// to the graph's file identity; see the CLI's history command).
#[derive(Debug, Clone)]
pub struct HistoryInput {
    pub source: String,
    pub window: String,
    pub transactions: Vec<Vec<String>>,
}

/// Compute the history measurements of `graph` over `input`'s
/// transactions. Files are the graph's file identity (the `loc.file`
/// of its symbols). Transactions naming files the graph does not know
/// keep only their known files; a transaction left empty does not
/// count toward the total.
pub fn graph_history(graph: &Graph, input: &HistoryInput, min_support: usize) -> HistoryReport {
    // Per-file complexity: the sum over the file's functions/methods,
    // present only when the file has at least one callable and every
    // callable was measured.
    let mut file_complexity: BTreeMap<&str, Option<u32>> = BTreeMap::new();
    for n in graph.nodes() {
        if !matches!(n.kind, NodeKind::Function | NodeKind::Method) || n.loc.file.is_empty() {
            continue;
        }
        let entry = file_complexity.entry(n.loc.file.as_str()).or_insert(Some(0));
        match (*entry, n.complexity()) {
            (Some(sum), Some(v)) => *entry = Some(sum + v),
            _ => *entry = None,
        }
    }

    // Change counts per file over the mapped transactions.
    let known_files: std::collections::HashSet<&str> =
        graph.nodes().map(|n| n.loc.file.as_str()).collect();
    let mut changes: BTreeMap<&str, usize> = BTreeMap::new();
    let mut pair_support: HashMap<(&str, &str), usize> = HashMap::new();
    let mut file_transactions: BTreeMap<&str, usize> = BTreeMap::new();
    let mut counted = 0usize;
    for tx in &input.transactions {
        let files: BTreeSet<&str> = tx
            .iter()
            .map(|f| f.as_str())
            .filter(|f| known_files.contains(f))
            .collect();
        if files.is_empty() {
            continue;
        }
        counted += 1;
        for f in &files {
            *changes.entry(*f).or_insert(0) += 1;
            *file_transactions.entry(*f).or_insert(0) += 1;
        }
        let list: Vec<&str> = files.iter().copied().collect();
        for i in 0..list.len() {
            for j in (i + 1)..list.len() {
                *pair_support.entry((list[i], list[j])).or_insert(0) += 1;
            }
        }
    }

    let mut hotspots: Vec<FileHotspot> = changes
        .iter()
        .map(|(file, ch)| {
            let complexity = file_complexity.get(file).copied().flatten();
            FileHotspot {
                file: file.to_string(),
                changes: *ch,
                complexity,
                score: complexity.map(|c| *ch as u64 * c as u64),
            }
        })
        .collect();
    hotspots.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.changes.cmp(&a.changes))
            .then_with(|| a.file.cmp(&b.file))
    });

    let mut coupling: Vec<CoupledFiles> = pair_support
        .iter()
        .filter(|(_, support)| **support >= min_support.max(1))
        .map(|((a, b), support)| {
            // Union count: transactions containing a, plus those
            // containing b, minus those containing both.
            let either = file_transactions.get(a).copied().unwrap_or(0)
                + file_transactions.get(b).copied().unwrap_or(0)
                - support;
            CoupledFiles {
                a: a.to_string(),
                b: b.to_string(),
                support: *support,
                either,
                confidence: *support as f64 / either as f64,
            }
        })
        .collect();
    coupling.sort_by(|x, y| {
        y.support
            .cmp(&x.support)
            .then_with(|| x.a.cmp(&y.a))
            .then_with(|| x.b.cmp(&y.b))
    });

    HistoryReport {
        measured: true,
        reason: None,
        source: Some(input.source.clone()),
        window: Some(input.window.clone()),
        transactions: counted,
        min_support,
        hotspots,
        coupling,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Loc, Node, Tier};
    use crate::snapshot::{Snapshot, SCHEMA_VERSION};

    fn node(id: &str, kind: NodeKind, file: &str, decisions: Option<u32>) -> Node {
        Node {
            id: id.to_string(),
            kind,
            label: id.rsplit("::").next().unwrap_or(id).to_string(),
            loc: Loc {
                file: file.to_string(),
                line: 1,
                column: None,
            },
            tier: Tier::Optional,
            defines: Vec::new(),
            refs: Vec::new(),
            end_line: Some(5),
            signature: None,
            decisions,
        }
    }

    /// Files: f1.rs (functions of v 3 and v 2 — file complexity 5),
    /// f2.rs (one function, v 4), f3.rs (one unmeasured function).
    fn fixture() -> Graph {
        Snapshot {
            schema_version: SCHEMA_VERSION.to_string(),
            nodes: vec![
                node("p::f1::a", NodeKind::Function, "f1.rs", Some(2)),
                node("p::f1::b", NodeKind::Function, "f1.rs", Some(1)),
                node("p::f2::c", NodeKind::Function, "f2.rs", Some(3)),
                node("p::f3::d", NodeKind::Function, "f3.rs", None),
            ],
            edges: Vec::new(),
            audit_head: String::new(),
            audit_log: Vec::new(),
        }
        .to_graph()
    }

    fn input(transactions: Vec<Vec<&str>>) -> HistoryInput {
        HistoryInput {
            source: "synthetic fixture history".to_string(),
            window: "entire recorded history".to_string(),
            transactions: transactions
                .into_iter()
                .map(|tx| tx.into_iter().map(|s| s.to_string()).collect())
                .collect(),
        }
    }

    /// The planted history: (f1, f2) co-change in 3 of the 5
    /// transactions; (f2, f3) co-change exactly once — below the
    /// default minimum support of 2, so the pair must NOT be reported.
    #[test]
    fn hotspots_and_coupling_hand_verified() {
        let g = fixture();
        let report = graph_history(
            &g,
            &input(vec![
                vec!["f1.rs", "f2.rs"],
                vec!["f1.rs", "f2.rs"],
                vec!["f1.rs"],
                vec!["f2.rs", "f3.rs"],
                vec!["f1.rs", "f2.rs"],
            ]),
            2,
        );
        assert!(report.measured);
        assert_eq!(report.transactions, 5);

        assert_eq!(report.coupling.len(), 1, "only the planted pair: {report:?}");
        let pair = &report.coupling[0];
        assert_eq!((pair.a.as_str(), pair.b.as_str()), ("f1.rs", "f2.rs"));
        assert_eq!(pair.support, 3, "transactions containing both");
        assert_eq!(pair.either, 5, "every transaction touches f1 or f2");
        assert!((pair.confidence - 0.6).abs() < 1e-9, "3/5");

        // f1: 4 changes x complexity 5 = 20. f2: 4 x 4 = 16.
        // f3: 1 change, complexity absent (unmeasured function).
        assert_eq!(report.hotspots.len(), 3);
        let h = &report.hotspots[0];
        assert_eq!((h.file.as_str(), h.changes, h.complexity, h.score), ("f1.rs", 4, Some(5), Some(20)));
        let h = &report.hotspots[1];
        assert_eq!((h.file.as_str(), h.changes, h.complexity, h.score), ("f2.rs", 4, Some(4), Some(16)));
        let h = &report.hotspots[2];
        assert_eq!(
            (h.file.as_str(), h.changes, h.complexity, h.score),
            ("f3.rs", 1, None, None),
            "unmeasured complexity: no factor, no product — not zero"
        );
    }

    #[test]
    fn unknown_files_and_empty_transactions_do_not_count() {
        let g = fixture();
        let report = graph_history(
            &g,
            &input(vec![vec!["gone.rs"], vec!["f1.rs", "gone.rs"], vec![]]),
            2,
        );
        assert_eq!(report.transactions, 1, "only the transaction touching a known file");
        assert_eq!(report.hotspots.len(), 1);
        assert_eq!(report.hotspots[0].changes, 1);
        assert!(report.coupling.is_empty());
    }

    #[test]
    fn not_measured_report_carries_the_reason_and_no_numbers() {
        let report = HistoryReport::not_measured("no history on this surface", 2);
        assert!(!report.measured);
        assert_eq!(report.reason.as_deref(), Some("no history on this surface"));
        assert!(report.hotspots.is_empty() && report.coupling.is_empty());
        let json = serde_json::to_string(&report).expect("serializes");
        assert!(!json.contains("\"changes\""), "no invented numbers: {json}");
    }
}
