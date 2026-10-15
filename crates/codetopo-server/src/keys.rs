// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Runtime API key store (paid-path plan S1, gap G3).
//!
//! Keys live in a SQLite database in the data dir (`keys.db`), not in a
//! flat file loaded once at startup: a key can be issued, rotated, and
//! revoked while the server runs, by writing to the store — the admin
//! subcommand (`codetopo-server keys …`) does exactly that, and the
//! server's next verification sees the change with no restart.
//!
//! A secret is never stored. The row carries a per-key random salt and
//! `SHA-256(salt ‖ secret)`; verification recomputes the hash of the
//! presented secret and compares hashes with [`subtle::ConstantTimeEq`],
//! so the wire contract (`Authorization: Bearer <key_id>:<secret>`) and
//! its constant-time property are unchanged — only the storage moved.
//!
//! The legacy flat `key_id:secret` file is an import source, not a store:
//! at startup, when the database holds no active key and the file exists,
//! its entries are imported (hashed, plan [`SELF_HOSTED_PLAN`]). Existing
//! single-host deployments therefore keep working untouched, and the file
//! can be deleted once the import has happened. One value never imports:
//! the documented example secret the README quickstart once printed
//! ([`DOCUMENTED_EXAMPLE_SECRET`]) — it is public, so the import refuses
//! any file still carrying it.
//!
//! Keys are provisioned with a `plan` name (the paid tier's plans arrive
//! with the billing bridge; the store only records the string) and a
//! lifecycle `status`: `active` or `revoked`. Only `active` keys verify.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::error::ApiError;

/// File name of the key store inside the data dir.
pub const STORE_FILE_NAME: &str = "keys.db";

/// Plan recorded for keys that predate plans: the operator's own keys
/// (legacy flat-file import, [`KeyStore::parse`]), not a billed tenant's.
pub const SELF_HOSTED_PLAN: &str = "self-hosted";

/// The secret the README quickstart printed as its example credential
/// (`demo-key:change-me`) until the release-gate fix loop. It is public
/// knowledge — anyone who has read the README knows it — so a keys file
/// still carrying it would hand the server to the first caller who
/// tries it. [`KeyStore::import_legacy_file`] refuses it by name
/// (fleet release gate F-3).
pub const DOCUMENTED_EXAMPLE_SECRET: &str = "change-me";

/// Plan a key is issued with when the admin names none: the hosted tier
/// from the paid-path plan.
pub const DEFAULT_PLAN: &str = "hosted";

/// Lifecycle status of a key that may authenticate.
const STATUS_ACTIVE: &str = "active";
/// Lifecycle status of a key that may no longer authenticate.
const STATUS_REVOKED: &str = "revoked";

/// What a successful verification yields: the key's identity and plan.
///
/// The secret is not reachable from here — handlers and the metering
/// layer see only this, exactly as they saw only the `key_id` before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedKey {
    /// The authenticated `key_id`.
    pub key_id: String,
    /// The plan recorded for the key at issuance.
    pub plan: String,
}

/// One key's operator-visible state. Never carries a secret or a hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySummary {
    /// The key's identifier.
    pub key_id: String,
    /// The plan recorded for the key.
    pub plan: String,
    /// `active` or `revoked`.
    pub status: String,
    /// Unix seconds at issuance.
    pub created_at: i64,
    /// Unix seconds at the last rotation, if the secret was ever rotated.
    pub rotated_at: Option<i64>,
}

/// One `api_keys` row, as verification reads it.
struct KeyRow {
    secret_hash: Vec<u8>,
    salt: Vec<u8>,
    plan: String,
    status: String,
}

/// The key store: one SQLite connection, serialised behind a mutex.
///
/// Verification is one indexed lookup per request — trivial next to the
/// whole-graph load every query already performs — and reading the store
/// on every request is what makes runtime issuance, rotation, and
/// revocation visible without a restart or an invalidation protocol.
/// The database runs in WAL mode with a busy timeout so the admin
/// subcommand can write while the server reads.
#[derive(Debug)]
pub struct KeyStore {
    conn: Mutex<Connection>,
}

impl KeyStore {
    /// Opens (creating if needed) the key store database at `path`.
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        let conn = Connection::open(path).map_err(|err| {
            ApiError::Internal(format!(
                "cannot open key store {}: {err}",
                path.display()
            ))
        })?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS api_keys (
                 key_id      TEXT PRIMARY KEY,
                 secret_hash BLOB NOT NULL,
                 salt        BLOB NOT NULL,
                 plan        TEXT NOT NULL,
                 status      TEXT NOT NULL,
                 created_at  INTEGER NOT NULL,
                 rotated_at  INTEGER
             );",
        )
        .map_err(|err| ApiError::Internal(format!("cannot initialise key store: {err}")))?;
        Ok(KeyStore {
            conn: Mutex::new(conn),
        })
    }

    /// An in-memory store, for [`KeyStore::parse`] and tests.
    fn in_memory() -> Result<Self, ApiError> {
        let conn = Connection::open_in_memory()
            .map_err(|err| ApiError::Internal(format!("cannot open in-memory key store: {err}")))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS api_keys (
                 key_id      TEXT PRIMARY KEY,
                 secret_hash BLOB NOT NULL,
                 salt        BLOB NOT NULL,
                 plan        TEXT NOT NULL,
                 status      TEXT NOT NULL,
                 created_at  INTEGER NOT NULL,
                 rotated_at  INTEGER
             );",
        )
        .map_err(|err| ApiError::Internal(format!("cannot initialise key store: {err}")))?;
        Ok(KeyStore {
            conn: Mutex::new(conn),
        })
    }

    /// Builds a store from legacy flat-file contents.
    ///
    /// Reads `key_id:secret` lines, ignoring blank lines and `#` comments.
    /// A line is split on the *first* `:` so a secret may itself contain
    /// colons. Lines without a colon are skipped. Imported keys are
    /// `active` and carry plan [`SELF_HOSTED_PLAN`]. Building an in-memory
    /// SQLite database cannot fail in any way a caller could act on, so
    /// this keeps the infallible signature the original parser had.
    pub fn parse(contents: &str) -> Self {
        let store = KeyStore::in_memory().expect("in-memory key store");
        for (key_id, secret) in parse_pairs(contents) {
            store
                .insert(&key_id, &secret, SELF_HOSTED_PLAN)
                .expect("insert into a fresh in-memory store");
        }
        store
    }

    /// Imports a legacy flat keys file into this store.
    ///
    /// Keys already present (by `key_id`) are left untouched — an import
    /// must never resurrect a revoked key or overwrite a rotated secret.
    /// Returns how many keys were newly imported.
    ///
    /// The import refuses the whole file, before importing anything,
    /// when any entry carries [`DOCUMENTED_EXAMPLE_SECRET`] — the
    /// example credential the README quickstart used to print. That
    /// value is published, so importing it would provision a key whose
    /// secret every reader of the old README knows (gate F-3). Failing
    /// the whole import, rather than skipping the one entry, keeps the
    /// refusal loud: a half-imported file that boots with fewer keys
    /// than the operator wrote is a quieter failure than not booting.
    pub fn import_legacy_file(&self, path: &Path) -> Result<usize, ApiError> {
        let contents = std::fs::read_to_string(path).map_err(|err| {
            ApiError::Internal(format!("cannot read keys file {}: {err}", path.display()))
        })?;
        let pairs = parse_pairs(&contents);
        if let Some((key_id, _)) = pairs
            .iter()
            .find(|(_, secret)| secret == DOCUMENTED_EXAMPLE_SECRET)
        {
            return Err(ApiError::Internal(format!(
                "keys file {} carries key '{key_id}' with the documented example secret \
                 '{DOCUMENTED_EXAMPLE_SECRET}' — that value was printed in the README \
                 quickstart and is public; generate a real secret instead \
                 (e.g. `openssl rand -hex 16`)",
                path.display()
            )));
        }
        let mut imported = 0;
        for (key_id, secret) in pairs {
            if self.lookup(&key_id)?.is_none() {
                self.insert(&key_id, &secret, SELF_HOSTED_PLAN)?;
                imported += 1;
            }
        }
        Ok(imported)
    }

    /// Issues a new key and returns its secret — the only time the secret
    /// exists outside the caller's hands, so the caller must record it
    /// now. Fails if `key_id` is already taken or is unusable in a bearer
    /// token (empty, or containing `:` or whitespace).
    pub fn issue(&self, key_id: &str, plan: &str) -> Result<String, ApiError> {
        validate_key_id(key_id)?;
        if self.lookup(key_id)?.is_some() {
            return Err(ApiError::BadRequest {
                code: "key_exists",
                message: format!("key '{key_id}' already exists"),
            });
        }
        let secret = generate_secret();
        self.insert(key_id, &secret, plan)?;
        Ok(secret)
    }

    /// Replaces a key's secret and returns the new one. The old secret
    /// stops verifying with this call. The key's plan, status, and
    /// creation time are unchanged. A revoked key cannot be rotated back
    /// to life — issue a new key instead.
    pub fn rotate(&self, key_id: &str) -> Result<String, ApiError> {
        let conn = self.lock()?;
        let status = conn
            .query_row(
                "SELECT status FROM api_keys WHERE key_id = ?1",
                rusqlite::params![key_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| ApiError::Internal(format!("cannot read key store: {err}")))?;
        match status.as_deref() {
            None => return Err(unknown_key(key_id)),
            Some(STATUS_ACTIVE) => {}
            Some(_) => {
                return Err(ApiError::BadRequest {
                    code: "key_revoked",
                    message: format!("key '{key_id}' is revoked and cannot be rotated"),
                })
            }
        }
        let secret = generate_secret();
        let salt = new_salt();
        let hash = hash_secret(&salt, &secret);
        conn.execute(
            "UPDATE api_keys SET secret_hash = ?2, salt = ?3, rotated_at = ?4 WHERE key_id = ?1",
            rusqlite::params![key_id, hash, salt, now_unix_secs()],
        )
        .map_err(|err| ApiError::Internal(format!("cannot rotate key: {err}")))?;
        Ok(secret)
    }

    /// Revokes a key: it stops verifying immediately. Idempotent for an
    /// already-revoked key; an unknown `key_id` is an error, so a typo in
    /// an admin command cannot look like a successful revocation.
    pub fn revoke(&self, key_id: &str) -> Result<(), ApiError> {
        let conn = self.lock()?;
        let updated = conn
            .execute(
                "UPDATE api_keys SET status = ?2 WHERE key_id = ?1",
                rusqlite::params![key_id, STATUS_REVOKED],
            )
            .map_err(|err| ApiError::Internal(format!("cannot revoke key: {err}")))?;
        if updated == 0 {
            return Err(unknown_key(key_id));
        }
        Ok(())
    }

    /// Every key's summary, ordered by `key_id`.
    pub fn list(&self) -> Result<Vec<KeySummary>, ApiError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT key_id, plan, status, created_at, rotated_at
                 FROM api_keys ORDER BY key_id",
            )
            .map_err(|err| ApiError::Internal(format!("cannot read key store: {err}")))?;
        let rows = statement
            .query_map([], |row| {
                Ok(KeySummary {
                    key_id: row.get(0)?,
                    plan: row.get(1)?,
                    status: row.get(2)?,
                    created_at: row.get(3)?,
                    rotated_at: row.get(4)?,
                })
            })
            .map_err(|err| ApiError::Internal(format!("cannot read key store: {err}")))?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(
                row.map_err(|err| ApiError::Internal(format!("cannot read key store: {err}")))?,
            );
        }
        Ok(summaries)
    }

    /// Number of provisioned keys, in any status.
    pub fn len(&self) -> usize {
        self.count("SELECT COUNT(*) FROM api_keys")
    }

    /// True when no key is provisioned at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of keys that can authenticate right now.
    pub fn active_len(&self) -> usize {
        self.count("SELECT COUNT(*) FROM api_keys WHERE status = 'active'")
    }

    /// Checks a presented token and returns the key's identity and plan.
    ///
    /// An unknown `key_id`, a wrong secret, and a revoked key are all the
    /// same `401`, and the work done in each case is roughly equal (an
    /// unknown key still burns a hash and a comparison), so the response
    /// does not reveal which keys exist or why one failed.
    pub fn verify(&self, key_id: &str, secret: &str) -> Result<VerifiedKey, ApiError> {
        let row = self.lookup(key_id)?;
        let Some(row) = row else {
            // Burn the same work as a real verification — one hash over a
            // fresh random salt and one constant-time comparison — so an
            // unknown key is not measurably cheaper than a wrong secret.
            let dummy = hash_secret(&new_salt(), secret);
            let _ = dummy.as_slice().ct_eq(&[0u8; 32]);
            return Err(ApiError::Unauthorized);
        };
        let presented = hash_secret(&row.salt, secret);
        let matched: bool = row.secret_hash.as_slice().ct_eq(presented.as_slice()).into();
        if row.status == STATUS_ACTIVE && matched {
            Ok(VerifiedKey {
                key_id: key_id.to_string(),
                plan: row.plan,
            })
        } else {
            Err(ApiError::Unauthorized)
        }
    }

    /// Reads one key row, if the key exists.
    fn lookup(&self, key_id: &str) -> Result<Option<KeyRow>, ApiError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT secret_hash, salt, plan, status FROM api_keys WHERE key_id = ?1",
            rusqlite::params![key_id],
            |row| {
                Ok(KeyRow {
                    secret_hash: row.get(0)?,
                    salt: row.get(1)?,
                    plan: row.get(2)?,
                    status: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|err| ApiError::Internal(format!("cannot read key store: {err}")))
    }

    /// Inserts a key with a plaintext secret, storing only its hash.
    fn insert(&self, key_id: &str, secret: &str, plan: &str) -> Result<(), ApiError> {
        let salt = new_salt();
        let hash = hash_secret(&salt, secret);
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO api_keys (key_id, secret_hash, salt, plan, status, created_at, rotated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
            rusqlite::params![key_id, hash, salt, plan, STATUS_ACTIVE, now_unix_secs()],
        )
        .map_err(|err| ApiError::Internal(format!("cannot write key store: {err}")))?;
        Ok(())
    }

    /// Runs a `COUNT(*)` query, treating a store failure as zero. The
    /// count only feeds startup diagnostics and [`KeyStore::len`]; the
    /// fail-closed decision lives in `Config::prepare`, which checks the
    /// count it gets — a store that cannot be counted cannot serve
    /// either, and zero sends startup down the fail-closed path.
    fn count(&self, sql: &str) -> usize {
        let Ok(conn) = self.lock() else {
            return 0;
        };
        conn.query_row(sql, [], |row| row.get::<_, i64>(0))
            .map(|n| n.max(0) as usize)
            .unwrap_or(0)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ApiError> {
        self.conn
            .lock()
            .map_err(|_| ApiError::Internal("key store lock poisoned".to_string()))
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

/// Convenience: where the legacy flat keys file lives by default.
pub fn default_keys_file() -> PathBuf {
    PathBuf::from("./api_keys")
}

/// Parses legacy flat-file contents into `(key_id, secret)` pairs.
///
/// Blank lines and `#` comments are ignored; a line splits on its first
/// `:`; a repeated `key_id` keeps its last secret, matching the map the
/// original parser built.
fn parse_pairs(contents: &str) -> Vec<(String, String)> {
    let mut pairs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key_id, secret)) = line.split_once(':') {
            let key_id = key_id.trim();
            let secret = secret.trim();
            if !key_id.is_empty() && !secret.is_empty() {
                pairs.insert(key_id.to_string(), secret.to_string());
            }
        }
    }
    pairs.into_iter().collect()
}

/// `SHA-256(salt ‖ secret)` — the only form in which a secret is stored.
fn hash_secret(salt: &[u8], secret: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(salt);
    hasher.update(secret.as_bytes());
    hasher.finalize().to_vec()
}

/// A fresh per-key salt. A salt needs uniqueness, not secrecy; 128 bits
/// from the UUID generator is far past sufficient.
fn new_salt() -> Vec<u8> {
    uuid::Uuid::new_v4().as_bytes().to_vec()
}

/// A fresh secret: 64 lowercase hex characters (two UUID v4s), ~244 bits
/// of entropy, safe to print and to paste into a bearer token.
fn generate_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// A `key_id` must survive the bearer format: non-empty, no `:`, no
/// whitespace. Enforced at issuance so an unusable key is never created.
fn validate_key_id(key_id: &str) -> Result<(), ApiError> {
    if key_id.is_empty() || key_id.contains(':') || key_id.chars().any(char::is_whitespace) {
        return Err(ApiError::BadRequest {
            code: "invalid_key_id",
            message: format!(
                "invalid key_id '{key_id}': must be non-empty and contain no ':' or whitespace"
            ),
        });
    }
    Ok(())
}

/// 404 `unknown_key` for admin operations naming a key that does not exist.
fn unknown_key(key_id: &str) -> ApiError {
    ApiError::NotFound {
        code: "unknown_key",
        message: format!("unknown key '{key_id}'"),
    }
}

/// Current time as Unix seconds; 0 if the clock predates the epoch.
pub(crate) fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> KeyStore {
        KeyStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn parse_keeps_the_legacy_semantics() {
        let store = KeyStore::parse("# comment\n\nagent:s3cr3t\nmulti:a:b:c\n");
        assert_eq!(store.len(), 2);
        let verified = store.verify("agent", "s3cr3t").expect("agent verifies");
        assert_eq!(verified.key_id, "agent");
        assert_eq!(verified.plan, SELF_HOSTED_PLAN);
        // Split on the first colon: the rest is the secret.
        assert!(store.verify("multi", "a:b:c").is_ok());
        assert!(store.verify("agent", "wrong").is_err());
    }

    #[test]
    fn issued_key_verifies_with_its_plan() {
        let store = store();
        let secret = store.issue("tenant-a", "hosted").expect("issue");
        assert_eq!(secret.len(), 64, "secret is 64 hex chars");
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
        let verified = store.verify("tenant-a", &secret).expect("verifies");
        assert_eq!(verified.plan, "hosted");
        assert_eq!(store.active_len(), 1);
    }

    #[test]
    fn issue_rejects_a_duplicate_and_an_unusable_id() {
        let store = store();
        store.issue("tenant-a", "hosted").expect("issue");
        let err = store.issue("tenant-a", "hosted").expect_err("duplicate");
        assert_eq!(err.code(), "key_exists");
        for bad in ["", "has:colon", "has space"] {
            let err = store.issue(bad, "hosted").expect_err("unusable id");
            assert_eq!(err.code(), "invalid_key_id", "id {bad:?}");
        }
    }

    #[test]
    fn rotate_replaces_the_secret_and_keeps_the_plan() {
        let store = store();
        let old = store.issue("tenant-a", "hosted").expect("issue");
        let new = store.rotate("tenant-a").expect("rotate");
        assert_ne!(old, new);
        assert!(matches!(
            store.verify("tenant-a", &old),
            Err(ApiError::Unauthorized)
        ));
        let verified = store.verify("tenant-a", &new).expect("new secret verifies");
        assert_eq!(verified.plan, "hosted");
        let summary = &store.list().expect("list")[0];
        assert!(summary.rotated_at.is_some(), "rotation is recorded");
        assert_eq!(summary.status, STATUS_ACTIVE);
    }

    #[test]
    fn rotate_rejects_unknown_and_revoked_keys() {
        let store = store();
        assert_eq!(
            store.rotate("ghost").expect_err("unknown").code(),
            "unknown_key"
        );
        store.issue("tenant-a", "hosted").expect("issue");
        store.revoke("tenant-a").expect("revoke");
        assert_eq!(
            store.rotate("tenant-a").expect_err("revoked").code(),
            "key_revoked"
        );
    }

    #[test]
    fn revoke_stops_verification_immediately() {
        let store = store();
        let secret = store.issue("tenant-a", "hosted").expect("issue");
        assert!(store.verify("tenant-a", &secret).is_ok());
        store.revoke("tenant-a").expect("revoke");
        assert!(matches!(
            store.verify("tenant-a", &secret),
            Err(ApiError::Unauthorized)
        ));
        assert_eq!(store.active_len(), 0);
        assert_eq!(store.len(), 1, "the row remains for the record");
        // Revoking again is not an error; revoking a ghost is.
        store.revoke("tenant-a").expect("revoke is idempotent");
        assert_eq!(
            store.revoke("ghost").expect_err("unknown").code(),
            "unknown_key"
        );
    }

    #[test]
    fn wrong_secret_and_unknown_key_are_both_unauthorized() {
        let store = store();
        store.issue("tenant-a", "hosted").expect("issue");
        assert!(matches!(
            store.verify("tenant-a", "not-the-secret"),
            Err(ApiError::Unauthorized)
        ));
        assert!(matches!(
            store.verify("ghost", "not-the-secret"),
            Err(ApiError::Unauthorized)
        ));
    }

    #[test]
    fn secrets_are_never_stored_in_plaintext() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("keys.db");
        let store = KeyStore::open(&path).expect("open store");
        let secret = store.issue("tenant-a", "hosted").expect("issue");
        drop(store);
        let bytes = std::fs::read(&path).expect("read store file");
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(&secret),
            "the plaintext secret must not appear in the store file"
        );
        assert!(text.contains("tenant-a"), "the key_id is not a secret");
    }

    #[test]
    fn a_file_store_survives_reopening() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("keys.db");
        let secret = {
            let store = KeyStore::open(&path).expect("open");
            store.issue("tenant-a", "hosted").expect("issue")
        };
        // A second handle on the same file — the admin subcommand beside
        // a running server — sees the key with no restart anywhere.
        let reopened = KeyStore::open(&path).expect("reopen");
        assert!(reopened.verify("tenant-a", &secret).is_ok());
        let rotated = reopened.rotate("tenant-a").expect("rotate");
        let third = KeyStore::open(&path).expect("third handle");
        assert!(third.verify("tenant-a", &rotated).is_ok());
        assert!(third.verify("tenant-a", &secret).is_err());
    }

    #[test]
    fn legacy_import_hashes_and_never_overwrites() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("api_keys");
        std::fs::write(&file, "agent:file-secret\nsecond:other-secret\n").expect("write file");
        let store = store();
        let existing = store.issue("agent", "hosted").expect("issue");

        let imported = store.import_legacy_file(&file).expect("import");
        assert_eq!(imported, 1, "only the new key is imported");
        // The pre-existing key keeps its issued secret and plan; the file
        // secret for the same id must not verify.
        assert!(store.verify("agent", &existing).is_ok());
        assert!(store.verify("agent", "file-secret").is_err());
        let verified = store.verify("second", "other-secret").expect("imported key");
        assert_eq!(verified.plan, SELF_HOSTED_PLAN);
    }

    /// F-3: a keys file still carrying the README's old documented
    /// example credential is refused whole — the error names the value
    /// as the documented example, and nothing from the file imports,
    /// not even its well-formed entries.
    #[test]
    fn legacy_import_refuses_the_documented_example_secret() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("api_keys");
        std::fs::write(&file, "agent:real-secret\ndemo-key:change-me\n").expect("write file");
        let store = store();

        let err = store
            .import_legacy_file(&file)
            .expect_err("the documented example secret must be refused");
        let message = err.to_string();
        assert!(
            message.contains("documented example secret"),
            "the refusal names it as a documented example: {message}"
        );
        assert!(
            message.contains("change-me"),
            "the refusal names the value: {message}"
        );
        assert_eq!(store.len(), 0, "a refused import imports nothing");
        assert!(store.verify("agent", "real-secret").is_err());
    }

    /// The refusal keys on the secret value, not the example's key id:
    /// the published secret under any other name is the same hole.
    #[test]
    fn legacy_import_refuses_the_example_secret_under_any_key_id() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("api_keys");
        std::fs::write(&file, "my-own-key:change-me\n").expect("write file");
        let store = store();
        assert!(store.import_legacy_file(&file).is_err());
        assert_eq!(store.len(), 0);
    }

    #[test]
    fn list_reports_operator_state_without_secrets() {
        let store = store();
        store.issue("b-key", "hosted").expect("issue");
        store.issue("a-key", SELF_HOSTED_PLAN).expect("issue");
        store.revoke("b-key").expect("revoke");
        let summaries = store.list().expect("list");
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].key_id, "a-key", "ordered by key_id");
        assert_eq!(summaries[1].key_id, "b-key");
        assert_eq!(summaries[1].status, STATUS_REVOKED);
        assert!(summaries.iter().all(|s| s.created_at > 0));
        assert!(summaries.iter().all(|s| s.rotated_at.is_none()));
    }
}
