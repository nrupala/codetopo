// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Tree-sitter extractors: Rust and TypeScript/JavaScript source files into
//! [`codetopo_core::emit::FileSymbols`].
//!
//! One tree-sitter parse per file, then a single recursive walk over the
//! syntax tree. The walk never panics on odd input: unparseable shapes are
//! skipped, and a hard parse failure surfaces as [`ExtractError`] so the
//! caller can skip the file with a warning.
//!
//! ID rules, node-kind mappings, and known limitations are documented in the
//! crate README (`crates/codetopo-extract/README.md`).

mod rust;
mod ts;

use std::path::Path;

/// Options for a future batch-oriented extraction API.
///
/// Kept minimal today: [`extract_file`] takes the package name directly.
/// The struct exists so callers can thread configuration through without
/// changing signatures later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractOptions {
    pub package: String,
}

/// What went wrong while extracting one file.
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    /// The file extension is not one this crate handles.
    #[error("unsupported file extension: {0}")]
    UnsupportedExtension(String),
    /// tree-sitter could not produce a syntax tree at all.
    ///
    /// Note: files that parse *with errors* (the common case for
    /// half-written code) do **not** fail here — the walk is error-tolerant
    /// and extracts what it can. Only a total parse failure returns `Err`.
    #[error("parse failed for {0}: {1}")]
    ParseFailed(String, String),
    /// Something unexpected inside the extractor itself (never bad input).
    #[error("extractor internal error: {0}")]
    Internal(String),
}

/// File extensions this crate can extract (case-insensitive).
const SUPPORTED: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "mts", "cts"];

/// True when `path` names a file this crate can extract.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| SUPPORTED.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Extract the symbols, imports, and calls from one source file.
///
/// * `package` — the owning package name (crate / npm package); becomes the
///   first segment of every emitted id.
/// * `rel_path` — path of the file *relative to the package root*, e.g.
///   `src/net.rs`. Drives language selection and the module id.
/// * `source` — the file's text.
///
/// Returns [`ExtractError::UnsupportedExtension`] for unknown extensions and
/// [`ExtractError::ParseFailed`] when tree-sitter produces no tree at all.
pub fn extract_file(
    package: &str,
    rel_path: &Path,
    source: &str,
) -> Result<FileSymbols, ExtractError> {
    if !is_supported(rel_path) {
        let ext = rel_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        return Err(ExtractError::UnsupportedExtension(ext));
    }
    let ext = rel_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "rs" => rust::extract(package, rel_path, source),
        // `.tsx`/`.jsx` need the TSX grammar (JSX syntax); the rest of the
        // JS/TS family parses with the plain TypeScript grammar.
        "tsx" | "jsx" => ts::extract(package, rel_path, source, true),
        _ => ts::extract(package, rel_path, source, false),
    }
}

/// Re-export the core emit types so callers need only this crate.
pub use codetopo_core::emit::{FileSymbols, RawCall, RawImport, RawSymbol};

// ---------------------------------------------------------------------------
// Shared walk helpers (crate-private).
// ---------------------------------------------------------------------------

/// Text slice for a node, or `None` on invalid UTF-8 (caller skips the node).
pub(crate) fn text_of<'a>(node: tree_sitter::Node, src: &'a [u8]) -> Option<&'a str> {
    node.utf8_text(src).ok()
}

/// 1-based line number of a node's start.
pub(crate) fn line_of(node: tree_sitter::Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Named children of a node, collected so the walker can recurse while
/// holding `&mut self` (the borrowing iterator can't coexist with that).
pub(crate) fn children(node: tree_sitter::Node) -> Vec<tree_sitter::Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// File path as a forward-slash string for [`FileSymbols::file`].
pub(crate) fn display_path(rel_path: &Path) -> String {
    rel_path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract_file;

    #[test]
    fn supported_extensions() {
        for ext in ["rs", "ts", "tsx", "js", "jsx", "mts", "cts", "RS", "TS"] {
            assert!(
                is_supported(Path::new(&format!("a/b.{ext}"))),
                "{ext} should be supported"
            );
        }
    }

    #[test]
    fn unsupported_extensions() {
        for ext in ["py", "go", "md", "toml", "json", ""] {
            assert!(
                !is_supported(Path::new(&format!("a/b.{ext}"))),
                "{ext} should not be supported"
            );
        }
        assert!(!is_supported(Path::new("Makefile")));
    }

    #[test]
    fn unsupported_extension_is_an_error_not_a_panic() {
        let err = extract_file("pkg", Path::new("x.py"), "print(1)").unwrap_err();
        assert!(matches!(err, ExtractError::UnsupportedExtension(_)));
    }

    #[test]
    fn garbage_input_never_panics() {
        // Deliberately broken Rust and TS: must not panic, may return partial output.
        let _ = extract_file("pkg", Path::new("a.rs"), "fn broken( { struct ");
        let _ = extract_file("pkg", Path::new("a.ts"), "class {{{ function ((");
    }
}
