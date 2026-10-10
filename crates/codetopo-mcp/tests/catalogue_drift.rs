// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Drift gate: `schemas/mcp-tools.json` must equal the live tool
//! catalogue, byte-for-semantics. The file is generated from the
//! server (see its `$comment`), and this test is what keeps
//! "generated once" from silently becoming "stale forever": change a
//! tool, its description, or its input schema — or bump the crate
//! version — without regenerating the file, and CI fails here.

use std::path::PathBuf;

fn catalogue_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/mcp-tools.json")
}

#[test]
fn published_catalogue_matches_the_live_server() {
    let text = std::fs::read_to_string(catalogue_path())
        .expect("schemas/mcp-tools.json must exist");
    let published: serde_json::Value =
        serde_json::from_str(&text).expect("mcp-tools.json parses");

    // The recorded provenance must describe this build.
    assert_eq!(
        published["source"]["serverInfo"]["name"], "codetopo",
        "catalogue source names the server"
    );
    assert_eq!(
        published["source"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION"),
        "catalogue was generated from a different crate version — regenerate it"
    );

    // The tools array must be exactly the live catalogue.
    let live = serde_json::Value::Array(codetopo_mcp::tool_catalog());
    assert_eq!(
        published["tools"], live,
        "schemas/mcp-tools.json has drifted from tool_catalog() — regenerate it \
         (see the $comment in the file for the command)"
    );
}
