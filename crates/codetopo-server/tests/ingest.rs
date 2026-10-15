// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Hosted ingest, end to end over HTTP (paid-path plan S2, gap G1).
//!
//! The properties under test are the ones a paying stranger's safety
//! stands on:
//!
//! * a hosted key cannot name a path on the server host — that mode
//!   belongs to the operator's self-hosted keys alone;
//! * a hosted key *can* get a repository in two ways — an archive
//!   upload (zip and tar.gz here) — and the resulting index is theirs
//!   under the same tenancy rules as any other;
//! * a malicious archive (a traversal entry) is refused and leaves no
//!   files behind, inside or outside the ingest workdir;
//! * `repo_url` refusals are decided before any clone: wrong scheme,
//!   embedded credentials, and non-public hosts are all `400`;
//! * an ingested tree dies with its index: expiry removes the workdir
//!   together with the database.
//!
//! Clone transport itself is covered hermetically in
//! `src/ingest.rs`'s tests (a local `git daemon`): the URL validator
//! rightly refuses loopback, so no HTTP-level test can both obey it
//! and clone from a local fixture.

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
use codetopo_server::metering::Metering;
use codetopo_server::tenancy::Tenancy;

/// The operator's key (legacy import: plan `self-hosted`).
const OPERATOR: (&str, &str) = ("operator", "operator-secret-value");

fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-repo")
}

/// A server whose key store stays reachable, so a test can issue the
/// hosted key the billing bridge would issue.
struct Harness {
    router: Router,
    config: Config,
    keys: Arc<KeyStore>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn with_ttl(index_ttl_secs: Option<u64>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = Config {
            data_dir: dir.path().join("data"),
            keys_file: dir.path().join("api_keys"),
            metering_log: dir.path().join("metering.log"),
            bind: "127.0.0.1:0".parse().expect("bind addr"),
            index_ttl_secs,
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
                tenancy,
            ),
            config,
            keys,
            _dir: dir,
        }
    }

    /// A hosted-plan key, issued the way the billing bridge issues one.
    fn hosted_key(&self, key_id: &str) -> (String, String) {
        let secret = self.keys.issue(key_id, "hosted").expect("issue hosted key");
        (key_id.to_string(), secret)
    }

    async fn send_raw(
        &self,
        key: (&str, &str),
        uri: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder()
            .method(Method::POST)
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

    async fn send_json(
        &self,
        key: (&str, &str),
        uri: &str,
        body: Value,
    ) -> (StatusCode, Value) {
        self.send_raw(
            key,
            uri,
            Some("application/json"),
            serde_json::to_vec(&body).expect("serialise body"),
        )
        .await
    }

    async fn stats(&self, key: (&str, &str), index_id: &str) -> (StatusCode, Value) {
        let builder = Request::builder()
            .method(Method::GET)
            .uri(format!("/v1/graphs/{index_id}/stats"))
            .header(AUTHORIZATION, format!("Bearer {}:{}", key.0, key.1));
        let response = self
            .router
            .clone()
            .oneshot(builder.body(Body::empty()).expect("build request"))
            .await
            .expect("router response");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    fn ingest_dir(&self) -> PathBuf {
        self.config.data_dir.join("ingest")
    }

    /// Whatever ingest workdirs currently exist (names only).
    fn workdirs(&self) -> Vec<String> {
        match std::fs::read_dir(self.ingest_dir()) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// The fixture repository as a zip, wrapped in one top-level directory
/// the way a repository archive conventionally arrives.
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

/// The fixture repository as a .tar.gz, files at the archive root.
fn fixture_tar_gz() -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        for name in ["src/alpha.rs", "src/beta.rs", "src/main.rs"] {
            let bytes = std::fs::read(fixture_repo().join(name)).expect("fixture file");
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, bytes.as_slice())
                .expect("append entry");
        }
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip")
}

/// The plan gate: the whole point of S2's split. A hosted key naming a
/// host path is refused in the API's own vocabulary, and no index or
/// workdir appears.
#[tokio::test]
async fn hosted_key_cannot_index_a_host_path() {
    let harness = Harness::with_ttl(None);
    let hosted = harness.hosted_key("tenant-h");
    let repo = fixture_repo().to_string_lossy().to_string();

    let (status, body) = harness
        .send_json(
            (hosted.0.as_str(), hosted.1.as_str()),
            "/v1/index",
            json!({ "repo_path": repo }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!("invalid_repo_path"));
    assert!(
        body["message"]
            .as_str()
            .expect("message")
            .contains("self-hosted"),
        "the refusal names the way in instead: {body}"
    );
    assert!(harness.workdirs().is_empty(), "no ingest was staged");
}

/// The operator's own key keeps the local-path mode: S2 adds modes, it
/// does not take the existing one away from its owner.
#[tokio::test]
async fn self_hosted_key_keeps_the_local_path_mode() {
    let harness = Harness::with_ttl(None);
    let repo = fixture_repo().to_string_lossy().to_string();
    let (status, body) = harness
        .send_json(OPERATOR, "/v1/index", json!({ "repo_path": repo }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["index_id"].as_str().is_some());
    assert!(harness.workdirs().is_empty(), "a local index stages nothing");
}

/// A hosted key uploads a zip: the index is created, owned by that key,
/// and the tree lives in the index's workdir.
#[tokio::test]
async fn hosted_key_indexes_an_uploaded_zip() {
    let harness = Harness::with_ttl(None);
    let hosted = harness.hosted_key("tenant-h");
    let other = harness.hosted_key("tenant-other");

    let (status, body) = harness
        .send_raw(
            (hosted.0.as_str(), hosted.1.as_str()),
            "/v1/index",
            Some("application/zip"),
            fixture_zip(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["files_indexed"], json!(3), "the fixture's three files");
    let index_id = body["index_id"].as_str().expect("index_id").to_string();

    // The tree is on disk under the index's own workdir, unwrapped.
    let workdir = harness.config.index_workdir_path(&index_id);
    assert!(
        workdir.join("fixture-repo/src/alpha.rs").is_file(),
        "the uploaded tree landed in the workdir"
    );

    // Tenancy applies to ingested indexes exactly as to local ones.
    let (status, _) = harness.stats((hosted.0.as_str(), hosted.1.as_str()), &index_id).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = harness.stats((other.0.as_str(), other.1.as_str()), &index_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!("unknown_index"));
}

/// A .tar.gz upload, this time with the package named on the query
/// string and the content type left generic: magic bytes decide.
#[tokio::test]
async fn hosted_key_indexes_an_uploaded_tar_gz() {
    let harness = Harness::with_ttl(None);
    let hosted = harness.hosted_key("tenant-h");

    let (status, body) = harness
        .send_raw(
            (hosted.0.as_str(), hosted.1.as_str()),
            "/v1/index?package=fixture",
            Some("application/octet-stream"),
            fixture_tar_gz(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["files_indexed"], json!(3));
    let index_id = body["index_id"].as_str().expect("index_id");
    let (status, stats) = harness.stats((hosted.0.as_str(), hosted.1.as_str()), index_id).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["nodes"]["total"], body["nodes"]);
}

/// An operator key may use the hosted modes too — the gate is on the
/// local-path mode only, not a wall between plans and features.
#[tokio::test]
async fn self_hosted_key_can_upload_an_archive() {
    let harness = Harness::with_ttl(None);
    let (status, body) = harness
        .send_raw(OPERATOR, "/v1/index", Some("application/zip"), fixture_zip())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A traversal entry is refused, and the refusal is total: no workdir
/// remains, and nothing was written outside one.
#[tokio::test]
async fn traversal_upload_is_refused_and_leaves_nothing() {
    let harness = Harness::with_ttl(None);
    let hosted = harness.hosted_key("tenant-h");

    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    writer
        .start_file("../evil.rs", options)
        .expect("start file");
    writer.write_all(b"pub fn evil() {}").expect("write file");
    let bytes = writer.finish().expect("finish zip").into_inner();

    let (status, body) = harness
        .send_raw((hosted.0.as_str(), hosted.1.as_str()), "/v1/index", Some("application/zip"), bytes)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!("invalid_body"));
    assert!(harness.workdirs().is_empty(), "the failed ingest was cleaned");
    assert!(
        !harness.config.data_dir.join("evil.rs").exists()
            && !harness.config.data_dir.parent().expect("parent").join("evil.rs").exists(),
        "nothing escaped the workdir"
    );
}

/// A body that is no archive at all, in either mode's clothing.
#[tokio::test]
async fn garbage_upload_is_refused() {
    let harness = Harness::with_ttl(None);
    let hosted = harness.hosted_key("tenant-h");

    let (status, body) = harness
        .send_raw(
            (hosted.0.as_str(), hosted.1.as_str()),
            "/v1/index",
            Some("application/octet-stream"),
            b"this is not an archive".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!("invalid_body"));

    let (status, body) = harness
        .send_raw(
            (hosted.0.as_str(), hosted.1.as_str()),
            "/v1/index",
            Some("application/json"),
            b"{not json".to_vec(),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!("invalid_body"));
}

/// `repo_url` refusals happen before any clone is attempted, for every
/// plan — validation is not a tenant-only courtesy.
#[tokio::test]
async fn repo_url_refusals_precede_any_clone() {
    let harness = Harness::with_ttl(None);
    let hosted = harness.hosted_key("tenant-h");

    for (key, url) in [
        ((hosted.0.as_str(), hosted.1.as_str()), "http://github.com/nrupala/codetopo"),
        ((hosted.0.as_str(), hosted.1.as_str()), "https://127.0.0.1/nrupala/codetopo.git"),
        ((hosted.0.as_str(), hosted.1.as_str()), "https://user:secret@github.com/a/b"),
        ((hosted.0.as_str(), hosted.1.as_str()), "ssh://git@github.com/a/b"),
        (OPERATOR, "https://10.0.0.8/a/b.git"),
        (OPERATOR, "https://localhost/a/b.git"),
    ] {
        let (status, body) = harness
            .send_json(key, "/v1/index", json!({ "repo_url": url }))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {body}");
        assert_eq!(body["error"], json!("invalid_repo_url"), "{url}: {body}");
    }
    assert!(harness.workdirs().is_empty(), "no refusal staged a clone");
}

/// The JSON mode's shape rules: exactly one source, named in the
/// vocabulary of the field that is wrong.
#[tokio::test]
async fn json_mode_requires_exactly_one_source() {
    let harness = Harness::with_ttl(None);
    let repo = fixture_repo().to_string_lossy().to_string();

    let (status, body) = harness
        .send_json(OPERATOR, "/v1/index", json!({}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!("invalid_repo_path"));

    let (status, body) = harness
        .send_json(
            OPERATOR,
            "/v1/index",
            json!({ "repo_path": repo, "repo_url": "https://github.com/a/b" }),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], json!("invalid_body"));
}

/// Expiry takes the ingested tree with the database: the workdir that
/// S2 created is S1's cleanup's business too.
#[tokio::test]
async fn expired_ingested_index_loses_its_workdir() {
    let harness = Harness::with_ttl(Some(0));
    let hosted = harness.hosted_key("tenant-h");

    let (status, body) = harness
        .send_raw(
            (hosted.0.as_str(), hosted.1.as_str()),
            "/v1/index",
            Some("application/zip"),
            fixture_zip(),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let index_id = body["index_id"].as_str().expect("index_id").to_string();
    let workdir = harness.config.index_workdir_path(&index_id);
    assert!(workdir.is_dir(), "the tree exists while the index does");

    let (status, body) = harness.stats((hosted.0.as_str(), hosted.1.as_str()), &index_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], json!("unknown_index"));
    assert!(
        !workdir.exists(),
        "expiry removed the ingested tree with the database"
    );
    assert!(
        !harness.config.index_db_path(&index_id).exists(),
        "expiry removed the database"
    );
}
