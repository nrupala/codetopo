// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The abuse envelope, end to end over HTTP (paid-path plan S3,
//! gaps G4/G5).
//!
//! The properties under test are the ones a paying stranger's bill
//! and every other tenant's service stand on:
//!
//! * a hosted key's 25th active index is allowed and its 26th is
//!   refused `429 index_quota_exceeded`, naming the limit and the
//!   count — the boundary, not a neighbourhood of it;
//! * a hosted key's monthly queries are counted from the metering
//!   log: the 10,000th query of the UTC month is served, the next
//!   is refused `429 query_quota_exceeded`, and index creation is
//!   not a query — it still works with the query quota spent;
//! * a tree past the 50,000-file cap is refused
//!   `429 file_quota_exceeded` and leaves nothing behind — no
//!   database, no staged tree;
//! * the operator's self-hosted key is not enveloped at all: past
//!   every hosted number, it still queries and indexes.
//!
//! The boundary counts are reached by seeding the stores the
//! envelope reads — ownership records in the tenancy store, usage
//! lines in the metering log — rather than by performing 25 index
//! builds or 10,000 HTTP calls: the seeds *are* the state the
//! envelope sees, written through the same stores production uses.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header::AUTHORIZATION, Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

use codetopo_server::app;
use codetopo_server::config::Config;
use codetopo_server::keys::KeyStore;
use codetopo_server::metering::{current_month, Metering};
use codetopo_server::tenancy::Tenancy;

/// The operator's key (legacy import: plan `self-hosted`).
const OPERATOR: (&str, &str) = ("operator", "operator-secret-value");

fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-repo")
}

/// A server whose stores stay reachable, so a test can issue keys
/// and seed exactly the state the envelope will read.
struct Harness {
    router: Router,
    config: Config,
    keys: Arc<KeyStore>,
    tenancy: Arc<Tenancy>,
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

        let keys = Arc::new(KeyStore::parse(&format!(
            "{}:{}\n",
            OPERATOR.0, OPERATOR.1
        )));
        let tenancy = Arc::new(Tenancy::open(&config.tenancy_db_path()).expect("tenancy store"));
        let metering = Arc::new(Metering::new(&config.metering_log));
        Harness {
            router: app(
                Arc::new(config.clone()),
                Arc::clone(&keys),
                metering,
                Arc::clone(&tenancy),
            ),
            config,
            keys,
            tenancy,
            _dir: dir,
        }
    }

    /// A hosted-plan key, issued the way the billing bridge issues one.
    fn hosted_key(&self, key_id: &str) -> (String, String) {
        let secret = self.keys.issue(key_id, "hosted").expect("issue hosted key");
        (key_id.to_string(), secret)
    }

    async fn send(
        &self,
        key: (&str, &str),
        method: Method,
        uri: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(AUTHORIZATION, format!("Bearer {}:{}", key.0, key.1));
        if let Some(content_type) = content_type {
            builder = builder.header("content-type", content_type);
        }
        let response = self
            .router
            .clone()
            .oneshot(builder.body(Body::from(body)).expect("build request"))
            .await
            .expect("router response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn get(&self, key: (&str, &str), uri: &str) -> (StatusCode, Value) {
        self.send(key, Method::GET, uri, None, Vec::new()).await
    }

    async fn upload(&self, key: (&str, &str), zip: Vec<u8>) -> (StatusCode, Value) {
        self.send(
            key,
            Method::POST,
            "/v1/index",
            Some("application/zip"),
            zip,
        )
        .await
    }

    /// Records `count` live indexes as owned by `key_id` — the state
    /// a key that had created them would be in.
    fn seed_active_indexes(&self, key_id: &str, count: usize) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs() as i64;
        for i in 0..count {
            self.tenancy
                .record(&format!("seed-{key_id}-{i}"), key_id, now, None)
                .expect("seed index record");
        }
    }

    /// Index database files currently in the data dir (the tenancy
    /// store's own files excluded).
    fn index_db_files(&self) -> Vec<String> {
        std::fs::read_dir(&self.config.data_dir)
            .expect("data dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".db") && name != "tenancy.db")
            .collect()
    }

    /// Whatever ingest workdirs currently exist (names only).
    fn workdirs(&self) -> Vec<String> {
        match std::fs::read_dir(self.config.data_dir.join("ingest")) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// Appends `count` metered query records for `key_id` in the
/// current UTC month to the harness's metering log — the usage a
/// key that had made those queries would carry. The lines are
/// written in the log's own format; the envelope parses them back
/// through the real record type, so a format drift breaks this
/// test rather than silently unseeding it.
fn seed_query_records(harness: &Harness, key_id: &str, count: usize) {
    let month = current_month();
    let mut text = String::new();
    for _ in 0..count {
        text.push_str(&format!(
            "{{\"ts\":\"{month}-15T00:00:00.000Z\",\"key_id\":\"{key_id}\",\
             \"plan\":\"hosted\",\"method\":\"GET\",\"path\":\"/v1/graphs/x/stats\",\
             \"status\":200,\"response_bytes\":1,\"duration_ms\":1}}\n"
        ));
    }
    std::fs::write(&harness.config.metering_log, text).expect("seed metering log");
}

/// The fixture repository as a zip, wrapped in one top-level directory.
fn fixture_zip() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    for name in ["src/alpha.rs", "src/beta.rs", "src/main.rs"] {
        let bytes = std::fs::read(fixture_repo().join(name)).expect("fixture file");
        writer
            .start_file(format!("fixture-repo/{name}"), options)
            .expect("start file");
        writer.write_all(&bytes).expect("write file");
    }
    writer.finish().expect("finish zip").into_inner()
}

/// A zip of `files` empty `.rs` files at the archive root.
fn many_files_zip(files: usize) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    for i in 0..files {
        writer
            .start_file(format!("f{i:05}.rs"), options)
            .expect("start file");
        writer.write_all(b"").expect("write file");
    }
    writer.finish().expect("finish zip").into_inner()
}

#[tokio::test]
async fn hosted_index_quota_allows_the_25th_and_refuses_the_26th() {
    let harness = Harness::new();
    let tenant = harness.hosted_key("tenant");
    let tenant = (tenant.0.as_str(), tenant.1.as_str());
    harness.seed_active_indexes(tenant.0, 24);

    // The 25th active index: allowed.
    let (status, body) = harness.upload(tenant, fixture_zip()).await;
    assert_eq!(status, StatusCode::OK, "the 25th index is served: {body}");
    assert!(body["index_id"].is_string());

    // The 26th: refused before any ingest work, naming the limit
    // and the current count.
    let (status, body) = harness.upload(tenant, fixture_zip()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "index_quota_exceeded");
    let message = body["message"].as_str().expect("message");
    assert!(message.contains("25"), "the limit is named: {message}");
    assert!(message.contains("has 25"), "the count is named: {message}");

    // The refusal created nothing: still 25 owned, one real database.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    assert_eq!(
        harness
            .tenancy
            .count_active_for_owner(tenant.0, now)
            .expect("count"),
        25
    );
    assert_eq!(harness.index_db_files().len(), 1);
}

#[tokio::test]
async fn monthly_query_quota_serves_the_10000th_query_and_refuses_the_next() {
    let harness = Harness::new();
    let tenant = harness.hosted_key("tenant");
    let tenant = (tenant.0.as_str(), tenant.1.as_str());
    seed_query_records(&harness, tenant.0, 9_999);

    // Query 10,000 of the month: served.
    let (status, _) = harness.get(tenant, "/v1/schema").await;
    assert_eq!(status, StatusCode::OK, "the 10,000th query is served");

    // Query 10,001: refused, naming the allowance and the month.
    let (status, body) = harness.get(tenant, "/v1/schema").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "query_quota_exceeded");
    let message = body["message"].as_str().expect("message");
    assert!(message.contains("10000"), "the limit is named: {message}");
    assert!(
        message.contains(&current_month()),
        "the month is named: {message}"
    );

    // Index creation is not a query: with the query quota spent, a
    // creation still succeeds — and afterwards queries are still
    // refused (the creation added no query record).
    let (status, body) = harness.upload(tenant, fixture_zip()).await;
    assert_eq!(status, StatusCode::OK, "creation is not a query: {body}");
    let (status, _) = harness.get(tenant, "/v1/schema").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn file_cap_refuses_an_over_cap_tree_and_leaves_nothing_behind() {
    let harness = Harness::new();
    let tenant = harness.hosted_key("tenant");
    let tenant = (tenant.0.as_str(), tenant.1.as_str());

    let (status, body) = harness.upload(tenant, many_files_zip(50_001)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["error"], "file_quota_exceeded");
    assert!(
        body["message"]
            .as_str()
            .expect("message")
            .contains("50000"),
        "the cap is named: {body}"
    );

    // No partial index: no database, no staged tree, no ownership.
    assert!(harness.index_db_files().is_empty());
    assert!(harness.workdirs().is_empty());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    assert_eq!(
        harness
            .tenancy
            .count_active_for_owner(tenant.0, now)
            .expect("count"),
        0
    );
}

#[tokio::test]
async fn self_hosted_key_is_not_enveloped() {
    let harness = Harness::new();
    // Past every hosted number: 30 active indexes, 10,000 queries
    // already metered this month.
    harness.seed_active_indexes(OPERATOR.0, 30);
    seed_query_records(&harness, OPERATOR.0, 10_000);

    let (status, _) = harness.get(OPERATOR, "/v1/schema").await;
    assert_eq!(status, StatusCode::OK, "the operator still queries");

    let (status, body) = harness
        .send(
            OPERATOR,
            Method::POST,
            "/v1/index",
            Some("application/json"),
            serde_json::to_vec(&json!({
                "repo_path": fixture_repo().to_string_lossy(),
            }))
            .expect("serialise body"),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the operator's 31st index is served: {body}"
    );
}
