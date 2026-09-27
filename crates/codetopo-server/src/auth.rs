// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Bearer-key authentication and per-request metering (Phase 2 design §4, §5).
//!
//! This module is the only place a secret is ever visible: handlers receive a
//! [`KeyId`] and nothing else, so a handler cannot leak a secret by accident
//! and the metering layer has a stable identity to record without touching the
//! credential.
//!
//! Order of operations per request:
//!
//! 1. Read `Authorization` and require the exact form `Bearer <key_id>:<secret>`.
//! 2. Verify against the in-memory [`KeyStore`] in constant time.
//! 3. On success, attach [`KeyId`] to the request extensions.
//! 4. Run the inner service.
//! 5. Measure the response body and append exactly one metering record.
//!
//! A rejected request (step 1 or 2) is answered `401` and is *not* metered. See
//! the deviation note in [`crate::metering`].

use std::sync::Arc;
use std::time::SystemTime;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Router;

use crate::error::ApiError;
use crate::keys::{parse_bearer, KeyStore};
use crate::metering::Metering;

/// The authenticated `key_id`, attached to the request by [`auth_and_meter`].
///
/// Handlers read this to attribute work to a key. The secret is not reachable
/// from here, and neither is any other caller's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyId(pub String);

impl KeyId {
    /// The key identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for KeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Largest response body the middleware will buffer in order to measure it.
///
/// Buffering is the only way to learn a body's length, and an unbounded buffer
/// is a denial-of-service primitive: `snapshot` is whole-graph by design, so a
/// large graph would otherwise pin its full serialisation in RAM for a metric.
/// A body that declares a length past this cap is passed through un-buffered
/// and metered from its `Content-Length`.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Everything the auth + metering layer needs, shared by reference.
#[derive(Debug, Clone)]
pub struct AuthMetering {
    keys: Arc<KeyStore>,
    metering: Arc<Metering>,
}

impl AuthMetering {
    /// Binds a key store and a metering log to one layer.
    pub fn new(keys: Arc<KeyStore>, metering: Arc<Metering>) -> Self {
        AuthMetering { keys, metering }
    }
}

/// Wraps `router` so every route requires a bearer key and is metered.
///
/// There is deliberately no unauthenticated variant, not even for a health
/// route: an unauthenticated route is a hole an agent can probe for free.
pub fn metered<S>(router: Router<S>, auth: Arc<AuthMetering>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    router.route_layer(axum::middleware::from_fn_with_state(auth, auth_and_meter))
}

/// Authenticates the request, runs the inner service, meters the result.
async fn auth_and_meter(
    State(auth): State<Arc<AuthMetering>>,
    mut request: Request,
    next: Next,
) -> Response {
    // One wall-clock reading serves both record fields: `ts` is the request
    // start and `duration_ms` is `started.elapsed()`. A monotonic `Instant`
    // would survive a clock step mid-request, but `ts` is a wall-clock reading
    // by definition, so the one clock is the only one that could be reported.
    let started = SystemTime::now();
    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();

    let presented = parse_bearer(
        request
            .headers()
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    );
    let Some((key_id, secret)) = presented else {
        return unauthorized();
    };
    let key_id = match auth.keys.verify(key_id, secret) {
        Ok(key_id) => key_id,
        Err(_) => return unauthorized(),
    };
    request.extensions_mut().insert(KeyId(key_id.clone()));

    let response = next.run(request).await;
    measure_and_meter(
        auth.metering.as_ref(),
        &key_id,
        &method,
        path,
        response,
        started,
    )
    .await
}

/// A single `401` for every authentication failure.
///
/// The message is intentionally identical for a missing header, a bad scheme, an
/// unknown `key_id`, and a wrong secret, so the response reveals nothing about
/// which keys exist. `WWW-Authenticate` is still sent because that is a
/// protocol requirement, not a disclosure.
fn unauthorized() -> Response {
    let mut response = ApiError::Unauthorized.into_response();
    response.headers_mut().insert(
        axum::http::header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer realm=\"codetopo\""),
    );
    response
}

/// Measures the response body, appends one record, and rebuilds the response.
async fn measure_and_meter(
    metering: &Metering,
    key_id: &str,
    method: &str,
    path: String,
    response: Response,
    started: SystemTime,
) -> Response {
    let status = response.status().as_u16();
    let (parts, body) = response.into_parts();

    let (body, response_bytes) = match declared_length(&parts) {
        // Declared past the cap: do not buffer. The declared length is the best
        // available size and the body is served untouched.
        Some(len) if len > MAX_RESPONSE_BYTES => (body, len as u64),
        _ => match axum::body::to_bytes(body, MAX_RESPONSE_BYTES).await {
            Ok(bytes) => {
                let len = bytes.len() as u64;
                (Body::from(bytes), len)
            }
            // The cap was hit without a declared length to check against, and
            // `to_bytes` has already consumed the body, so it cannot be
            // replayed. Answering 500 is the only honest option: serving an
            // empty 200 would look like a successful empty result.
            Err(err) => {
                eprintln!("warning: response body exceeded the metering buffer cap: {err}");
                return ApiError::Internal(format!(
                    "response exceeded the {MAX_RESPONSE_BYTES}-byte metering buffer limit"
                ))
                .into_response();
            }
        },
    };

    let record = metering.build(key_id, method, &path, status, response_bytes, started);
    // Appended, not propagated: metering is accounting, and failing a served
    // read-only query because the log was momentarily unwritable would trade a
    // correct answer for a correct metric. `append` reports its own failures.
    metering.append(&record);

    Response::from_parts(parts, body)
}

/// The `Content-Length` a response declares, if it declares a valid one.
fn declared_length(parts: &axum::http::response::Parts) -> Option<usize> {
    parts
        .headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn auth(key: &str) -> (Arc<AuthMetering>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let keys = KeyStore::parse(&format!("{key}:s3cr3t\n"));
        let metering = Arc::new(Metering::new(&dir.path().join("metering.log")));
        (Arc::new(AuthMetering::new(Arc::new(keys), metering)), dir)
    }

    fn bearer(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value.parse().expect("header value"));
        headers
    }

    /// Resolves the presented credential against `auth`, as the middleware does.
    fn resolve(auth: &AuthMetering, headers: &HeaderMap) -> Result<String, ApiError> {
        let Some((key_id, secret)) = parse_bearer(
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
        ) else {
            return Err(ApiError::Unauthorized);
        };
        auth.keys.verify(key_id, secret)
    }

    #[test]
    fn key_id_exposes_identifier_not_secret() {
        let key_id = KeyId("research-agent".to_string());
        assert_eq!(key_id.as_str(), "research-agent");
        assert_eq!(key_id.to_string(), "research-agent");
    }

    #[test]
    fn correct_credential_authenticates() {
        let (auth, _dir) = auth("agent");
        assert_eq!(
            resolve(&auth, &bearer("Bearer agent:s3cr3t")).unwrap(),
            "agent"
        );
    }

    #[test]
    fn scheme_is_case_insensitive() {
        let (auth, _dir) = auth("agent");
        assert_eq!(
            resolve(&auth, &bearer("bearer agent:s3cr3t")).unwrap(),
            "agent"
        );
    }

    #[test]
    fn wrong_secret_is_unauthorized() {
        let (auth, _dir) = auth("agent");
        assert!(matches!(
            resolve(&auth, &bearer("Bearer agent:wrong")),
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn unknown_key_is_unauthorized() {
        let (auth, _dir) = auth("agent");
        assert!(matches!(
            resolve(&auth, &bearer("Bearer nobody:s3cr3t")),
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn missing_header_is_unauthorized() {
        let (auth, _dir) = auth("agent");
        assert!(matches!(
            resolve(&auth, &HeaderMap::new()),
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn wrong_scheme_is_unauthorized() {
        let (auth, _dir) = auth("agent");
        assert!(matches!(
            resolve(&auth, &bearer("Basic agent:s3cr3t")),
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn token_without_colon_is_unauthorized() {
        let (auth, _dir) = auth("agent");
        assert!(matches!(
            resolve(&auth, &bearer("Bearer agentonly")),
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn declared_length_reads_content_length() {
        let response = Response::builder()
            .header(CONTENT_LENGTH, "42")
            .body(Body::empty())
            .unwrap();
        let (parts, _body) = response.into_parts();
        assert_eq!(declared_length(&parts), Some(42));
    }

    #[test]
    fn declared_length_absent_when_undeclared() {
        let (parts, _body) = Response::new(Body::empty()).into_parts();
        assert_eq!(declared_length(&parts), None);
    }

    #[test]
    fn metering_records_the_served_body() {
        let (auth, dir) = auth("agent");
        let response = Response::builder()
            .status(200)
            .header(CONTENT_LENGTH, "5")
            .body(Body::from("hello"))
            .unwrap();
        let out = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(measure_and_meter(
                auth.metering.as_ref(),
                "agent",
                "GET",
                "/v1/graphs/x/stats".to_string(),
                response,
                SystemTime::now(),
            ));
        assert_eq!(out.status(), 200);
        let log = std::fs::read_to_string(dir.path().join("metering.log")).unwrap();
        let record: serde_json::Value = serde_json::from_str(log.trim()).unwrap();
        assert_eq!(record["key_id"], "agent");
        assert_eq!(record["method"], "GET");
        assert_eq!(record["path"], "/v1/graphs/x/stats");
        assert_eq!(record["status"], 200);
        assert_eq!(record["response_bytes"], 5);
    }
}
