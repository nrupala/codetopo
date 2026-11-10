// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The keyed human ask surface (endpoints audit gap G1).
//!
//! Until this module, a paying customer could discover codetopo, buy a
//! tier, and claim a hosted key entirely unaided — and then met only a
//! Bearer-key JSON API over their own index. There was no page where
//! they could ask a question in plain words and read a rendered answer.
//! This module closes that break, on the demo front door's pattern
//! (`space/index.html`: one self-contained page, live fetches, tables
//! plus the raw JSON in a `<details>` one click below):
//!
//! * `GET /ask` serves the page. It is public like `GET /` — it carries
//!   no data; the visitor types their own hosted key into it.
//! * `POST /v1/graphs/{id}/ask` answers one question. It sits inside
//!   the same metered Bearer wrapper as every other `/v1` route: the
//!   page sends the key the visitor typed as the standard
//!   `Authorization: Bearer <key_id>:<secret>` header. No new auth
//!   system exists here — the key is never stored by the page, never
//!   logged by this handler, and tenancy is enforced exactly as on the
//!   query routes, so a key can only ask about its own indexes.
//!
//! The question router is the ACP agent's deterministic intent router
//! ([`codetopo_acp::classify_intent`] — the same classifier that drives
//! `codetopo-acp` in editors), extended with the remaining chartered
//! measurements the ACP surface does not carry (rank, metrics,
//! communities, topo-sort, diagnostics, search, dossier, neighborhood,
//! render, history, schema). Every answer is *measured*: the data comes
//! from the same `codetopo_core` / `codetopo_cli` computations the CLI,
//! MCP, ACP, and HTTP surfaces share, the summary is a template over
//! those numbers, and the rendered fragment is built from the same
//! data. A question the router cannot map to a chartered function —
//! or an edit request, which this read-only product refuses — is
//! refused plainly, with the questions that *can* be answered named.
//! Nothing here invents content.

use std::collections::BTreeMap;
use std::path::Path;

use axum::extract::{Path as UrlPath, RawQuery, State};
use axum::response::Html;
use axum::{Extension, Json};
use serde::Serialize;
use serde_json::{json, Value};

use codetopo_acp::{classify_intent, Intent, RefusalReply};
use codetopo_cli::{describe_schema, render_path};
use codetopo_core::graph::Graph;
use codetopo_core::schema::{EdgeKind, NodeKind};
use codetopo_store::Store;

use crate::auth::KeyId;
use crate::error::ApiError;
use crate::handlers::{enforce_tenancy, load_graph_blocking, reject_unknown_params, AppState};

/// One routed answer: the machine data, the plain-language summary
/// templated over it, an optional measured SVG drawing, and whether
/// the question was refused.
#[derive(Debug, Clone, Serialize)]
pub struct AskAnswer {
    pub intent: String,
    pub refused: bool,
    pub summary: String,
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub svg: Option<String>,
}

impl AskAnswer {
    fn measured(intent: &str, summary: String, data: Value) -> Self {
        AskAnswer { intent: intent.to_string(), refused: false, summary, data, svg: None }
    }

    fn refused(intent: &str, summary: String) -> Self {
        AskAnswer {
            intent: intent.to_string(),
            refused: true,
            summary,
            data: json!({ "examples": EXAMPLE_QUESTIONS }),
            svg: None,
        }
    }
}

/// The questions the ask page offers as examples — every one of them
/// routes to a chartered function.
pub const EXAMPLE_QUESTIONS: [&str; 6] = [
    "How many nodes and edges are in my index?",
    "What does <symbol> affect?",
    "Who calls <symbol>?",
    "What is the blast radius of <symbol>?",
    "Rank my symbols by blast radius",
    "Draw the graph of my index",
];

const REFUSAL_PREFIX: &str =
    "I can only answer questions about your index, measured from its code graph — \
     I don't invent answers. ";

/// The reason hosted history is always unmeasured — the same words
/// the `/history` route uses (`handlers::HOSTED_HISTORY_REASON`).
const HOSTED_HISTORY_REASON: &str = "hosted indexes carry no repository history; \
     history is measured on local surfaces only (the CLI's history command, \
     or the MCP tool with a local working tree)";

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// Route one plain-words question over a loaded graph to a measured
/// answer (or a plain refusal). Pure over the graph plus the index
/// database path (verify and diagnostics read the store), so the
/// integration tests pin it through the HTTP surface and the page and
/// any future surface share one router.
pub fn route_question(graph: &Graph, question: &str, db_path: Option<&Path>) -> AskAnswer {
    let q = question.trim();
    if q.is_empty() {
        return AskAnswer::refused(
            "unknown",
            format!("{REFUSAL_PREFIX}Type a question about your index first — for example: \"How many nodes and edges are in my index?\""),
        );
    }
    let lower = q.to_lowercase();

    // The ACP router first for the requests it already adjudicates:
    // edit requests are its standing read-only refusal, and a snapshot
    // export is a whole-graph download, not a rendered answer — the
    // API's snapshot route serves it; the ask page refuses plainly.
    match classify_intent(q) {
        Intent::Edit(_) => {
            let refusal = RefusalReply::standard();
            return AskAnswer::refused(
                "edit",
                format!("{} This page can answer: {}.", refusal.refusal, refusal.capabilities.join("; ")),
            );
        }
        Intent::Snapshot => {
            return AskAnswer::refused(
                "snapshot",
                format!("{REFUSAL_PREFIX}A snapshot is a whole-graph export, not a rendered answer — the API's snapshot route (GET /v1/graphs/<your index>/snapshot) or the CLI serves it."),
            );
        }
        _ => {}
    }

    // The plainest stats question — "how many nodes and edges…" —
    // carries none of the ACP router's stats keywords, so it is routed
    // here, before the extended functions.
    if lower.contains("how many") || lower.contains("overview") || lower.contains("statistics") {
        return answer_stats(graph);
    }

    // Extended chartered functions the ACP surface does not carry.
    if lower.contains("render") || lower.contains("draw") || lower.contains("svg") || lower.contains("show me the graph") || lower.contains("show me the map") {
        return answer_render(graph);
    }
    if lower.contains("communit") {
        let data = codetopo_cli::communities_value(graph, 20);
        let total = data["total"].as_u64().unwrap_or(0);
        let summary = match data["modularity"].as_f64() {
            Some(qv) => format!("Your index partitions into {total} communities (modularity Q = {qv:.3}, measured by the Leiden partition)."),
            None => format!("Your index partitions into {total} communities. Modularity is not reported: the graph has no edges to measure it over."),
        };
        return AskAnswer::measured("communities", summary, data);
    }
    if lower.contains("topo") || lower.contains("change order") {
        let data = codetopo_cli::topo_sort_value(graph);
        let summary = format!(
            "The safe change order for your index: {} groups covering {} nodes, {} of them cyclic (a cyclic group must change together).",
            data["total_groups"], data["total_nodes"], data["cyclic_groups"]
        );
        return AskAnswer::measured("topo_sort", summary, data);
    }
    if lower.contains("metric") || lower.contains("coupling") || lower.contains("cohesion") {
        let data = codetopo_cli::metrics_value(graph);
        let types = data["types"].as_array().map(Vec::len).unwrap_or(0);
        let packages = data["packages"].as_array().map(Vec::len).unwrap_or(0);
        return AskAnswer::measured(
            "metrics",
            format!("Measured coupling and cohesion for {types} types and {packages} packages in your index. Each metric's method travels with its numbers in the raw data."),
            data,
        );
    }
    if lower.contains("diagnostic") || lower.contains("problems recorded") {
        return match db_path {
            Some(db) => match codetopo_cli::diagnostics_value(db) {
                Ok(data) => {
                    let total = data["total"].as_u64().unwrap_or(0);
                    AskAnswer::measured(
                        "diagnostics",
                        format!("The index recorded {total} diagnostic(s) when it was built — cycles, multi-definitions, unresolved calls, and the rest, exactly as recorded."),
                        data,
                    )
                }
                Err(err) => AskAnswer::refused("diagnostics", format!("The diagnostics could not be read: {err}")),
            },
            None => AskAnswer::refused("diagnostics", "Diagnostics need the index database, which this surface was not given.".to_string()),
        };
    }
    if lower.contains("rank") || lower.contains("top symbol") || lower.contains("most important") || lower.contains("highest") {
        let metric_name = if lower.contains("pagerank") || lower.contains("page rank") {
            "pagerank"
        } else if lower.contains("fan_in") || lower.contains("fan-in") || lower.contains("fan in") {
            "fan_in"
        } else if lower.contains("fan_out") || lower.contains("fan-out") || lower.contains("fan out") {
            "fan_out"
        } else if lower.contains("betweenness") {
            "betweenness"
        } else if lower.contains("closeness") {
            "closeness"
        } else if lower.contains("eigenvector") {
            "eigenvector"
        } else if lower.contains("katz") {
            "katz"
        } else if lower.contains("complexity") {
            "complexity"
        } else {
            "blast_radius"
        };
        let metric = codetopo_core::RankMetric::parse(metric_name).unwrap_or(codetopo_core::RankMetric::BlastRadius);
        let data = codetopo_cli::rank_value(graph, metric, 20);
        let summary = if data["measured"] == json!(false) {
            format!("{} (metric: {})", data["reason"].as_str().unwrap_or("not measured"), metric_name)
        } else {
            let lead = data["results"].as_array().and_then(|r| r.first()).map(|e| {
                format!(" {} leads.", e["label"].as_str().unwrap_or(""))
            }).unwrap_or_default();
            format!("Top symbols in your index by {} ({} ranked in total).{}", metric_name, data["total"], lead)
        };
        return AskAnswer::measured("rank", summary, data);
    }
    if lower.contains("histor") {
        let data = codetopo_cli::history_value(graph, None, 2, HOSTED_HISTORY_REASON);
        return AskAnswer::measured(
            "history",
            "Change history is not measured for hosted indexes — absence stated, never zeros invented. The raw data carries the reason.".to_string(),
            data,
        );
    }
    if lower.contains("schema") {
        let data = describe_schema();
        let version = data["schema_version"].as_str().unwrap_or("the published version").to_string();
        return AskAnswer::measured(
            "schema",
            format!("Your index answers under CODE_SCHEMA {version} — the node kinds, edge kinds, and tiers in the raw data are the whole contract."),
            data,
        );
    }
    if lower.starts_with("search") || lower.contains("find symbol") || lower.contains("look up") {
        let term = lower
            .split("search for").nth(1)
            .or_else(|| lower.split("search").nth(1))
            .or_else(|| lower.split("look up").nth(1))
            .unwrap_or("")
            .trim()
            .trim_matches(|c| matches!(c, '?' | '.' | '"'))
            .to_string();
        if term.is_empty() {
            return AskAnswer::refused("search", format!("{REFUSAL_PREFIX}Name what to search for — for example: \"search for entry\""));
        }
        let hits = graph.search_symbols(&term, 20);
        let results: Vec<Value> = hits.iter().map(|n| serde_json::to_value(n).unwrap_or(Value::Null)).collect();
        let total = results.len();
        return AskAnswer::measured(
            "search",
            format!("Found {total} symbol(s) matching '{term}' in your index."),
            json!({ "query": term, "total": total, "results": results }),
        );
    }
    if lower.contains("neighborhood") || lower.contains("neighbours of") || lower.contains("neighbors of") {
        let hint = lower.rsplit(" of ").next().unwrap_or("").to_string();
        return match resolve_from_question(graph, q, &hint) {
            Some(id) => {
                let sub = graph.neighborhood(&id, 1);
                let (nn, ne) = (sub.nodes.len(), sub.edges.len());
                let data = json!({ "node": id, "depth": 1, "nodes": sub.nodes, "edges": sub.edges });
                AskAnswer::measured("neighborhood", format!("The neighborhood of {id}: {nn} nodes and {ne} edges within one hop, both directions."), data)
            }
            None => not_found("neighborhood", q),
        };
    }
    if lower.contains("dossier") || lower.contains("describe") || lower.contains("tell me about") {
        let hint = lower
            .split("describe").nth(1)
            .or_else(|| lower.split("tell me about").nth(1))
            .or_else(|| lower.split("dossier for").nth(1))
            .or_else(|| lower.split("dossier of").nth(1))
            .unwrap_or("")
            .trim()
            .to_string();
        return match resolve_from_question(graph, q, &hint) {
            Some(id) => match graph.dossier(&id) {
                Some(dossier) => {
                    let mut data = serde_json::to_value(&dossier).unwrap_or(Value::Null);
                    data["node_id"] = json!(id);
                    AskAnswer::measured("symbol", format!("The measured dossier for {id}: the symbol, its incident edges grouped by kind and direction, and their counts."), data)
                }
                None => not_found("symbol", q),
            },
            None => not_found("symbol", q),
        };
    }

    // The ACP router's own intents, executed against the same core
    // walks the HTTP closure routes serve.
    match classify_intent(q) {
        Intent::Descendants(token) => closure_answer(graph, q, &token, "descendants"),
        Intent::Ancestors(token) => closure_answer(graph, q, &token, "ancestors"),
        Intent::BlastRadius(token) => closure_answer(graph, q, &token, "blast_radius"),
        Intent::Path(a, b) => {
            let (from, to) = (resolve_from_question(graph, q, &a), resolve_node(graph, &b).or_else(|| resolve_from_question(graph, q, &b)));
            match (from, to) {
                (Some(from), Some(to)) => match graph.path(&from, &to) {
                    Some(steps) => {
                        let rendered = render_path(&steps);
                        let data = json!({ "from": from, "to": to, "steps": steps, "rendered": rendered });
                        AskAnswer::measured("path", format!("The shortest typed path from {from} to {to} is {} step(s): {rendered}", steps.len()), data)
                    }
                    None => AskAnswer::measured("path", format!("No typed path connects {from} and {to} in your index."), json!({ "from": from, "to": to, "steps": [] })),
                },
                _ => not_found("path", q),
            }
        }
        Intent::Stats => answer_stats(graph),
        Intent::Verify => match db_path {
            Some(db) => match Store::open(db).and_then(|store| store.load_audit().map(|audit| (audit.len(), audit.verify(), audit.head().to_string()))) {
                Ok((entries, valid, head)) => AskAnswer::measured(
                    "verify",
                    format!("Audit chain: {} after {entries} entries (head {}…).", if valid { "valid" } else { "INVALID" }, head.chars().take(16).collect::<String>()),
                    json!({ "entries": entries, "chain_valid": valid, "head": head }),
                ),
                Err(err) => AskAnswer::refused("verify", format!("The audit chain could not be read: {err}")),
            },
            None => AskAnswer::refused("verify", "Verification needs the index database, which this surface was not given.".to_string()),
        },
        Intent::Edit(_) | Intent::Snapshot => unreachable!("handled above"),
        Intent::Unknown => AskAnswer::refused(
            "unknown",
            format!("{REFUSAL_PREFIX}Ask about your index instead — for example: \"{}\"", EXAMPLE_QUESTIONS[0]),
        ),
    }
}

fn not_found(intent: &str, question: &str) -> AskAnswer {
    AskAnswer {
        intent: intent.to_string(),
        refused: true,
        summary: format!("I could not find the symbol named in \"{question}\" in this index. Search first — ask \"search for <name>\" — then ask about the symbol the search returns."),
        data: json!({ "examples": EXAMPLE_QUESTIONS }),
        svg: None,
    }
}

fn answer_stats(graph: &Graph) -> AskAnswer {
    let mut node_kinds: BTreeMap<String, usize> = BTreeMap::new();
    for node in graph.nodes() {
        *node_kinds.entry(node_kind_name(node.kind).to_string()).or_default() += 1;
    }
    let mut edge_kinds: BTreeMap<String, usize> = BTreeMap::new();
    for kind in ALL_EDGE_KINDS {
        let count = graph.edge_count_by_kind(kind);
        if count > 0 {
            edge_kinds.insert(kind.as_str().to_string(), count);
        }
    }
    let (nodes, edges) = (graph.node_count(), graph.edge_count());
    AskAnswer::measured(
        "stats",
        format!("Your index holds {nodes} nodes and {edges} edges, measured from its stored graph."),
        json!({
            "nodes": { "total": nodes, "by_kind": node_kinds },
            "edges": { "total": edges, "by_kind": edge_kinds },
        }),
    )
}

fn answer_render(graph: &Graph) -> AskAnswer {
    let mut answer = AskAnswer::measured(
        "render",
        format!("Here is the measured map of your index: {} nodes and {} edges, laid out by the deterministic layered renderer.", graph.node_count(), graph.edge_count()),
        codetopo_cli::render_value(graph, "layered").unwrap_or_else(|err| json!({ "rendered": false, "reason": err })),
    );
    answer.svg = Some(codetopo_core::render_svg(&codetopo_core::layout(graph)));
    answer
}

fn closure_answer(graph: &Graph, question: &str, token: &str, relation: &str) -> AskAnswer {
    match resolve_from_question(graph, question, token) {
        Some(id) => {
            let ids = match relation {
                "ancestors" => graph.ancestors(&id),
                "blast_radius" => graph.blast_radius(&id),
                _ => graph.descendants(&id),
            };
            let total = ids.len();
            let summary = match relation {
                "ancestors" => format!("{total} node(s) depend on {id} (its ancestors in the call/import graph)."),
                "blast_radius" => format!("If {id} changed, {total} node(s) are in its measured blast radius."),
                _ => format!("{id} affects {total} node(s) (its descendants in the call/import graph)."),
            };
            AskAnswer::measured(relation, summary, json!({ "relation": relation, "node": id, "total": total, "nodes": ids }))
        }
        None => not_found(relation, question),
    }
}

/// Resolve a plain-words name hint to a node id: an exact id wins,
/// otherwise the graph's own symbol search (exact label first).
fn resolve_node(graph: &Graph, hint: &str) -> Option<String> {
    let hint = hint.trim().trim_matches(|c| matches!(c, '?' | '.' | ',' | '"' | '\'' | ':'));
    if hint.is_empty() || hint.eq_ignore_ascii_case("unknown") {
        return None;
    }
    if graph.nodes().any(|n| n.id == hint) {
        return Some(hint.to_string());
    }
    graph.search_symbols(hint, 20).first().map(|n| n.id.clone())
}

/// Resolve using the router's hint first, then the question's words
/// from the end (the symbol is usually named last: "who calls X?").
fn resolve_from_question(graph: &Graph, question: &str, hint: &str) -> Option<String> {
    if let Some(id) = resolve_node(graph, hint) {
        return Some(id);
    }
    for word in question.split_whitespace().rev() {
        if let Some(id) = resolve_node(graph, word) {
            return Some(id);
        }
    }
    None
}

const ALL_EDGE_KINDS: [EdgeKind; 9] = [
    EdgeKind::Imports, EdgeKind::Calls, EdgeKind::Inherits, EdgeKind::Implements,
    EdgeKind::Defines, EdgeKind::References, EdgeKind::Contains, EdgeKind::Reads, EdgeKind::Writes,
];

fn node_kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Package => "package",
        NodeKind::Module => "module",
        NodeKind::File => "file",
        NodeKind::Class => "class",
        NodeKind::Interface => "interface",
        NodeKind::Function => "function",
        NodeKind::Method => "method",
        NodeKind::Variable => "variable",
        NodeKind::External => "external",
        NodeKind::Doc => "doc",
    }
}

// ---------------------------------------------------------------------------
// Rendered fragment (the page inserts this; the raw JSON sits below it)
// ---------------------------------------------------------------------------

/// HTML-escape a measured string for the fragment.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => esc(s),
        Value::Null => "—".to_string(),
        other => esc(&other.to_string()),
    }
}

fn object_table(rows: &[Value]) -> String {
    let Some(first) = rows.first().and_then(Value::as_object) else { return String::new() };
    let cols: Vec<String> = first.keys().take(6).cloned().collect();
    let mut out = String::from("<table><tr>");
    for c in &cols {
        out += &format!("<th>{}</th>", esc(c));
    }
    out += "</tr>";
    for row in rows.iter().take(50) {
        out += "<tr>";
        for c in &cols {
            out += &format!("<td>{}</td>", scalar(row.get(c).unwrap_or(&Value::Null)));
        }
        out += "</tr>";
    }
    out + "</table>"
}

/// Build the rendered fragment for an answer: the plain-language
/// summary, the measured SVG when there is one, and a table shaped
/// from the answer data. Deterministic in the data, like every other
/// codetopo rendering.
pub fn render_fragment(answer: &AskAnswer) -> String {
    let mut out = format!("<p class=\"summary\">{}</p>", esc(&answer.summary));
    if let Some(svg) = &answer.svg {
        out += &format!("<div class=\"graph\">{svg}</div>");
    }
    let data = &answer.data;
    if let (Some(nodes), Some(edges)) = (data.get("nodes"), data.get("edges")) {
        if nodes.get("by_kind").is_some() {
            for (label, section) in [("Nodes by kind", nodes), ("Edges by kind", edges)] {
                if let Some(by_kind) = section.get("by_kind").and_then(Value::as_object) {
                    out += &format!("<h3>{label}</h3><table><tr><th>kind</th><th>count</th></tr>");
                    for (k, v) in by_kind {
                        out += &format!("<tr><td>{}</td><td>{}</td></tr>", esc(k), scalar(v));
                    }
                    out += "</table>";
                }
            }
            return out;
        }
    }
    for key in ["nodes", "results", "steps", "groups", "types", "packages", "communities", "diagnostics"] {
        if let Some(rows) = data.get(key).and_then(Value::as_array) {
            if rows.first().map(Value::is_string).unwrap_or(false) {
                out += "<table><tr><th>node</th></tr>";
                for r in rows.iter().take(50) {
                    out += &format!("<tr><td>{}</td></tr>", scalar(r));
                }
                out += "</table>";
            } else if !rows.is_empty() {
                out += &object_table(rows);
            }
            return out;
        }
    }
    if let Some(obj) = data.as_object() {
        out += "<table><tr><th>fact</th><th>value</th></tr>";
        for (k, v) in obj {
            let cell = match v {
                Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join(", "),
                Value::Object(_) => esc(&v.to_string()),
                _ => scalar(v),
            };
            out += &format!("<tr><td>{}</td><td>{}</td></tr>", esc(k), cell);
        }
        out += "</table>";
    }
    out
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

/// Body of `POST /v1/graphs/{id}/ask`, read as raw JSON for the same
/// reason `POST /v1/index` reads its body itself: a malformed body is
/// answered in the API's own error vocabulary.
#[derive(Debug, Default, serde::Deserialize)]
struct AskRequest {
    question: Option<String>,
}

/// `POST /v1/graphs/{id}/ask` — one plain-words question over the
/// caller's own index, answered with the measured data, a
/// plain-language summary, a rendered HTML fragment (tables + SVG),
/// and the SVG itself when the question asked for a drawing. The raw
/// answer *is* the JSON response; the page shows it one click below
/// the rendering, as the demo does.
pub async fn ask(
    State(state): State<AppState>,
    Extension(key): Extension<KeyId>,
    UrlPath(index_id): UrlPath<String>,
    RawQuery(raw_query): RawQuery,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    // The ask route declares no query parameters (its input is the
    // JSON body); the generalized refusal covers it like every
    // other /v1 route, so a stray parameter is refused by name
    // rather than silently ignored.
    reject_unknown_params(raw_query.as_deref(), "/ask", &[])?;
    let parsed: AskRequest = serde_json::from_slice(&body)
        .map_err(|err| ApiError::invalid_body(&format!("not valid JSON for this route: {err}")))?;
    let question = parsed
        .question
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| ApiError::missing_param("question"))?
        .to_string();
    enforce_tenancy(&state, &index_id, &key)?;
    let db = state.db_path(&index_id);
    let graph = load_graph_blocking(&db, &index_id)?;
    let answer = route_question(&graph, &question, Some(&db));
    let html = render_fragment(&answer);
    Ok(Json(json!({
        "index_id": index_id,
        "question": question,
        "intent": answer.intent,
        "refused": answer.refused,
        "summary": answer.summary,
        "html": html,
        "svg": answer.svg,
        "data": answer.data,
    })))
}

/// `GET /ask` — the ask page itself (see the module docs).
pub async fn ask_page() -> Html<&'static str> {
    Html(ASK_HTML)
}

/// The ask page, modelled on the demo front door (`space/index.html`):
/// the same dark monospace styling, the same live-fetch shape, the
/// same raw-JSON `<details>` under every rendered answer. One
/// self-contained document, no external assets. The key the visitor
/// types is used only as the Bearer header of the ask request, from
/// their browser to this API — it is never stored or sent anywhere
/// else.
const ASK_HTML: &str = r#"<!DOCTYPE html>
<!--
  Copyright (C) 2026 2351485 ALBERTA INC.
  SPDX-License-Identifier: AGPL-3.0-or-later
  Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).
  Ask your index: the keyed human ask surface of the hosted codetopo API.
  Every answer on this page is measured by the engine from the visitor's
  own index; the page invents nothing, and questions outside the
  chartered functions are refused plainly.
-->
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>codetopo — ask your index</title>
<style>
  :root { color-scheme: dark; }
  body { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
         margin: 0 auto; max-width: 880px; padding: 24px 16px 48px;
         background: #0d1117; color: #e6edf3; line-height: 1.5; }
  h1 { font-size: 28px; margin: 0 0 4px; }
  h2 { font-size: 17px; margin: 28px 0 8px; }
  .sub { color: #9da7b3; margin: 0 0 16px; }
  .honest { border: 1px solid #2f81f7; border-radius: 8px; padding: 10px 14px;
            background: #0f1d33; font-size: 14px; }
  label { display: block; margin: 14px 0 4px; font-size: 14px; color: #9da7b3; }
  input[type="text"], input[type="password"] { width: 100%; box-sizing: border-box; padding: 10px;
    font: inherit; background: #161b22; color: inherit; border: 1px solid #30363d; border-radius: 8px; }
  button { font: inherit; padding: 8px 14px; border-radius: 8px; border: 1px solid #30363d;
           background: #21262d; color: inherit; cursor: pointer; }
  .examples button { margin: 0 6px 6px 0; font-size: 13px; }
  table { border-collapse: collapse; font-size: 14px; margin: 8px 0; }
  td, th { border: 1px solid #30363d; padding: 4px 10px; text-align: left; }
  .summary { font-size: 16px; }
  .graph { overflow-x: auto; background: #ffffff; border-radius: 8px; padding: 8px; margin: 8px 0; }
  .bad { color: #f85149; }
  details { margin-top: 8px; } summary { cursor: pointer; color: #9da7b3; font-size: 13px; }
  pre { overflow-x: auto; background: #161b22; padding: 10px; border-radius: 8px; font-size: 12px; }
  a { color: #58a6ff; }
  footer { margin-top: 40px; color: #9da7b3; font-size: 13px; }
</style>
</head>
<body>
<h1>codetopo — ask your index</h1>
<p class="sub">Ask your own codebase a plain-words question. The engine measures the answer from your index — tables, a drawn map, and a plain-language summary, with the raw JSON one click below.</p>

<p class="honest"><strong>Your key stays yours.</strong> Paste the hosted API key you claimed (it looks like <code>key_id:secret</code>). Your browser sends it only as the sign-in header of the ask request to this API — it is not stored, not sent anywhere else, and a key can only ever ask about its own indexes. No key yet? <a href="https://aimlds.org/codetopo">Get one on the product page</a>.</p>

<form id="ask-form">
  <label for="key">Your hosted API key</label>
  <input type="password" id="key" autocomplete="off" placeholder="key_id:secret" aria-label="Your hosted API key">
  <label for="index">Your index id</label>
  <input type="text" id="index" autocomplete="off" placeholder="the index_id returned when your repository was indexed" aria-label="Your index id">
  <label for="q">Your question</label>
  <input type="text" id="q" placeholder="Try: How many nodes and edges are in my index?" aria-label="Your question">
  <p><button type="submit">Ask</button></p>
</form>

<div class="examples">
  <button data-q="How many nodes and edges are in my index?">Index size</button>
  <button data-q="Rank my symbols by blast radius">Top symbols</button>
  <button data-q="Verify the audit chain">Verify</button>
  <button data-q="Draw the graph of my index">Draw the graph</button>
</div>

<h2>Answer</h2>
<div id="answer"><p>Ask a question above — the measured answer will appear here.</p></div>

<footer>
  <p>Engine: AGPL-3.0-or-later · Contracts (<a href="/v1/schema">/v1/schema</a>): Apache-2.0 ·
  Source of truth: <a href="https://github.com/nrupala/codetopo">github.com/nrupala/codetopo</a>.</p>
  <p>Owned by <a href="https://nrupalakolkar.com">Nrupal Akolkar</a> · Built with <a href="https://muse.ai/join">Muse by Meta</a>.</p>
</footer>

<script>
"use strict";
function esc(s) {
  return String(s).replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
}
function rawDetails(data) {
  return "<details><summary>raw JSON</summary><pre>" + esc(JSON.stringify(data, null, 2)) + "</pre></details>";
}
const params = new URLSearchParams(location.search);
if (params.get("index")) document.getElementById("index").value = params.get("index");
document.querySelectorAll(".examples button").forEach(b => {
  b.addEventListener("click", () => { document.getElementById("q").value = b.dataset.q; });
});
document.getElementById("ask-form").addEventListener("submit", async ev => {
  ev.preventDefault();
  const el = document.getElementById("answer");
  const key = document.getElementById("key").value.trim();
  const index = document.getElementById("index").value.trim();
  const question = document.getElementById("q").value.trim();
  if (!key) { el.innerHTML = '<p class="bad">Paste your hosted API key first — a key is how the API knows the index is yours.</p>'; return; }
  if (!index || !question) { el.innerHTML = '<p class="bad">An index id and a question are both needed.</p>'; return; }
  el.innerHTML = "<p>Measuring…</p>";
  try {
    const r = await fetch("/v1/graphs/" + encodeURIComponent(index) + "/ask", {
      method: "POST",
      headers: {"Authorization": "Bearer " + key, "Content-Type": "application/json"},
      body: JSON.stringify({question})
    });
    const data = await r.json();
    if (r.status === 401) { el.innerHTML = '<p class="bad">That key was refused. Check it is the full key (key_id:secret), copied exactly.</p>' + rawDetails(data); return; }
    if (!r.ok) { el.innerHTML = '<p class="bad">' + esc(data.message || ("HTTP " + r.status)) + "</p>" + rawDetails(data); return; }
    el.innerHTML = data.html + rawDetails(data);
  } catch (e) {
    el.innerHTML = '<p class="bad">The ask request failed — is this the hosted API host?</p>';
  }
});
</script>
</body>
</html>
"#;
