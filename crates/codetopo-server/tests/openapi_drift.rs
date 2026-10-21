// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Drift gate: `schemas/openapi.yaml` must describe the HTTP surface
//! this crate actually serves — same version, same route set. Add a
//! route in `handlers.rs` (or bump the crate) without updating the
//! published contract, and CI fails here. The route list below is the
//! test's half of the contract: update it and the YAML together.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Every route the router serves, in OpenAPI path form (`{id}`).
const EXPECTED_ROUTES: &[&str] = &[
    "/v1/graphs/{id}/ancestors",
    "/v1/graphs/{id}/blast-radius",
    "/v1/graphs/{id}/descendants",
    "/v1/graphs/{id}/diagnostics",
    "/v1/graphs/{id}/neighborhood",
    "/v1/graphs/{id}/path",
    "/v1/graphs/{id}/search",
    "/v1/graphs/{id}/snapshot",
    "/v1/graphs/{id}/stats",
    "/v1/graphs/{id}/symbol",
    "/v1/graphs/{id}/verify",
    "/v1/index",
    "/v1/schema",
];

fn openapi_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/openapi.yaml");
    std::fs::read_to_string(path).expect("schemas/openapi.yaml must exist")
}

/// The `info.version` value: the first `version:` inside the `info:`
/// block (top-level keys end the block).
fn info_version(text: &str) -> Option<String> {
    let mut in_info = false;
    for line in text.lines() {
        if line == "info:" {
            in_info = true;
            continue;
        }
        if in_info {
            if !line.starts_with(' ') && !line.is_empty() {
                break; // next top-level key
            }
            let trimmed = line.trim();
            if let Some(v) = trimmed.strip_prefix("version:") {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// The path keys declared under `paths:` (two-space-indented keys
/// ending in `:`), in declaration order.
fn declared_paths(text: &str) -> BTreeSet<String> {
    let mut in_paths = false;
    let mut out = BTreeSet::new();
    for line in text.lines() {
        if line == "paths:" {
            in_paths = true;
            continue;
        }
        if in_paths {
            if !line.starts_with(' ') && !line.is_empty() {
                break; // next top-level key (e.g. components:)
            }
            if let Some(rest) = line.strip_prefix("  ") {
                if !rest.starts_with(' ') && rest.ends_with(':') {
                    out.insert(rest.trim_end_matches(':').to_string());
                }
            }
        }
    }
    out
}

#[test]
fn openapi_version_matches_the_crate() {
    let text = openapi_text();
    assert_eq!(
        info_version(&text).as_deref(),
        Some(env!("CARGO_PKG_VERSION")),
        "schemas/openapi.yaml info.version has drifted from the crate version"
    );
}

#[test]
fn openapi_paths_match_the_router() {
    let text = openapi_text();
    let declared = declared_paths(&text);
    let expected: BTreeSet<String> = EXPECTED_ROUTES.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        declared, expected,
        "schemas/openapi.yaml paths have drifted from the routes in handlers.rs \
         (update the YAML and EXPECTED_ROUTES together)"
    );
}
