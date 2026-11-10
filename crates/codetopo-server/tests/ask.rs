// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The keyed human ask surface (endpoints audit gap G1), pinned
//! end-to-end through the assembled [`app`]:
//!
//! 1. keyed access works — a hosted key asks and gets a rendered answer;
//! 2. keyless is refused — no key and a wrong key both answer 401, and
//!    the page itself carries no credential;
//! 3. a known-answer question renders the known answer — the summary
//!    and the rendered table carry the numbers the CLI's own graph
//!    load computes over the same fixture index;
//! 4. an out-of-scope question refuses plainly — no invented answer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header::AUTHORIZATION, header::CONTENT_TYPE, Method, Request, StatusCode};
use axum::Router;
use codetopo_cli::load_graph_from_db;
use serde_json::{json, Value};
use tower::ServiceExt;

use codetopo_server::app;
use codetopo_server::config::Config;
use codetopo_server::keys::KeyStore;
use codetopo_server::metering::Metering;
use codetopo_server::tenancy::Tenancy;

const KEY_ID: &str = "integration-agent";
const SECRET: &str = "fixture-secret-value";

fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-repo")
}

struct Harness {
    router: Router,
    config: Config,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config {
            data_dir: dir.path().join("data"),
            keys_file: dir.path().join("api_keys"),
            metering_log: dir.path().join("metering.log"),
            bind: "127.0.0.1:0".parse().expect("bind addr"),
            index_ttl_secs: None,
            deny_ownerless_indexes: None,
        };
        std::fs::create_dir_all(&config.data_dir).expect("data dir");
        let keys = Arc::new(KeyStore::parse(&format!("{KEY_ID}:{SECRET}\n")));
        let tenancy = Arc::new(Tenancy::open(&config.tenancy_db_path()).expect("tenancy store"));
        let metering = Arc::new(Metering::new(&config.metering_log));
        Harness { router: app(Arc::new(config.clone()), keys, metering, tenancy), config, _dir: dir }
    }

    async fn send(&self, method: Method, uri: &str, body: Option<Value>, authorization: Option<String>) -> (StatusCode, Option<String>, String) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(value) = &authorization {
            builder = builder.header(AUTHORIZATION, value.as_str());
        }
        let body = match &body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(value).expect("serialise body"))
            }
            None => Body::empty(),
        };
        let response = self.router.clone().oneshot(builder.body(body).expect("build request")).await.expect("router response");
        let status = response.status();
        let content_type = response.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(str::to_string);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("response body");
        (status, content_type, String::from_utf8_lossy(&bytes).into_owned())
    }

    async fn ask(&self, index_id: &str, question: &str, authorization: Option<String>) -> (StatusCode, Value) {
        let (status, _, body) = self
            .send(Method::POST, &format!("/v1/graphs/{index_id}/ask"), Some(json!({ "question": question })), authorization)
            .await;
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    fn keyed() -> Option<String> {
        Some(format!("Bearer {KEY_ID}:{SECRET}"))
    }

    async fn index_fixture(&self) -> String {
        let repo = fixture_repo().to_string_lossy().to_string();
        let (status, _, body) = self
            .send(Method::POST, "/v1/index", Some(json!({ "repo_path": repo, "package": "fixture" })), Self::keyed())
            .await;
        assert_eq!(status, StatusCode::OK, "index failed: {body}");
        let report: Value = serde_json::from_str(&body).expect("index report json");
        report["index_id"].as_str().expect("index_id").to_string()
    }
}

// (2, page half) The page is served without a key — it must be, so a
// customer can open it and paste their key — and it carries no
// credential of its own.
#[tokio::test]
async fn ask_page_is_served_and_carries_no_key() {
    let harness = Harness::new();
    let (status, content_type, body) = harness.send(Method::GET, "/ask", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.as_deref().is_some_and(|t| t.contains("text/html")), "content-type: {content_type:?}");
    assert!(body.contains("ask your index"), "page names the surface: {body}");
    assert!(body.contains("Your hosted API key"), "page takes a key: {body}");
    assert!(!body.contains(SECRET), "page must never carry a credential");
}

// (2) Keyless — and wrong-key — asks are refused 401 by the same
// Bearer wrapper as every other /v1 route.
#[tokio::test]
async fn keyless_ask_is_refused() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await;
    let (status, _) = harness.ask(&index_id, "How many nodes and edges are in my index?", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "no key must refuse");
    let (status, _) = harness
        .ask(&index_id, "How many nodes and edges are in my index?", Some(format!("Bearer {KEY_ID}:wrong-secret")))
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a wrong key must refuse");
}

// (1) + (3) A keyed known-answer question renders the known answer:
// the fixture's stats, computed independently from the stored graph
// through the CLI's own loader.
#[tokio::test]
async fn keyed_known_answer_renders_the_known_answer() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await;
    let graph = load_graph_from_db(&harness.config.index_db_path(&index_id)).expect("load graph");

    let (status, answer) = harness
        .ask(&index_id, "How many nodes and edges are in my index?", Harness::keyed())
        .await;
    assert_eq!(status, StatusCode::OK, "keyed ask failed: {answer}");
    assert_eq!(answer["refused"], json!(false));
    assert_eq!(answer["intent"], json!("stats"));
    assert_eq!(answer["data"]["nodes"]["total"], json!(graph.node_count()));
    assert_eq!(answer["data"]["edges"]["total"], json!(graph.edge_count()));
    let summary = answer["summary"].as_str().expect("summary");
    assert!(summary.contains(&graph.node_count().to_string()), "summary carries the measured node count: {summary}");
    assert!(summary.contains(&graph.edge_count().to_string()), "summary carries the measured edge count: {summary}");
    let html = answer["html"].as_str().expect("rendered fragment");
    assert!(html.contains("<table"), "the answer renders a table: {html}");
    assert!(html.contains("function"), "the table carries the per-kind counts: {html}");
}

// (3, closure form) A known-answer closure question: the descendants
// of the fixture's `main`, checked against the core walk itself.
#[tokio::test]
async fn keyed_descendants_question_renders_the_measured_closure() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await;
    let graph = load_graph_from_db(&harness.config.index_db_path(&index_id)).expect("load graph");
    let main_id = graph
        .nodes()
        .find(|n| n.label == "main")
        .map(|n| n.id.clone())
        .expect("fixture has a main");
    let expected = graph.descendants(&main_id);
    assert!(!expected.is_empty(), "fixture main must have descendants");

    let (status, answer) = harness
        .ask(&index_id, &format!("What does {main_id} affect?"), Harness::keyed())
        .await;
    assert_eq!(status, StatusCode::OK, "keyed ask failed: {answer}");
    assert_eq!(answer["refused"], json!(false));
    assert_eq!(answer["data"]["total"], json!(expected.len()));
    let html = answer["html"].as_str().expect("rendered fragment");
    for id in &expected {
        assert!(html.contains(id.as_str()), "rendered table carries {id}: {html}");
    }
}

// (1, drawing form) A render question returns the measured SVG.
#[tokio::test]
async fn keyed_render_question_returns_svg() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await;
    let (status, answer) = harness.ask(&index_id, "Draw the graph of my index", Harness::keyed()).await;
    assert_eq!(status, StatusCode::OK, "keyed ask failed: {answer}");
    assert_eq!(answer["intent"], json!("render"));
    let svg = answer["svg"].as_str().expect("render answer carries an svg");
    assert!(svg.contains("<svg"), "svg is a drawing: {}", &svg[..svg.len().min(120)]);
    assert!(answer["html"].as_str().expect("html").contains("<svg"), "the fragment embeds the drawing");
}

// (4) Out-of-scope questions refuse plainly — and so do edit requests,
// which this read-only product never performs.
#[tokio::test]
async fn out_of_scope_questions_refuse_plainly() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await;
    for question in [
        "What is the weather in Calgary today?",
        "Write me a haiku about autumn leaves",
        "Rename helper_a to helper_c for me",
    ] {
        let (status, answer) = harness.ask(&index_id, question, Harness::keyed()).await;
        assert_eq!(status, StatusCode::OK, "ask failed for {question:?}: {answer}");
        assert_eq!(answer["refused"], json!(true), "{question:?} must be refused: {answer}");
        let summary = answer["summary"].as_str().expect("summary");
        assert!(!summary.is_empty(), "the refusal is stated, not blank");
    }
    let (_, answer) = harness.ask(&index_id, "What is the weather in Calgary today?", Harness::keyed()).await;
    assert!(
        answer["summary"].as_str().expect("summary").contains("can only answer questions about your index"),
        "the refusal says plainly what this surface does: {answer}"
    );
}
