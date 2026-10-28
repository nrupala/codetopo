// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

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
/// builder classifies tier by node kind (CODE_SCHEMA §2: the
/// package/module/file skeleton is `core`, type declarations are
/// `recommended`, everything else `optional`). Symbol `id`s are taken
/// verbatim as node ids.
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
    /// Symbols referenced here (marimo contract). Each entry is a value
    /// *read*; entries resolve best-effort into `reads` edges;
    /// unresolvable refs are silent.
    #[serde(default)]
    pub refs: Vec<String>,
    /// Symbols *written* here (assignment targets, by name). Each entry
    /// resolves best-effort into a `writes` edge; unresolvable writes
    /// are silent, like refs.
    #[serde(default)]
    pub writes: Vec<String>,
    /// Last source line of the symbol's declaration (1-based, inclusive)
    /// — CODE_SCHEMA §3, schema 1.1. `None` when the extractor did not
    /// measure it; absence means *not measured*, never a span of zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    /// The declaration text of the symbol — body and initializer
    /// excluded, whitespace collapsed (CODE_SCHEMA §3). `None` when not
    /// measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Decision points counted in the symbol's body, per the
    /// per-language enumeration in CODE_SCHEMA §3.2. Emitted only for
    /// function/method symbols that have a body. `None` means *not
    /// measured* (body-less declaration, non-callable symbol, or an
    /// extractor that does not count) — never read it as zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions: Option<u32>,
}

/// One inheritance or implementation claim found in a file: the named
/// type (`type_name`) inherits from or implements the named target
/// (`target_name`) — Rust `impl Trait for Type`, `trait A: B`, TS
/// `class X extends Y` / `implements Z`, `interface A extends B`.
/// Names resolve at build time with the same best-effort rules as
/// refs; unresolvable heritage is silent. `kind` is only ever
/// [`EdgeKind::Inherits`] or [`EdgeKind::Implements`] — the builder
/// ignores anything else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawHeritage {
    pub type_name: String,
    pub target_name: String,
    pub kind: crate::schema::EdgeKind,
    pub line: u32,
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
    #[serde(default)]
    pub heritage: Vec<RawHeritage>,
}
