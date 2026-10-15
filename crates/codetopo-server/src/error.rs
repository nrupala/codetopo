// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! JSON error type shared by every handler and middleware.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

/// Every failure the API can report, with the JSON body clients receive.
///
/// ## Deviation from the design
///
/// Phase 2 design §7 specified a nested body carrying the status twice
/// (`{"error": {"code", "message", "status"}}`). The body here is flat —
/// `{"error": "<code>", "message": "<text>"}` — and the status is only in the
/// HTTP status line. Rationale: the status is already authoritative on the wire
/// and repeating it in the body is a second source of truth that can disagree
/// with the status line. Flattening also means an agent parses one object with
/// no branch before it can read the machine code.
///
/// `error` is a stable machine code drawn from a closed set
/// ([`ApiError::code`]) and is what a client should branch on. The codes are the
/// ones §7 enumerates; `message` is prose for humans and is not guaranteed to
/// be stable across releases.
///
/// ## Why `NotFound` and `BadRequest` carry their own code
///
/// Both map to a single HTTP status, so the status alone cannot tell an agent
/// what went wrong: `404` covers both "no such index" and "no such node", and
/// `400` covers a missing param, an unparseable param, and a bad body. The
/// status says *what kind* of failure; the code says *which*, which is what
/// lets a client retry sensibly without matching on prose.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// No `Authorization` header, wrong scheme, unknown key, or wrong secret.
    #[error("unauthenticated")]
    Unauthorized,
    /// The request was well-formed but names something that does not exist.
    #[error("{message}")]
    NotFound {
        /// Stable machine code, e.g. `unknown_index`.
        code: &'static str,
        /// Human-readable detail.
        message: String,
    },
    /// The request itself is malformed (missing field, bad value).
    #[error("{message}")]
    BadRequest {
        /// Stable machine code, e.g. `missing_param`.
        code: &'static str,
        /// Human-readable detail.
        message: String,
    },
    /// The request was authenticated and well-formed, but a limit of
    /// the caller's plan refused it (paid-path S3). A distinct status
    /// — not a `400` with a quota-flavoured code — because `429` is
    /// the one answer a client can act on without parsing prose: stop,
    /// wait for the window the message names, retry then.
    #[error("{message}")]
    TooManyRequests {
        /// Stable machine code, e.g. `index_quota_exceeded`.
        code: &'static str,
        /// Human-readable detail.
        message: String,
    },
    /// A query outlived its plan's per-request time limit.
    #[error("{message}")]
    Timeout {
        /// Human-readable detail. One timeout shape exists, so the
        /// code (`request_timeout`) is fixed rather than carried.
        message: String,
    },
    /// Storage, indexing, or an unexpected internal failure.
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    /// 404 `unknown_index`: `{id}` does not name a database in the data dir.
    pub fn unknown_index(id: &str) -> Self {
        ApiError::NotFound {
            code: "unknown_index",
            message: format!("unknown index id '{id}'"),
        }
    }

    /// 404 `unknown_node`: the index loaded, but the node is not in the graph.
    pub fn unknown_node(id: &str) -> Self {
        ApiError::NotFound {
            code: "unknown_node",
            message: format!("unknown node '{id}'"),
        }
    }

    /// 404 `no_path`: no typed path connects the two nodes.
    ///
    /// Not in the §7 table, which does not cover a well-formed `path` query
    /// whose answer is "these two symbols are unrelated". The CLI rejects the
    /// same case rather than returning an empty path, so `404` is preserved and
    /// given its own code instead of being folded into `unknown_index` or
    /// reported as an empty `200`.
    pub fn no_path(from: &str, to: &str) -> Self {
        ApiError::NotFound {
            code: "no_path",
            message: format!("no typed path between '{from}' and '{to}'"),
        }
    }

    /// 400 `missing_param`: a required query parameter is absent or empty.
    pub fn missing_param(name: &str) -> Self {
        ApiError::BadRequest {
            code: "missing_param",
            message: format!("missing required query parameter '{name}'"),
        }
    }

    /// 400 `invalid_param`: a query parameter is present but unusable.
    pub fn invalid_param(name: &str, reason: &str) -> Self {
        ApiError::BadRequest {
            code: "invalid_param",
            message: format!("invalid query parameter '{name}': {reason}"),
        }
    }

    /// 400 `invalid_body`: the request body is not the expected JSON shape.
    pub fn invalid_body(reason: &str) -> Self {
        ApiError::BadRequest {
            code: "invalid_body",
            message: format!("invalid request body: {reason}"),
        }
    }

    /// 400 `invalid_repo_path`: `repo_path` is missing or is not a directory.
    pub fn invalid_repo_path(reason: &str) -> Self {
        ApiError::BadRequest {
            code: "invalid_repo_path",
            message: format!("invalid repo_path: {reason}"),
        }
    }

    /// 400 `invalid_repo_url`: `repo_url` is unparseable, not a public
    /// HTTPS URL, or its clone failed (paid-path S2). The sibling of
    /// `invalid_repo_path` for the hosted ingest mode: one code per
    /// ingest field, so a client can branch on which source was refused
    /// without matching on prose.
    pub fn invalid_repo_url(reason: &str) -> Self {
        ApiError::BadRequest {
            code: "invalid_repo_url",
            message: format!("invalid repo_url: {reason}"),
        }
    }

    /// 429 `index_quota_exceeded`: the key already holds its plan's
    /// active-index limit. Names the limit and the current count, so
    /// the caller knows exactly what must be deleted or expire before
    /// another creation can succeed.
    pub fn index_quota(plan: &str, limit: usize, current: usize) -> Self {
        ApiError::TooManyRequests {
            code: "index_quota_exceeded",
            message: format!(
                "plan '{plan}' allows at most {limit} active indexes; \
                 this key already has {current}"
            ),
        }
    }

    /// 429 `query_quota_exceeded`: the key has used its plan's
    /// monthly query allowance. The month is the UTC calendar month
    /// (see [`crate::limits`]); naming it tells the caller when the
    /// quota resets without a second request to ask.
    pub fn query_quota(plan: &str, limit: u64, used: u64, month: &str) -> Self {
        ApiError::TooManyRequests {
            code: "query_quota_exceeded",
            message: format!(
                "plan '{plan}' allows {limit} queries per month; \
                 this key has used {used} in {month} (UTC)"
            ),
        }
    }

    /// 429 `file_quota_exceeded`: the tree holds more files than the
    /// plan's per-index cap. The count is a lower bound (the walk
    /// stops just past the limit), so the message says "more than".
    pub fn file_quota(plan: &str, limit: usize) -> Self {
        ApiError::TooManyRequests {
            code: "file_quota_exceeded",
            message: format!(
                "the repository tree holds more than {limit} files, \
                 past plan '{plan}'s per-index file limit"
            ),
        }
    }

    /// 429 `index_concurrency_exceeded`: the key already runs its
    /// plan's allowance of concurrent index operations.
    pub fn index_concurrency(plan: &str, limit: usize) -> Self {
        ApiError::TooManyRequests {
            code: "index_concurrency_exceeded",
            message: format!(
                "plan '{plan}' allows {limit} index operation(s) at a time \
                 for one key; one is already running for this key"
            ),
        }
    }

    /// 504 `request_timeout`: the query did not finish within the
    /// plan's per-request limit.
    pub fn request_timeout(plan: &str, secs: u64) -> Self {
        ApiError::Timeout {
            message: format!(
                "the query did not finish within {secs}s, \
                 the per-request limit of plan '{plan}'"
            ),
        }
    }

    /// 500 from any internal failure, carrying `reason`.
    pub fn internal(reason: impl Into<String>) -> Self {
        ApiError::Internal(reason.into())
    }

    fn status(&self) -> StatusCode {
        match self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::NotFound { .. } => StatusCode::NOT_FOUND,
            ApiError::BadRequest { .. } => StatusCode::BAD_REQUEST,
            ApiError::TooManyRequests { .. } => StatusCode::TOO_MANY_REQUESTS,
            ApiError::Timeout { .. } => StatusCode::GATEWAY_TIMEOUT,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The stable machine code clients branch on.
    pub fn code(&self) -> &'static str {
        match self {
            ApiError::Unauthorized => "unauthenticated",
            ApiError::NotFound { code, .. }
            | ApiError::BadRequest { code, .. }
            | ApiError::TooManyRequests { code, .. } => code,
            ApiError::Timeout { .. } => "request_timeout",
            ApiError::Internal(_) => "internal",
        }
    }

    /// The only message a client ever sees for an internal failure.
    ///
    /// The `Internal` variant's payload is operator detail — host
    /// filesystem paths, rusqlite error text, store file names — that
    /// helps the person running the server and must never reach a
    /// client (fleet release gate F-1). [`ApiError::body`] logs the
    /// payload server-side and renders this generic text instead.
    pub const INTERNAL_CLIENT_MESSAGE: &'static str = "internal error";

    /// Body as a JSON value; exposed for tests and for asserting shape.
    ///
    /// This is the single render point for client responses
    /// ([`IntoResponse`] goes through it), so the `Internal` redaction
    /// lives here rather than at the variant's many construction sites:
    /// the detail is written to the server's stderr and the client
    /// receives [`ApiError::INTERNAL_CLIENT_MESSAGE`]. Every other
    /// variant's message is prose about the caller's own request and
    /// renders as before.
    pub fn body(&self) -> serde_json::Value {
        let message = match self {
            ApiError::Internal(detail) => {
                eprintln!("error: internal failure: {detail}");
                ApiError::INTERNAL_CLIENT_MESSAGE.to_string()
            }
            _ => self.to_string(),
        };
        json!({ "error": self.code(), "message": message })
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status(), Json(self.body())).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_matches_the_kind_of_failure() {
        assert_eq!(ApiError::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(ApiError::unknown_index("x").status(), StatusCode::NOT_FOUND);
        assert_eq!(ApiError::unknown_node("x").status(), StatusCode::NOT_FOUND);
        assert_eq!(ApiError::no_path("a", "b").status(), StatusCode::NOT_FOUND);
        assert_eq!(
            ApiError::missing_param("node").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::invalid_param("limit", "not an integer").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::invalid_body("bad json").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::invalid_repo_path("missing").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::invalid_repo_url("not https").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::invalid_repo_url("not https").code(),
            "invalid_repo_url"
        );
        assert_eq!(
            ApiError::internal("boom").status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(
            ApiError::index_quota("hosted", 25, 25).status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            ApiError::request_timeout("hosted", 30).status(),
            StatusCode::GATEWAY_TIMEOUT
        );
    }

    /// The S3 limit refusals: each plan limit has its own code, so a
    /// client can tell "delete an index" from "wait for next month"
    /// from "shrink the repository" without matching on prose.
    #[test]
    fn limit_codes_are_distinct_and_name_the_limit() {
        let index = ApiError::index_quota("hosted", 25, 25);
        assert_eq!(index.code(), "index_quota_exceeded");
        assert!(index.to_string().contains("25"), "limit is named: {index}");
        assert!(
            index.to_string().contains("has 25"),
            "current count is named: {index}"
        );

        let query = ApiError::query_quota("hosted", 10_000, 10_000, "2026-10");
        assert_eq!(query.code(), "query_quota_exceeded");
        assert!(query.to_string().contains("10000"), "{query}");
        assert!(query.to_string().contains("2026-10"), "{query}");

        let file = ApiError::file_quota("hosted", 50_000);
        assert_eq!(file.code(), "file_quota_exceeded");
        assert!(file.to_string().contains("50000"), "{file}");

        let busy = ApiError::index_concurrency("hosted", 1);
        assert_eq!(busy.code(), "index_concurrency_exceeded");

        let timeout = ApiError::request_timeout("hosted", 30);
        assert_eq!(timeout.code(), "request_timeout");
        assert!(timeout.to_string().contains("30s"), "{timeout}");
    }

    /// Every code §7 enumerates must be reachable, so the closed set clients
    /// branch on is actually the set the server can emit.
    #[test]
    fn design_codes_are_all_reachable() {
        for code in [
            "unauthenticated",
            "unknown_index",
            "unknown_node",
            "missing_param",
            "invalid_param",
            "invalid_body",
            "invalid_repo_path",
            "internal",
        ] {
            let err = match code {
                "unauthenticated" => ApiError::Unauthorized,
                "unknown_index" => ApiError::unknown_index("id"),
                "unknown_node" => ApiError::unknown_node("id"),
                "missing_param" => ApiError::missing_param("node"),
                "invalid_param" => ApiError::invalid_param("limit", "out of range"),
                "invalid_body" => ApiError::invalid_body("not json"),
                "invalid_repo_path" => ApiError::invalid_repo_path("not a directory"),
                _ => ApiError::internal("boom"),
            };
            assert_eq!(err.code(), code);
        }
    }

    #[test]
    fn body_is_flat_with_code_and_message() {
        let body = ApiError::unknown_index("abc").body();
        let obj = body.as_object().expect("body is an object");
        assert_eq!(obj.len(), 2, "flat body has exactly error+message: {body}");
        assert_eq!(obj["error"], "unknown_index");
        assert!(obj["message"].as_str().unwrap().contains("abc"));
        // The status is not duplicated into the body (see module docs).
        assert!(body.get("status").is_none());
    }

    /// §7's 404-vs-400 split: naming a nonexistent thing is 404, being
    /// malformed is 400, and the code keeps them apart within a status.
    #[test]
    fn nonexistent_is_404_and_malformed_is_400() {
        assert_eq!(ApiError::unknown_index("i").code(), "unknown_index");
        assert_eq!(ApiError::unknown_node("n").code(), "unknown_node");
        assert_eq!(ApiError::unknown_index("i").status(), StatusCode::NOT_FOUND);
        assert_eq!(ApiError::missing_param("node").code(), "missing_param");
        assert_eq!(
            ApiError::missing_param("node").status(),
            StatusCode::BAD_REQUEST
        );
    }

    /// The rendered message must not leak a secret, and must include the detail
    /// the client needs to fix its own request.
    #[test]
    fn message_carries_detail_not_internals() {
        assert!(ApiError::missing_param("node").to_string().contains("node"));
        assert!(ApiError::invalid_param("limit", "must be 1..=1000")
            .to_string()
            .contains("1..=1000"));
        assert_eq!(ApiError::Unauthorized.to_string(), "unauthenticated");
    }

    /// F-1: the client body for an internal failure is the generic
    /// message — never the variant's payload, which carries host paths
    /// and store error text. The code stays `internal`, the status
    /// stays 500, and no other variant's rendering changes.
    #[test]
    fn internal_body_is_generic_and_detail_free() {
        let err = ApiError::internal("cannot open key store /srv/private/keys.db: disk I/O error");
        let body = err.body();
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], ApiError::INTERNAL_CLIENT_MESSAGE);
        let rendered = body.to_string();
        assert!(
            !rendered.contains("/srv/private"),
            "host path leaked to the client body: {rendered}"
        );
        assert!(
            !rendered.contains("disk I/O"),
            "store error text leaked to the client body: {rendered}"
        );
        // The detail survives in the operator-facing Display — only
        // the client render is redacted.
        assert!(err.to_string().contains("/srv/private/keys.db"));
        // A non-internal body is untouched by the redaction.
        let other = ApiError::unknown_index("abc").body();
        assert!(other["message"].as_str().unwrap().contains("abc"));
    }
}
