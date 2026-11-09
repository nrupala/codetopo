// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The public root document (`GET /`) and the boundary around it.
//!
//! The root is the one route the assembled [`app`] serves without a key.
//! These tests pin both halves of that contract: the root answers, in the
//! negotiated shape — and the metered surface behind it is unchanged, so
//! `/v1/schema` still refuses a keyless request and an unknown path is
//! still a 404.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header::ACCEPT, header::CONTENT_TYPE, Request, StatusCode};
use axum::Router;
use serde_json::Value;
use tower::ServiceExt;

use codetopo_server::app;
use codetopo_server::config::Config;
use codetopo_server::keys::KeyStore;
use codetopo_server::metering::Metering;
use codetopo_server::tenancy::Tenancy;

/// A server with its own data dir, keys, and metering log — the same
/// assembly as `tests/api.rs`, minus the fixture indexing.
struct Harness {
    router: Router,
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

        let keys = Arc::new(KeyStore::parse("integration-agent:fixture-secret-value\n"));
        let tenancy = Arc::new(Tenancy::open(&config.tenancy_db_path()).expect("tenancy store"));
        let metering = Arc::new(Metering::new(&config.metering_log));
        Harness {
            router: app(Arc::new(config), keys, metering, tenancy),
            _dir: dir,
        }
    }

    /// Sends a keyless `GET` and returns status, content type, and body.
    async fn get(&self, uri: &str, accept: Option<&str>) -> (StatusCode, Option<String>, String) {
        let mut builder = Request::builder().method("GET").uri(uri);
        if let Some(value) = accept {
            builder = builder.header(ACCEPT, value);
        }
        let response = self
            .router
            .clone()
            .oneshot(builder.body(Body::empty()).expect("build request"))
            .await
            .expect("router response");
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        (status, content_type, String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[tokio::test]
async fn root_answers_json_to_a_machine_caller() {
    let harness = Harness::new();
    let (status, content_type, body) = harness.get("/", Some("application/json")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type.as_deref().is_some_and(|t| t.contains("application/json")),
        "content-type: {content_type:?}"
    );
    let doc: Value = serde_json::from_str(&body).expect("root document is JSON");
    assert_eq!(doc["service"], "codetopo");
    assert_eq!(doc["documentation"]["graph_schema"], "/v1/schema");
    assert_eq!(doc["authentication"]["type"], "bearer");
}

#[tokio::test]
async fn root_answers_html_to_a_browser() {
    let harness = Harness::new();
    let (status, content_type, body) = harness
        .get("/", Some("text/html,application/xhtml+xml"))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type.as_deref().is_some_and(|t| t.contains("text/html")),
        "content-type: {content_type:?}"
    );
    assert!(body.contains("<h1>codetopo API</h1>"), "body: {body}");
    assert!(body.contains("aimlds.org/codetopo"), "body: {body}");
}

#[tokio::test]
async fn root_defaults_to_json_without_an_accept_header() {
    let harness = Harness::new();
    let (status, _, body) = harness.get("/", None).await;
    assert_eq!(status, StatusCode::OK);
    let doc: Value = serde_json::from_str(&body).expect("root document is JSON");
    assert_eq!(doc["service"], "codetopo");
}

#[tokio::test]
async fn the_metered_surface_still_refuses_a_keyless_request() {
    let harness = Harness::new();
    let (status, _, _) = harness.get("/v1/schema", Some("application/json")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unknown_path_is_still_a_bare_404() {
    let harness = Harness::new();
    let (status, _, _) = harness.get("/no-such-path", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
