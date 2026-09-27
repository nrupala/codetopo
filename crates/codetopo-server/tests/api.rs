// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! End-to-end tests for the HTTP surface, over a real on-disk index.
//!
//! The unit tests in `src/` exercise parsing and matching in isolation. These
//! drive the assembled [`app`] through `tower::ServiceExt::oneshot`, so the
//! router, the auth layer, the store, and the extractor are all in the path.
//!
//! The comparison target is the CLI's own view of the same database
//! ([`codetopo_cli::load_graph_from_db`] plus the core walk), not a hand-written
//! expectation. That is the only definition of "correct" that cannot drift
//! from the CLI between releases: §3.2 makes CLI parity the contract, so a
//! fixture id hard-coded here would only test that the extractor still spells
//! ids the way it did when this file was written.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header::AUTHORIZATION, Method, Request, StatusCode};
use axum::Router;
use codetopo_cli::{load_graph_from_db, render_path};
use serde_json::{json, Value};
use tower::ServiceExt;

use codetopo_server::app;
use codetopo_server::config::Config;
use codetopo_server::keys::KeyStore;
use codetopo_server::metering::Metering;

/// `key_id` for the one key the test server trusts.
const KEY_ID: &str = "integration-agent";
/// Its secret. Asserted absent from the metering log.
const SECRET: &str = "fixture-secret-value";

/// The fixture repository: three files, two call chains.
///
/// A committed tree rather than files written into a temp dir at run time,
/// because the whole point is that this graph does not change. If the extractor
/// changes shape, the parity assertions below keep passing but
/// `fixture_has_call_chains` fails, naming the regression instead of quietly
/// reducing the suite to empty closures.
fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-repo")
}

/// A server with its own data dir, keys, and metering log.
///
/// `data_dir` is created eagerly because [`Config::prepare`] is what creates it
/// in production, and these tests call [`app`] directly rather than through
/// `main`.
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
        };
        std::fs::create_dir_all(&config.data_dir).expect("data dir");

        let keys = Arc::new(KeyStore::parse(&format!("{KEY_ID}:{SECRET}\n")));
        let metering = Arc::new(Metering::new(&config.metering_log));
        Harness {
            router: app(Arc::new(config.clone()), keys, metering),
            config,
            _dir: dir,
        }
    }

    fn db(&self, index_id: &str) -> std::path::PathBuf {
        self.config.index_db_path(index_id)
    }

    /// Sends an authenticated request and returns status plus parsed body.
    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.send(
            Method::GET,
            uri,
            None,
            Some(format!("Bearer {KEY_ID}:{SECRET}")),
        )
        .await
    }

    /// Sends a request with an explicit `Authorization` value, or none.
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

    /// Indexes the fixture through the API and returns the report.
    async fn index_fixture(&self) -> Value {
        let repo = fixture_repo().to_string_lossy().to_string();
        let (status, body) = self
            .send(
                Method::POST,
                "/v1/index",
                Some(json!({ "repo_path": repo, "package": "fixture" })),
                Some(format!("Bearer {KEY_ID}:{SECRET}")),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "index failed: {body}");
        body
    }

    /// Metering records appended so far, in order.
    fn metering(&self) -> Vec<Value> {
        match std::fs::read_to_string(&self.config.metering_log) {
            Ok(text) => text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str(line).expect("metering line is JSON"))
                .collect(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => panic!("cannot read metering log: {err}"),
        }
    }
}

/// The ids in a closure response.
fn ids(body: &Value) -> Vec<String> {
    body["nodes"]
        .as_array()
        .expect("nodes is an array")
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("closure node is a string id")
                .to_string()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// (a) Index parity
// ---------------------------------------------------------------------------

/// The API's index counts are the CLI's counts.
///
/// The API returns the `IndexReport` the CLI built, so if these ever diverge the
/// response is not being populated from the same run — which is the failure
/// §3.2 exists to prevent.
#[tokio::test]
async fn index_report_matches_the_indexed_graph() {
    let harness = Harness::new();
    let report = harness.index_fixture().await;
    let index_id = report["index_id"].as_str().expect("index_id").to_string();

    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");
    assert_eq!(report["files_indexed"], json!(3));
    assert_eq!(report["files_failed"], json!(0));
    assert_eq!(report["nodes"], json!(graph.node_count()));
    assert_eq!(report["edges"], json!(graph.edge_count()));
    assert!(report["diagnostics"].is_number());

    // And the same numbers from the stats route, which counts the stored graph
    // rather than reporting the indexing run.
    let (status, stats) = harness.get(&format!("/v1/graphs/{index_id}/stats")).await;
    assert_eq!(status, StatusCode::OK, "stats failed: {stats}");
    assert_eq!(stats["index_id"], json!(index_id));
    assert_eq!(stats["nodes"]["total"], json!(graph.node_count()));
    assert_eq!(stats["edges"]["total"], json!(graph.edge_count()));
}

/// The index id is opaque, and a re-index does not clobber the first index.
#[tokio::test]
async fn reindexing_yields_a_second_index() {
    let harness = Harness::new();
    let first = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let second = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();

    assert_ne!(first, second, "a new index must get a new id");
    assert!(harness.db(&first).exists(), "first index must survive");
    assert!(harness.db(&second).exists());
    assert_eq!(first.len(), 36, "index id should be a UUID");
}

/// The fixture is not silently degenerate.
///
/// A parity suite over a graph with no edges would pass while testing nothing,
/// so this states the precondition the other tests rely on.
#[tokio::test]
async fn fixture_has_call_chains() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");

    let with_callers = graph
        .nodes()
        .filter(|node| !graph.ancestors(&node.id).is_empty())
        .count();
    let with_callees = graph
        .nodes()
        .filter(|node| !graph.descendants(&node.id).is_empty())
        .count();
    assert!(with_callers > 0, "fixture produced no inbound edges");
    assert!(with_callees > 0, "fixture produced no outbound edges");
}

// ---------------------------------------------------------------------------
// (b) Query round-trip: HTTP closure == core walk
// ---------------------------------------------------------------------------

/// Every closure route agrees with `codetopo_core`, for every node.
///
/// `expand=true` lifts the response cap, so `nodes` is the whole closure in the
/// walk's own order — an exact comparison is available, and anything weaker
/// would hide an ordering or deduplication difference.
#[tokio::test]
async fn closure_routes_match_the_core_walk_for_every_node() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");

    let ids_seen: Vec<String> = graph.nodes().map(|node| node.id.clone()).collect();
    assert!(!ids_seen.is_empty());

    for node in &ids_seen {
        for (route, expected) in [
            ("descendants", graph.descendants(node)),
            ("ancestors", graph.ancestors(node)),
            ("blast-radius", graph.blast_radius(node)),
        ] {
            let uri = format!(
                "/v1/graphs/{index_id}/{route}?node={}&expand=true",
                urlencode(node)
            );
            let (status, body) = harness.get(&uri).await;
            assert_eq!(status, StatusCode::OK, "{route} {node} failed: {body}");
            assert_eq!(body["index_id"], json!(index_id));
            assert_eq!(body["node"], json!(node));
            assert_eq!(body["relation"], json!(route.replace('-', "_")));
            assert_eq!(body["total"], json!(expected.len()));
            assert_eq!(body["expand"], json!(true));
            assert_eq!(body["truncated"], json!(false));
            assert_eq!(ids(&body), expected, "{route} disagreed for {node}");
        }
    }
}

/// The default (non-expanded) list is a prefix of the full closure, and `total`
/// still reports the whole size.
#[tokio::test]
async fn default_list_is_capped_but_total_is_not() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");

    // A node with the largest closure in the fixture, so the cap is reached.
    let (node, full) = graph
        .nodes()
        .map(|node| (node.id.clone(), graph.descendants(&node.id)))
        .max_by_key(|(_, closure)| closure.len())
        .expect("fixture has nodes");
    let limit = full.len().min(2);

    let (status, body) = harness
        .get(&format!(
            "/v1/graphs/{index_id}/descendants?node={}&limit={limit}",
            urlencode(&node)
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "closure failed: {body}");
    assert_eq!(
        body["total"],
        json!(full.len()),
        "total is the whole closure"
    );
    assert_eq!(body["limit"], json!(limit));
    assert_eq!(body["expand"], json!(false));
    assert_eq!(body["truncated"], json!(full.len() > limit));
    assert_eq!(ids(&body), full[..limit].to_vec());
}

/// `ancestors` is not `descendants` run backwards on the same answer.
///
/// The two directions are easy to conflate in a handler; this pins that the
/// fixture actually distinguishes them, so the parity test above is not
/// comparing one closure to itself.
#[tokio::test]
async fn upstream_and_downstream_differ_on_the_fixture() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");

    let any_asymmetric = graph.nodes().any(|node| {
        let down = graph.descendants(&node.id);
        let up = graph.ancestors(&node.id);
        !down.is_empty() && !up.is_empty() && down != up
    });
    assert!(
        any_asymmetric,
        "fixture has no node with differing upstream and downstream closures"
    );
}

/// `path` agrees with the core's shortest-path walk.
#[tokio::test]
async fn path_route_matches_the_core_walk() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");

    let from = graph
        .nodes()
        .find(|node| !graph.descendants(&node.id).is_empty())
        .expect("a node with callees")
        .id
        .clone();
    let to = graph
        .descendants(&from)
        .last()
        .cloned()
        .expect("a reachable node");
    let expected = graph.path(&from, &to).expect("path exists");

    let (status, body) = harness
        .get(&format!(
            "/v1/graphs/{index_id}/path?from={}&to={}",
            urlencode(&from),
            urlencode(&to)
        ))
        .await;
    assert_eq!(status, StatusCode::OK, "path failed: {body}");
    assert_eq!(body["index_id"], json!(index_id));
    assert_eq!(body["from"], json!(from));
    assert_eq!(body["to"], json!(to));
    assert_eq!(body["steps"].as_array().map(Vec::len), Some(expected.len()));
    assert_eq!(body["rendered"], json!(render_path(&expected)));
}

// ---------------------------------------------------------------------------
// Snapshot and verify: the audit chain survives the round-trip
// ---------------------------------------------------------------------------

/// A snapshot of a real index verifies against its own chain.
#[tokio::test]
async fn snapshot_verifies_over_http() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");

    let (status, snapshot) = harness
        .get(&format!("/v1/graphs/{index_id}/snapshot"))
        .await;
    assert_eq!(status, StatusCode::OK, "snapshot failed: {snapshot}");
    assert_eq!(snapshot["schema_version"], json!("1.0-draft"));
    assert_eq!(
        snapshot["nodes"].as_array().map(Vec::len),
        Some(graph.node_count())
    );
    assert_eq!(
        snapshot["edges"].as_array().map(Vec::len),
        Some(graph.edge_count())
    );
    assert!(
        snapshot["audit_log"]
            .as_array()
            .is_some_and(|log| !log.is_empty()),
        "a snapshot always carries at least the genesis entry: {snapshot}"
    );
    let head = snapshot["audit_head"]
        .as_str()
        .expect("audit_head is a hash")
        .to_string();
    assert_eq!(head.len(), 64, "audit head should be a hex sha256");
    assert!(head.chars().all(|c| c.is_ascii_hexdigit()));

    let (status, verify) = harness.get(&format!("/v1/graphs/{index_id}/verify")).await;
    assert_eq!(status, StatusCode::OK, "verify failed: {verify}");
    assert_eq!(verify["index_id"], json!(index_id));
    assert_eq!(verify["chain_valid"], json!(true));
    assert_eq!(verify["head"], json!(head));
    assert!(verify["entries"].as_u64().expect("entries") >= 1);
}

/// Verifying an index that was never written is an error, not an empty pass.
#[tokio::test]
async fn verifying_an_unknown_index_is_not_found() {
    let harness = Harness::new();
    let (status, body) = harness.get("/v1/graphs/no-such-index/verify").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body was {body}");
    assert_eq!(body["error"], json!("unknown_index"));
}

// ---------------------------------------------------------------------------
// (d) Auth and metering
// ---------------------------------------------------------------------------

/// Every route requires a key, including the read-only ones.
#[tokio::test]
async fn unauthenticated_requests_are_rejected_everywhere() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();

    for (method, uri) in [
        (Method::GET, format!("/v1/graphs/{index_id}/stats")),
        (Method::GET, format!("/v1/graphs/{index_id}/snapshot")),
        (
            Method::GET,
            format!("/v1/graphs/{index_id}/descendants?node=x"),
        ),
        (Method::GET, format!("/v1/graphs/{index_id}/verify")),
    ] {
        let (status, body) = harness.send(method, &uri, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} was {body}");
        assert_eq!(body["error"], json!("unauthenticated"));
    }
}

/// A wrong secret is rejected, and the response does not reveal which part was
/// wrong.
#[tokio::test]
async fn wrong_secret_is_rejected_with_a_challenge() {
    let harness = Harness::new();
    let uri = "/v1/graphs/some-index/stats";

    let (status, wrong_secret) = harness
        .send(
            Method::GET,
            uri,
            None,
            Some(format!("Bearer {KEY_ID}:not-the-secret")),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_secret["error"], json!("unauthenticated"));

    let (status, _) = harness
        .send(
            Method::GET,
            uri,
            None,
            Some(format!("Bearer no-such-key:{SECRET}")),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    assert_eq!(
        wrong_secret,
        unknown_key_body(&harness).await,
        "a bad secret must be indistinguishable from an unknown key"
    );
}

/// The `401` carries a `WWW-Authenticate` challenge.
#[tokio::test]
async fn unauthorized_response_sends_a_challenge() {
    let harness = Harness::new();
    let response = harness
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/graphs/some-index/stats")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response
            .headers()
            .get("www-authenticate")
            .expect("challenge header"),
        "Bearer realm=\"codetopo\""
    );
}

async fn unknown_key_body(harness: &Harness) -> Value {
    let (_, body) = harness
        .send(
            Method::GET,
            "/v1/graphs/some-index/stats",
            None,
            Some(format!("Bearer no-such-key-at-all:{SECRET}")),
        )
        .await;
    body
}

/// One served request is one metering record, attributed to the key and not to
/// the secret.
#[tokio::test]
async fn a_served_request_appends_one_metering_record() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let before = harness.metering().len();

    // A query string is deliberately present: metering records the path without
    // it, so a client cannot inflate the log with parameters.
    let (status, _body) = harness
        .get(&format!(
            "/v1/graphs/{index_id}/descendants?node=anything&limit=5&extra=x"
        ))
        .await;
    assert_eq!(status, StatusCode::OK);

    let records = harness.metering();
    assert_eq!(records.len(), before + 1, "exactly one record per request");
    let record = &records[records.len() - 1];
    assert_eq!(record["key_id"], json!(KEY_ID));
    assert_eq!(record["method"], json!("GET"));
    assert_eq!(
        record["path"],
        json!(format!("/v1/graphs/{index_id}/descendants")),
        "query string must not be logged"
    );
    assert_eq!(record["status"], json!(200));
    assert!(record["response_bytes"].as_u64().expect("bytes") > 0);
    assert!(record["ts"].as_str().expect("ts").ends_with('Z'));

    let log = std::fs::read_to_string(&harness.config.metering_log).expect("read log");
    assert!(
        !log.contains(SECRET),
        "the secret must never reach the metering log"
    );
}

/// A rejected request is answered but not metered.
///
/// Metering records the key that did work; a `401` had none, and writing a
/// record would require inventing an identity.
#[tokio::test]
async fn rejected_requests_are_not_metered() {
    let harness = Harness::new();
    let before = harness.metering().len();

    for authorization in [
        None,
        Some(format!("Bearer {KEY_ID}:wrong")),
        Some(format!("Basic {KEY_ID}:{SECRET}")),
        Some(format!("Bearer {SECRET}")),
    ] {
        let (status, _) = harness
            .send(
                Method::GET,
                "/v1/graphs/some-index/stats",
                None,
                authorization,
            )
            .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    assert_eq!(
        harness.metering().len(),
        before,
        "401 responses must not be metered"
    );
}

/// The metering log is one JSON object per line, append-only.
#[tokio::test]
async fn metering_log_is_append_only_json_lines() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let after_index = harness.metering().len();
    assert!(after_index > 0, "the index request should be metered");

    let stats_path = format!("/v1/graphs/{index_id}/stats");
    harness.get(&stats_path).await;
    let records = harness.metering();
    assert_eq!(records.len(), after_index + 1);
    assert_eq!(records[after_index]["path"], json!(stats_path));
    assert!(records
        .iter()
        .all(|record| record["key_id"].is_string() && record["ts"].is_string()));
}

// ---------------------------------------------------------------------------
// Input validation
// ---------------------------------------------------------------------------

/// A missing `node` is a named parameter error, not a deserialization failure.
#[tokio::test]
async fn closure_routes_name_the_missing_parameter() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();

    for uri in [
        format!("/v1/graphs/{index_id}/descendants"),
        format!("/v1/graphs/{index_id}/descendants?node="),
    ] {
        let (status, body) = harness.get(&uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri} was {body}");
        assert_eq!(body["error"], json!("missing_param"));
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains("node"),
            "the message should name the parameter: {body}"
        );
    }
}

/// Bad `expand` and `limit` values are rejected with the offending value named.
#[tokio::test]
async fn closure_params_are_validated() {
    let harness = Harness::new();
    let index_id = harness.index_fixture().await["index_id"]
        .as_str()
        .expect("index_id")
        .to_string();
    let base = format!("/v1/graphs/{index_id}/descendants?node=x");

    for (query, parameter) in [
        ("&expand=yes", "expand"),
        ("&limit=abc", "limit"),
        ("&limit=0", "limit"),
    ] {
        let (status, body) = harness.get(&format!("{base}{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query} was {body}");
        assert_eq!(body["error"], json!("invalid_param"));
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains(parameter),
            "the message should name `{parameter}`: {body}"
        );
    }
}

/// An empty or non-directory `repo_path` is a client error naming the field.
#[tokio::test]
async fn index_validates_repo_path() {
    let harness = Harness::new();

    let (status, body) = harness
        .send(
            Method::POST,
            "/v1/index",
            Some(json!({})),
            Some(format!("Bearer {KEY_ID}:{SECRET}")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body was {body}");
    assert_eq!(body["error"], json!("invalid_repo_path"));

    let (status, body) = harness
        .send(
            Method::POST,
            "/v1/index",
            Some(json!({ "repo_path": fixture_repo().join("does-not-exist") })),
            Some(format!("Bearer {KEY_ID}:{SECRET}")),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body was {body}");
    assert_eq!(body["error"], json!("invalid_repo_path"));
}

/// A malformed JSON body is a 4xx, not a panic or a 500.
#[tokio::test]
async fn malformed_json_is_a_client_error() {
    let harness = Harness::new();
    let response = harness
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/index")
                .header(AUTHORIZATION, format!("Bearer {KEY_ID}:{SECRET}"))
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert!(
        response.status().is_client_error(),
        "got {}",
        response.status()
    );
}

/// Percent-encodes a node id for use in a query string.
///
/// Node ids contain `:`, `::`, and `#` from Rust paths. `#` in particular would
/// otherwise truncate the query and turn a closure request into a different one.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
