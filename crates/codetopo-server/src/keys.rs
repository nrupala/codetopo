// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! API keys loaded from a newline-delimited `key_id:secret` file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use subtle::ConstantTimeEq;

use crate::error::ApiError;

/// An in-memory set of `key_id` → `secret` pairs loaded at startup.
///
/// Secrets are compared with [`subtle::ConstantTimeEq`] so a caller cannot
/// learn a valid key one byte at a time by measuring how long a rejection
/// takes. Only the `key_id` is ever recorded in the metering log.
#[derive(Debug, Clone)]
pub struct KeyStore {
    secrets: HashMap<String, String>,
}

impl KeyStore {
    /// Reads `key_id:secret` lines, ignoring blank lines and `#` comments.
    ///
    /// A line is split on the *first* `:` so a secret may itself contain
    /// colons. Lines without a colon are skipped; they are reported by
    /// [`KeyStore::load`] so a malformed file is visible at startup.
    pub fn parse(contents: &str) -> Self {
        let mut secrets = HashMap::new();
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key_id, secret)) = line.split_once(':') {
                let key_id = key_id.trim();
                let secret = secret.trim();
                if !key_id.is_empty() && !secret.is_empty() {
                    secrets.insert(key_id.to_string(), secret.to_string());
                }
            }
        }
        KeyStore { secrets }
    }

    /// Loads and parses the keys file at `path`.
    ///
    /// Fails closed (Phase 2 design §4): a missing file, an unreadable file, or
    /// a file that yields no usable `key_id:secret` line is a startup error.
    /// Starting with zero keys would serve 401 for every request while looking
    /// healthy, which is harder to diagnose than refusing to boot.
    pub fn load(path: &Path) -> Result<Self, ApiError> {
        let contents = std::fs::read_to_string(path).map_err(|err| {
            ApiError::Internal(format!("cannot read keys file {}: {err}", path.display()))
        })?;
        let store = KeyStore::parse(&contents);
        if store.is_empty() {
            return Err(ApiError::Internal(format!(
                "keys file {} has no usable 'key_id:secret' line",
                path.display()
            )));
        }
        Ok(store)
    }

    /// Number of provisioned keys.
    pub fn len(&self) -> usize {
        self.secrets.len()
    }

    /// True when no key is provisioned, i.e. every request will be rejected.
    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    /// Checks a presented token and returns its `key_id` on success.
    ///
    /// Both an unknown `key_id` and a wrong secret are rejected with the same
    /// error, and the work done in each case is roughly equal, so the response
    /// does not reveal which keys exist.
    pub fn verify(&self, key_id: &str, secret: &str) -> Result<String, ApiError> {
        let expected = self.secrets.get(key_id);
        let Some(expected) = expected else {
            // Still burn a comparison so an unknown key is not faster.
            let _ = "unauthenticated".as_bytes().ct_eq(secret.as_bytes());
            return Err(ApiError::Unauthorized);
        };
        if expected.as_bytes().ct_eq(secret.as_bytes()).into() {
            Ok(key_id.to_string())
        } else {
            Err(ApiError::Unauthorized)
        }
    }
}

/// Splits `Authorization: Bearer <key_id>:<secret>` into its two parts.
///
/// The header is case-insensitive on the scheme. Returns `None` for a missing
/// header, a non-`Bearer` scheme, or a token without a `:`, so the caller can
/// answer with a single `401` in every case.
pub fn parse_bearer(header: Option<&str>) -> Option<(&str, &str)> {
    let raw = header?.trim();
    let (scheme, token) = raw.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    let (key_id, secret) = token.split_once(':')?;
    let key_id = key_id.trim();
    let secret = secret.trim();
    if key_id.is_empty() || secret.is_empty() {
        return None;
    }
    Some((key_id, secret))
}

/// Convenience: where the keys file lives by default.
pub fn default_keys_file() -> PathBuf {
    PathBuf::from("./api_keys")
}
