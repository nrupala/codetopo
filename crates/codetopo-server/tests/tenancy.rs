// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Tenancy and runtime key lifecycle, end to end over HTTP (plan S1).
//!
//! Two keys share one server and one data dir. The properties under test
//! are the ones the paid tier stands on:
//!
//! * an index belongs to the key that created it — every query route
//!   answers another key exactly as if the index did not exist;
//! * a legacy index (a database file with no ownership record) stays
//!   readable for a lone-key deployment, so existing self-hosted data
//!   dirs are not stranded — and stops being any-key readable once a
//!   second active key exists, unless the operator rules otherwise
//!   (gate F-5);
//! * an expired index is refused and its file is cleaned up;
//! * keys can be issued, rotated, and revoked against the running
//!   server's store with no restart, and metering lines carry the plan.

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

const KEY_A: (&str, &str) = ("tenant-a", "secret-a-value");
const KEY_B: (&str, &str) = ("tenant-b", "secret-b-value");

fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-repo")
}

/// A two-key server whose stores stay reachable, so a test can perform
/// the same runtime operations the admin subcommand performs.
struct Harness {
    router: Router,
    config: Config,
    keys: Arc<KeyStore>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn with_ttl(index_ttl_secs: Option<u64>) -> Self {
        Self::build(index_ttl_secs, None, &[KEY_A, KEY_B])
    }

    /// A server over a chosen key set and ownerless-index ruling, for
    /// the F-5 legacy tests: the default rule turns on how many keys
    /// are active, and the operator flag overrides it.
    fn with_ownerless(deny_ownerless_indexes: Option<bool>, key_pairs: &[(&str, &str)]) -> Self {
        Self::build(None, deny_ownerless_indexes, key_pairs)
    }

    fn build(
        index_ttl_secs: Option<u64>,
        deny_ownerless_indexes: Option<bool>,
        key_pairs: &[(&str, &str)],
    ) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config {
            data_dir: dir.path().join("data"),
            keys_file: dir.path().join("api_keys"),
            metering_log: dir.path().join("metering.log"),
            bind: "127.0.0.1:0".parse().expect("bind addr"),
            index_ttl_secs,
            deny_ownerless_indexes,
        };
        std::fs::create_dir_all(&config.data_dir).expect("data dir");

        let contents: String = key_pairs
            .iter()
            .map(|(key_id, secret)| format!("{key_id}:{secret}\n"))
            .collect();
        let keys = Arc::new(KeyStore::parse(&contents));
        let tenancy = Arc::new(
            Tenancy::open(&config.data_dir.join(codetopo_server::tenancy::STORE_FILE_NAME))
                .expect("tenancy store"),
        );
        let metering = Arc::new(Metering::new(&config.metering_log));
        Harness {
            router: app(
                Arc::new(config.clone()),
                Arc::clone(&keys),
                metering,
                tenancy,
            ),
            config,
            keys,
            _dir: dir,
        }
    }

    fn db(&self, index_id: &str) -> PathBuf {
        self.config.index_db_path(index_id)
    }

    async fn send_as(
        &self,
        key: (&str, &str),
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {}:{}", key.0, key.1));
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
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn index_as(&self, key: (&str, &str)) -> String {
        let repo = fixture_repo().to_string_lossy().to_string();
        let (status, body) = self
            .send_as(
                key,
                Method::POST,
                "/v1/index",
                Some(json!({ "repo_path": repo, "package": "fixture" })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "index failed: {body}");
        body["index_id"].as_str().expect("index_id").to_string()
    }

    /// Places an index database in the data dir with no ownership
    /// record — the shape every pre-tenancy data dir has.
    async fn place_legacy_index(&self, legacy_id: &str) {
        let db = self.db(legacy_id);
        let repo = fixture_repo();
        tokio::task::spawn_blocking(move || {
            codetopo_cli::index_repo(&repo, &db, "fixture", None, None).expect("index fixture")
        })
        .await
        .expect("index task");
    }

    /// `GET stats` on a legacy index as `key`: the one request the
    /// ownerless tests are all about.
    async fn legacy_stats(&self, key: (&str, &str), legacy_id: &str) -> (StatusCode, Value) {
        self.send_as(
            key,
            Method::GET,
            &format!("/v1/graphs/{legacy_id}/stats"),
            None,
        )
        .await
    }

    fn metering(&self) -> Vec<Value> {
        match std::fs::read_to_string(&self.config.metering_log) {
            Ok(text) => text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(|line| serde_json::from_str(line).expect("metering line is JSON"))
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Every query route names the index in its path; these are the URIs a
/// non-owner would try, built from a real node id so that only tenancy —
/// not a parameter or node error — can be what refuses them.
fn query_uris(index_id: &str, node: &str) -> Vec<String> {
    let node = urlencode(node);
    vec![
        format!("/v1/graphs/{index_id}/descendants?node={node}"),
        format!("/v1/graphs/{index_id}/ancestors?node={node}"),
        format!("/v1/graphs/{index_id}/blast-radius?node={node}"),
        format!("/v1/graphs/{index_id}/path?from={node}&to={node}"),
        format!("/v1/graphs/{index_id}/stats"),
        format!("/v1/graphs/{index_id}/verify"),
        format!("/v1/graphs/{index_id}/snapshot"),
        format!("/v1/graphs/{index_id}/neighborhood?node={node}"),
        format!("/v1/graphs/{index_id}/symbol?node={node}"),
        format!("/v1/graphs/{index_id}/search?q=fixture"),
    ]
}

/// Percent-encodes a node id for a query string, as `tests/api.rs` does:
/// ids contain `:` and could contain `#`, which would truncate the query.
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

#[tokio::test]
async fn another_keys_index_is_invisible() {
    let harness = Harness::with_ttl(None);
    let index_id = harness.index_as(KEY_A).await;
    let graph = load_graph_from_db(&harness.db(&index_id)).expect("load graph");
    let node = graph.nodes().next().expect("fixture has nodes").id.clone();

    for uri in query_uris(&index_id, &node) {
        let (status, body) = harness.send_as(KEY_B, Method::GET, &uri, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} answered {body}");
        assert_eq!(
            body["error"],
            json!("unknown_index"),
            "a foreign index must be indistinguishable from a missing one: {uri}"
        );
    }

    // The owner is unaffected.
    let (status, _) = harness
        .send_as(
            KEY_A,
            Method::GET,
            &format!("/v1/graphs/{index_id}/stats"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn each_key_uses_only_its_own_indexes() {
    let harness = Harness::with_ttl(None);
    let a_index = harness.index_as(KEY_A).await;
    let b_index = harness.index_as(KEY_B).await;
    assert_ne!(a_index, b_index);

    for (owner, own, foreign) in [(KEY_A, &a_index, &b_index), (KEY_B, &b_index, &a_index)] {
        let (status, _) = harness
            .send_as(owner, Method::GET, &format!("/v1/graphs/{own}/stats"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "own index must answer");
        let (status, body) = harness
            .send_as(
                owner,
                Method::GET,
                &format!("/v1/graphs/{foreign}/stats"),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], json!("unknown_index"));
    }
}

/// A database placed in the data dir without an ownership record — the
/// shape every pre-tenancy data dir has — stays readable while the
/// deployment has a single active key: the self-hosted upgrade the
/// legacy posture exists for is never stranded by it.
#[tokio::test]
async fn legacy_unowned_index_stays_readable_for_a_lone_key() {
    let harness = Harness::with_ownerless(None, &[KEY_A]);
    harness.place_legacy_index("legacy-index").await;

    let (status, body) = harness.legacy_stats(KEY_A, "legacy-index").await;
    assert_eq!(status, StatusCode::OK, "lone-key legacy index refused: {body}");
}

/// F-5's default branch: once the key store holds a second active
/// key, a legacy index stops being any-key readable — answered
/// exactly like a missing index, for every key, so the refusal does
/// not confirm the index exists.
#[tokio::test]
async fn legacy_unowned_index_is_denied_once_a_second_key_exists() {
    let harness = Harness::with_ownerless(None, &[KEY_A, KEY_B]);
    harness.place_legacy_index("legacy-index").await;

    for key in [KEY_A, KEY_B] {
        let (status, body) = harness.legacy_stats(key, "legacy-index").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "legacy index answered a tenant of a multi-key store: {body}"
        );
        assert_eq!(body["error"], json!("unknown_index"));
    }
}

/// F-5's operator flag, both directions: `0`/`false` keeps the any-key
/// legacy rule even with two keys; `1`/`true` denies a legacy index
/// even to a lone key.
#[tokio::test]
async fn legacy_unowned_index_follows_the_operator_flag() {
    let allowed = Harness::with_ownerless(Some(false), &[KEY_A, KEY_B]);
    allowed.place_legacy_index("legacy-index").await;
    for key in [KEY_A, KEY_B] {
        let (status, body) = allowed.legacy_stats(key, "legacy-index").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "operator-allowed legacy index refused: {body}"
        );
    }

    let denied = Harness::with_ownerless(Some(true), &[KEY_A]);
    denied.place_legacy_index("legacy-index").await;
    let (status, body) = denied.legacy_stats(KEY_A, "legacy-index").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!("unknown_index"));
}

/// A zero TTL makes the index expire the moment it exists: the first
/// query is refused, and the refusal removes the database file.
#[tokio::test]
async fn expired_index_is_refused_and_cleaned_up() {
    let harness = Harness::with_ttl(Some(0));
    let index_id = harness.index_as(KEY_A).await;
    assert!(harness.db(&index_id).exists(), "index file written");

    let (status, body) = harness
        .send_as(
            KEY_A,
            Method::GET,
            &format!("/v1/graphs/{index_id}/stats"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!("unknown_index"));
    assert!(
        !harness.db(&index_id).exists(),
        "an expired index's file is removed when it is refused"
    );
}

/// Issue → rotate → revoke against the store the running server reads:
/// each step takes effect on the very next request, with no restart.
#[tokio::test]
async fn runtime_key_lifecycle_needs_no_restart() {
    let harness = Harness::with_ttl(None);

    let secret = harness.keys.issue("tenant-c", "hosted").expect("issue");
    let key_c = ("tenant-c", secret.as_str());
    let (status, _) = harness.send_as(key_c, Method::GET, "/v1/schema", None).await;
    assert_eq!(status, StatusCode::OK, "a just-issued key must authenticate");

    let rotated = harness.keys.rotate("tenant-c").expect("rotate");
    let (status, _) = harness.send_as(key_c, Method::GET, "/v1/schema", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "the old secret dies at rotation");
    let (status, _) = harness
        .send_as(("tenant-c", rotated.as_str()), Method::GET, "/v1/schema", None)
        .await;
    assert_eq!(status, StatusCode::OK, "the new secret works at once");

    harness.keys.revoke("tenant-c").expect("revoke");
    let (status, _) = harness
        .send_as(("tenant-c", rotated.as_str()), Method::GET, "/v1/schema", None)
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a revoked key is refused");
}

/// Metering lines carry the plan of the key that made the request, so
/// monthly per-plan counters can be read straight off the log.
#[tokio::test]
async fn metering_records_carry_the_keys_plan() {
    let harness = Harness::with_ttl(None);
    let secret = harness.keys.issue("tenant-c", "hosted").expect("issue");

    harness.index_as(KEY_A).await;
    harness
        .send_as(("tenant-c", secret.as_str()), Method::GET, "/v1/schema", None)
        .await;

    let records = harness.metering();
    assert!(records.len() >= 2, "both requests metered: {records:?}");
    for record in &records {
        assert!(
            record["plan"].is_string(),
            "every record names its key's plan: {record}"
        );
    }
    let a_records: Vec<&Value> = records
        .iter()
        .filter(|r| r["key_id"] == json!(KEY_A.0))
        .collect();
    assert!(a_records
        .iter()
        .all(|r| r["plan"] == json!("self-hosted")));
    let c_records: Vec<&Value> = records
        .iter()
        .filter(|r| r["key_id"] == json!("tenant-c"))
        .collect();
    assert_eq!(c_records.len(), 1);
    assert_eq!(c_records[0]["plan"], json!("hosted"));
    assert_eq!(c_records[0]["path"], json!("/v1/schema"));
}
