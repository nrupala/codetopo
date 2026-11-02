// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! F4/F5 (hosted shape) for the Python extractor: the known-answer
//! fixture package (`codetopo-extract/tests/fixture-py`, package
//! `quarry`) indexed through `POST /v1/index`, with the standard
//! query set — stats, rank, symbol dossier, diagnostics, and a
//! self-diff — asserted identical to the CLI's view of the same
//! database, in the parity idiom of `tests/api.rs`.
//!
//! Full-directory indexing of the fixture counts 4 files (the three
//! `.py` files plus the fixture README, which indexes as a doc),
//! 29 nodes and 36 edges: the fixture README shows the arithmetic
//! (26 nodes / 33 edges for the Python files, +1 file node, +1
//! module node and +1 doc node, +3 contains edges for the README).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header::AUTHORIZATION, Method, Request, StatusCode};
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

fn fixture_py() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../codetopo-extract/tests/fixture-py")
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
        Harness {
            router: app(Arc::new(config.clone()), keys, metering, tenancy),
            config,
            _dir: dir,
        }
    }

    fn db(&self, index_id: &str) -> PathBuf {
        self.config.index_db_path(index_id)
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.send(
            Method::GET,
            uri,
            None,
            Some(format!("Bearer {KEY_ID}:{SECRET}")),
        )
        .await
    }

    async fn send(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
        authorization: Option<String>,
    ) -> (StatusCode, Value) {
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
        let response = self
            .router
            .clone()
            .oneshot(builder.body(body).expect("build request"))
            .await
            .expect("router response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    async fn index_fixture(&self) -> Value {
        let repo = fixture_py().to_string_lossy().to_string();
        let (status, body) = self
            .send(
                Method::POST,
                "/v1/index",
                Some(json!({ "repo_path": repo, "package": "quarry" })),
                Some(format!("Bearer {KEY_ID}:{SECRET}")),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "index failed: {body}");
        body
    }
}

#[tokio::test]
async fn py_index_report_and_stats_match_the_cli_graph() {
    let harness = Harness::new();
    let report = harness.index_fixture().await;
    let index_id = report["index_id"].as_str().expect("index_id").to_string();

    // Full-directory counts (fixture README shows the arithmetic):
    // 4 files (3 .py + the README), 29 nodes, 36 edges, and the
    // fixture's 2 known diagnostics (the property MultiDefinition
    // and render's UnresolvedCall).
    assert_eq!(report["files_indexed"], json!(4));
    assert_eq!(report["files_failed"], json!(0));
    assert_eq!(report["diagnostics"], json!(2));

    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");
    assert_eq!(graph.node_count(), 29);
    assert_eq!(graph.edge_count(), 36);
    assert_eq!(report["nodes"], json!(graph.node_count()));
    assert_eq!(report["edges"], json!(graph.edge_count()));

    let (status, stats) = harness.get(&format!("/v1/graphs/{index_id}/stats")).await;
    assert_eq!(status, StatusCode::OK, "stats failed: {stats}");
    assert_eq!(stats["nodes"]["total"], json!(graph.node_count()));
    assert_eq!(stats["edges"]["total"], json!(graph.edge_count()));
}

#[tokio::test]
async fn py_query_set_matches_the_cli_values() {
    let harness = Harness::new();
    let report = harness.index_fixture().await;
    let index_id = report["index_id"].as_str().expect("index_id").to_string();
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");

    // Rank, every metric: HTTP == CLI value (index_id stripped).
    for (by, metric) in [
        ("fan_in", codetopo_core::RankMetric::FanIn),
        ("fan_out", codetopo_core::RankMetric::FanOut),
        ("blast_radius", codetopo_core::RankMetric::BlastRadius),
        ("complexity", codetopo_core::RankMetric::Complexity),
    ] {
        let (status, body) = harness
            .get(&format!("/v1/graphs/{index_id}/rank?by={by}"))
            .await;
        assert_eq!(status, StatusCode::OK, "rank {by}: {body}");
        let mut via_http = body.clone();
        via_http
            .as_object_mut()
            .expect("object")
            .remove("index_id")
            .expect("index_id present");
        assert_eq!(
            via_http,
            codetopo_cli::rank_value(&graph, metric, 20),
            "HTTP rank == CLI rank for {by}"
        );
    }

    // Symbol dossier for a Python class, plus the 404 discipline.
    let circle = graph
        .nodes()
        .find(|n| n.id == "quarry::shapes::Circle")
        .expect("Circle indexed")
        .clone();
    let (status, body) = harness
        .get(&format!("/v1/graphs/{index_id}/symbol?node={}", circle.id))
        .await;
    assert_eq!(status, StatusCode::OK, "symbol: {body}");
    assert_eq!(body["node"]["id"], json!(circle.id));
    assert_eq!(body["node"]["label"], json!(circle.label));
    let (status, _) = harness
        .get(&format!("/v1/graphs/{index_id}/symbol?node=no::such::id"))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Diagnostics: HTTP == CLI list, and it is the fixture's known
    // pair (MultiDefinition on the property, render's unresolved
    // call).
    let (status, body) = harness
        .get(&format!("/v1/graphs/{index_id}/diagnostics"))
        .await;
    assert_eq!(status, StatusCode::OK, "diagnostics: {body}");
    let via_cli =
        codetopo_cli::diagnostics_value(&harness.db(&index_id)).expect("cli diagnostics value");
    let mut via_http = body.clone();
    via_http
        .as_object_mut()
        .expect("object")
        .remove("index_id")
        .expect("index_id present");
    assert_eq!(via_http, via_cli, "HTTP diagnostics == CLI diagnostics");
    let kinds: Vec<&str> = body["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .map(|d| {
            d.as_object()
                .and_then(|o| o.keys().next())
                .map(|k| k.as_str())
                .expect("variant tag")
        })
        .collect();
    assert_eq!(kinds.len(), 2, "the fixture's two diagnostics: {body}");
    assert!(kinds.contains(&"MultiDefinition"), "{kinds:?}");
    assert!(kinds.contains(&"UnresolvedCall"), "{kinds:?}");
}

#[tokio::test]
async fn py_diff_against_an_identical_index_is_empty() {
    let harness = Harness::new();
    let first = harness.index_fixture().await;
    let second = harness.index_fixture().await;
    let first_id = first["index_id"].as_str().expect("index_id").to_string();
    let second_id = second["index_id"].as_str().expect("index_id").to_string();
    assert_ne!(first_id, second_id, "re-indexing yields a second index");

    let g1 = load_graph_from_db(&harness.db(&first_id)).expect("load first");
    let g2 = load_graph_from_db(&harness.db(&second_id)).expect("load second");
    let (status, body) = harness
        .get(&format!("/v1/graphs/{first_id}/diff?other={second_id}"))
        .await;
    assert_eq!(status, StatusCode::OK, "diff: {body}");
    let mut via_http = body.clone();
    let obj = via_http.as_object_mut().expect("object");
    obj.remove("index_id").expect("index_id present");
    obj.remove("other_index_id").expect("other_index_id present");
    assert_eq!(
        via_http,
        codetopo_cli::diff_value(&g1, &g2),
        "HTTP diff == CLI diff"
    );
    // And the shared value itself is the empty diff.
    assert_eq!(via_http["added_nodes"], json!([]));
    assert_eq!(via_http["removed_nodes"], json!([]));
    assert_eq!(via_http["changed_nodes"], json!([]));
}
