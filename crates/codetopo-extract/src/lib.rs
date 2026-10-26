// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

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
const SUPPORTED: &[&str] = &["rs", "ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs", "md"];

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
        // Markdown is documentation, not code: it enters the graph as a
        // single Doc node so docs sit in the map next to what they
        // describe (CODE_SCHEMA §2 has the kind; nothing emitted it).
        "md" => Ok(extract_doc(package, rel_path)),
        // `.tsx`/`.jsx` need the TSX grammar (JSX syntax); the rest of the
        // JS/TS family parses with the plain TypeScript grammar.
        "tsx" | "jsx" => ts::extract(package, rel_path, source, true),
        _ => ts::extract(package, rel_path, source, false),
    }
}

/// A Markdown file as one Doc symbol. The module id follows the Rust
/// convention loosely (`docs/guide.md` -> `{pkg}::docs::guide`); the
/// symbol id appends the file stem once more so it cannot collide with
/// the module node. Content is not parsed — a doc node marks presence
/// and place, and the file text stays the documentation itself.
fn extract_doc(package: &str, rel_path: &Path) -> FileSymbols {
    let file = display_path(rel_path);
    let stem = rel_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .to_string();
    let mut comps: Vec<String> = rel_path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str().map(|s| s.to_string()),
            _ => None,
        })
        .collect();
    if let Some(last) = comps.last_mut() {
        *last = stem.clone();
    }
    let module_id = if comps.is_empty() {
        package.to_string()
    } else {
        format!("{}::{}", package, comps.join("::"))
    };
    FileSymbols {
        package: package.to_string(),
        file,
        module_id: module_id.clone(),
        symbols: vec![codetopo_core::RawSymbol {
            id: format!("{module_id}::{stem}"),
            kind: codetopo_core::NodeKind::Doc,
            label: stem,
            line: 1,
            column: None,
            defines: Vec::new(),
            refs: Vec::new(),
            writes: Vec::new(),
            end_line: None,
            signature: None,
            decisions: None,
        }],
        imports: Vec::new(),
        calls: Vec::new(),
        heritage: Vec::new(),
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

/// 1-based line number of a node's end (inclusive) — CODE_SCHEMA §3.
pub(crate) fn end_line_of(node: tree_sitter::Node) -> u32 {
    node.end_position().row as u32 + 1
}

/// Declaration text of a symbol node (CODE_SCHEMA §3): the node's own
/// text up to `cut` (its body / initializer / aliased-type node),
/// whitespace runs collapsed to single spaces, with a trailing `{`,
/// `;`, or `=` left by the cut — or terminating a body-less
/// declaration — stripped. `cut` is `None` when the whole node text is
/// the declaration (trait method signatures, type aliases).
pub(crate) fn signature_of(
    node: tree_sitter::Node,
    cut: Option<tree_sitter::Node>,
    src: &[u8],
) -> Option<String> {
    let start = node.start_byte();
    let end = cut.map(|c| c.start_byte()).unwrap_or(node.end_byte());
    let raw = std::str::from_utf8(src.get(start..end)?).ok()?;
    let mut sig = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    while let Some(stripped) = sig.strip_suffix(['{', ';', '=']) {
        sig = stripped.trim_end().to_string();
    }
    if sig.is_empty() {
        None
    } else {
        Some(sig)
    }
}

/// The CODE_SCHEMA §3 measured fields of one symbol node: end line and
/// signature for every symbol. The signature cut is the node's `body`
/// field, else its `value` (constant / declarator initializer), else
/// its `type` (type alias target) — field names shared by both
/// grammars. Decision counting is per-language (CODE_SCHEMA §3.2) and
/// happens in the walkers; this helper leaves it `None`.
pub(crate) fn measured_fields(
    node: tree_sitter::Node,
    src: &[u8],
) -> (Option<u32>, Option<String>) {
    let cut = node
        .child_by_field_name("body")
        .or_else(|| node.child_by_field_name("value"))
        .or_else(|| node.child_by_field_name("type"));
    (Some(end_line_of(node)), signature_of(node, cut, src))
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
        for ext in ["rs", "ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs", "md", "RS", "TS"] {
            assert!(
                is_supported(Path::new(&format!("a/b.{ext}"))),
                "{ext} should be supported"
            );
        }
    }

    #[test]
    fn unsupported_extensions() {
        for ext in ["py", "go", "toml", "json", ""] {
            assert!(
                !is_supported(Path::new(&format!("a/b.{ext}"))),
                "{ext} should not be supported"
            );
        }
        assert!(!is_supported(Path::new("Makefile")));
    }

    #[test]
    fn markdown_extracts_as_a_doc_node() {
        let fs = extract_file("pkg", Path::new("docs/guide.md"), "# Guide\n").unwrap();
        assert_eq!(fs.symbols.len(), 1);
        assert_eq!(fs.symbols[0].kind, codetopo_core::NodeKind::Doc);
        assert_eq!(fs.symbols[0].label, "guide");
        assert_eq!(fs.module_id, "pkg::docs::guide");
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
