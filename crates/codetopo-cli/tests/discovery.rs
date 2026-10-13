// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Package/workspace discovery: indexing a monorepo assigns each file
//! to the package its nearest manifest declares (Cargo.toml,
//! package.json, pyproject.toml), names symbols accordingly, and
//! links cross-package imports instead of stubbing them external.

use std::path::{Path, PathBuf};

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("codetopo-discovery-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create tempdir");
    dir
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

#[test]
fn monorepo_files_land_in_their_manifest_packages() {
    let root = tempdir("repo");
    // A Cargo workspace: virtual root manifest (names no package),
    // two member crates; beta uses alpha's function.
    write(&root, "Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    write(
        &root,
        "crates/alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\n",
    );
    write(&root, "crates/alpha/src/lib.rs", "pub fn thing() -> u32 { 1 }\n");
    write(
        &root,
        "crates/beta/Cargo.toml",
        "[package]\nname = \"beta\"\nversion = \"0.1.0\"\n",
    );
    write(
        &root,
        "crates/beta/src/lib.rs",
        "use alpha::thing;\npub fn run() -> u32 { thing() }\n",
    );
    // An npm workspace member.
    write(
        &root,
        "packages/ui/package.json",
        "{ \"name\": \"ui-kit\", \"version\": \"1.0.0\" }\n",
    );
    write(
        &root,
        "packages/ui/src/index.ts",
        "export function button(): string { return 'b'; }\n",
    );
    write(
        &root,
        "packages/web/package.json",
        "{ \"name\": \"web\", \"version\": \"1.0.0\" }\n",
    );
    write(
        &root,
        "packages/web/src/main.ts",
        "import { button } from 'ui-kit';\nexport function page(): string { return button(); }\n",
    );
    // A loose file with no manifest anywhere: the caller's package.
    write(&root, "loose.rs", "pub fn loose() {}\n");

    let db_dir = tempdir("db");
    let db = db_dir.join("mono.db");
    let report = codetopo_cli::index_repo(&root, &db, "rootpkg", None, None)
        .expect("index the monorepo");
    assert_eq!(report.files_failed, 0, "all files index: {report:?}");

    let store = codetopo_store::Store::open(&db).expect("open store");
    let graph = store.load_graph().expect("load graph");
    let packages: Vec<&str> = graph
        .nodes()
        .filter(|n| n.kind == codetopo_core::NodeKind::Package)
        .map(|n| n.id.as_str())
        .collect();
    for expected in ["alpha", "beta", "ui-kit", "web", "rootpkg"] {
        assert!(
            packages.contains(&expected),
            "package {expected} discovered (have {packages:?})"
        );
    }
    assert!(
        !packages.contains(&"workspace"),
        "the virtual workspace root names no package"
    );

    // Symbol ids carry their real package prefixes; locations use
    // the package-relative path (CODE_SCHEMA §3's record form).
    let thing = graph
        .get("alpha::thing")
        .expect("alpha's function id is package-scoped");
    assert_eq!(thing.loc.file, "src/lib.rs");
    assert!(graph.get("beta::run").is_some(), "beta symbol: {:?}", graph.get("beta::run"));
    assert!(
        graph.get("rootpkg::loose::loose").is_some(),
        "loose file keeps the fallback package"
    );

    // Cross-package imports link; nothing about the workspace is
    // external, and no prefix is ever doubled.
    assert!(
        graph.get("external::alpha").is_none() && graph.get("external::ui-kit").is_none(),
        "workspace packages are never external stubs"
    );
    assert!(
        graph.nodes().all(|n| !n.id.contains("external::external::")),
        "no doubled external prefixes anywhere"
    );
    let imports: Vec<(&str, &str)> = graph
        .edges()
        .filter(|e| e.kind == codetopo_core::EdgeKind::Imports)
        .map(|e| (e.from.as_str(), e.to.as_str()))
        .collect();
    assert!(
        imports.iter().any(|(f, t)| *f == "beta" && *t == "alpha"),
        "beta imports alpha's root module: {imports:?}"
    );
    assert!(
        imports.iter().any(|(_, t)| *t == "ui-kit::src"),
        "web imports the ui-kit module defining `button`: {imports:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&db_dir);
}
