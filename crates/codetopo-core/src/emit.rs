// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Extractor output contract.
//!
//! This is what language extractors (worker W3) produce, one value per file.
//! The contract is deliberately minimal (CODE_SCHEMA §1, principle 2 —
//! "minimal write path"): emitting a symbol is trivial; richness lives in the
//! derived graph views, never in what the extractor must produce.
//!
//! `GraphBuilder::add_file` validates a `FileSymbols` value and then applies
//! it; `GraphBuilder::build` resolves the cross-file references (`imports`,
//! `calls`, `refs`).

use serde::{Deserialize, Serialize};

use crate::schema::NodeKind;

/// One symbol found in a file.
///
/// Note: there is no `tier` field in this version of the contract — the
/// builder assigns [`crate::schema::Tier::Optional`] ("not classified") and
/// documents it. Symbol `id`s are taken verbatim as node ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawSymbol {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    /// Symbols defined here (marimo contract).
    #[serde(default)]
    pub defines: Vec<String>,
    /// Symbols referenced here (marimo contract). Each entry is resolved
    /// best-effort into a `references` edge; unresolvable refs are silent.
    #[serde(default)]
    pub refs: Vec<String>,
}

/// One module-level import statement.
///
/// `from_module_id` must be a known module (it is normally the file's own
/// `module_id`); `to_module_id` either links to a known module or becomes an
/// `external::{to_module_id}` stub — external dependencies are normal and
/// produce no diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawImport {
    pub from_module_id: String,
    pub to_module_id: String,
    #[serde(default)]
    pub bindings: Vec<String>,
    pub line: u32,
}

/// One call site. `to_name` is resolved at build time: exact symbol-id match
/// first, then `::{to_name}` suffix match (same module → same package → any,
/// deterministic), else an `external::{to_name}` stub plus a single deduped
/// `UnresolvedCall` diagnostic per distinct target name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawCall {
    pub from_id: String,
    pub to_name: String,
    pub line: u32,
}

/// Everything one extractor run produces for one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSymbols {
    pub package: String,
    pub file: String,
    pub module_id: String,
    #[serde(default)]
    pub symbols: Vec<RawSymbol>,
    #[serde(default)]
    pub imports: Vec<RawImport>,
    #[serde(default)]
    pub calls: Vec<RawCall>,
}
