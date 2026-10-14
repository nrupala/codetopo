// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Index ownership and expiry (paid-path plan S1, gap G2).
//!
//! Before tenancy, any valid key could query any `index_id`: the handlers
//! resolved `<data_dir>/<id>.db` straight from the URL and the caller's
//! identity was used only for metering. This module is the record that
//! closes that: every index created through the API is owned by the key
//! that created it, carries a `created_at`, and — when the operator
//! configures a TTL — an `expires_at` after which it is refused.
//!
//! The record lives in its own SQLite file in the data dir
//! (`tenancy.db`), not inside each index's database: `codetopo-store`
//! owns the per-index schema (graph + audit chain), and ownership is a
//! property of the *service*, not of one graph.
//!
//! An index with **no** owner row is a legacy index — written before
//! tenancy existed (or placed in the data dir by another tool, e.g. the
//! CLI). Legacy indexes stay readable by any authenticated key: refusing
//! them would strand every existing self-hosted data dir, and a hosted
//! deployment starts with an empty data dir, so nothing there is legacy.
//! The handlers enforce that rule; this module only keeps the record.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension};

use crate::error::ApiError;

/// File name of the tenancy store inside the data dir.
pub const STORE_FILE_NAME: &str = "tenancy.db";

/// One index's ownership record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRecord {
    /// The index id (the `<id>` in `/v1/graphs/<id>/…`).
    pub index_id: String,
    /// The `key_id` that created the index and may query it.
    pub owner_key_id: String,
    /// Unix seconds at creation.
    pub created_at: i64,
    /// Unix seconds after which the index is refused, if a TTL applies.
    pub expires_at: Option<i64>,
}

impl IndexRecord {
    /// True once `now` has reached the expiry, if this index has one.
    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|expires| expires <= now)
    }
}

/// The tenancy store: one SQLite connection, serialised behind a mutex.
///
/// Lookups are one indexed query per request, trivial next to the
/// whole-graph load every query already performs. WAL mode plus a busy
/// timeout keeps the file shareable, matching the key store.
#[derive(Debug)]
pub struct Tenancy {
    conn: Mutex<Connection>,
}

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS index_owners (
    index_id     TEXT PRIMARY KEY,
    owner_key_id TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    expires_at   INTEGER
);";

impl Tenancy {
    /// Opens (creating if needed) the tenancy store at `path`.
    pub fn open(path: &Path) -> Result<Self, ApiError> {
        let conn = Connection::open(path).map_err(|err| {
            ApiError::Internal(format!(
                "cannot open tenancy store {}: {err}",
                path.display()
            ))
        })?;
        conn.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             {SCHEMA}"
        ))
        .map_err(|err| ApiError::Internal(format!("cannot initialise tenancy store: {err}")))?;
        Ok(Tenancy {
            conn: Mutex::new(conn),
        })
    }

    /// An in-memory store, for tests.
    #[cfg(test)]
    fn in_memory() -> Result<Self, ApiError> {
        let conn = Connection::open_in_memory().map_err(|err| {
            ApiError::Internal(format!("cannot open in-memory tenancy store: {err}"))
        })?;
        conn.execute_batch(SCHEMA)
            .map_err(|err| ApiError::Internal(format!("cannot initialise tenancy store: {err}")))?;
        Ok(Tenancy {
            conn: Mutex::new(conn),
        })
    }

    /// Records `index_id` as owned by `owner_key_id`, created at
    /// `created_at` (Unix seconds) and expiring at `expires_at` if given.
    pub fn record(
        &self,
        index_id: &str,
        owner_key_id: &str,
        created_at: i64,
        expires_at: Option<i64>,
    ) -> Result<(), ApiError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO index_owners (index_id, owner_key_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![index_id, owner_key_id, created_at, expires_at],
        )
        .map_err(|err| ApiError::Internal(format!("cannot record index owner: {err}")))?;
        Ok(())
    }

    /// The ownership record for `index_id`, or `None` for a legacy index.
    pub fn lookup(&self, index_id: &str) -> Result<Option<IndexRecord>, ApiError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT index_id, owner_key_id, created_at, expires_at
             FROM index_owners WHERE index_id = ?1",
            rusqlite::params![index_id],
            |row| {
                Ok(IndexRecord {
                    index_id: row.get(0)?,
                    owner_key_id: row.get(1)?,
                    created_at: row.get(2)?,
                    expires_at: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|err| ApiError::Internal(format!("cannot read tenancy store: {err}")))
    }

    /// How many indexes `owner_key_id` actively holds at `now`:
    /// records it owns whose expiry has not passed. An expired
    /// record does not count — it is refused on access and pruned
    /// on sight, so a key is never charged quota for an index it
    /// can no longer read (paid-path S3's active-index quota reads
    /// this number).
    pub fn count_active_for_owner(&self, owner_key_id: &str, now: i64) -> Result<usize, ApiError> {
        let conn = self.lock()?;
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM index_owners
                 WHERE owner_key_id = ?1
                   AND (expires_at IS NULL OR expires_at > ?2)",
                rusqlite::params![owner_key_id, now],
                |row| row.get(0),
            )
            .map_err(|err| ApiError::Internal(format!("cannot read tenancy store: {err}")))?;
        Ok(count.max(0) as usize)
    }

    /// Drops the ownership record for `index_id`, if one exists.
    pub fn remove(&self, index_id: &str) -> Result<(), ApiError> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM index_owners WHERE index_id = ?1",
            rusqlite::params![index_id],
        )
        .map_err(|err| ApiError::Internal(format!("cannot update tenancy store: {err}")))?;
        Ok(())
    }

    /// Deletes every expired record and returns the expired index ids,
    /// so the caller can delete the matching `<id>.db` files. Records
    /// without an expiry never match.
    pub fn prune_expired(&self, now: i64) -> Result<Vec<String>, ApiError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT index_id FROM index_owners
                 WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            )
            .map_err(|err| ApiError::Internal(format!("cannot read tenancy store: {err}")))?;
        let rows = statement
            .query_map(rusqlite::params![now], |row| row.get::<_, String>(0))
            .map_err(|err| ApiError::Internal(format!("cannot read tenancy store: {err}")))?;
        let mut expired = Vec::new();
        for row in rows {
            expired.push(
                row.map_err(|err| {
                    ApiError::Internal(format!("cannot read tenancy store: {err}"))
                })?,
            );
        }
        drop(statement);
        for index_id in &expired {
            conn.execute(
                "DELETE FROM index_owners WHERE index_id = ?1",
                rusqlite::params![index_id],
            )
            .map_err(|err| ApiError::Internal(format!("cannot update tenancy store: {err}")))?;
        }
        Ok(expired)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ApiError> {
        self.conn
            .lock()
            .map_err(|_| ApiError::Internal("tenancy store lock poisoned".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenancy() -> Tenancy {
        Tenancy::in_memory().expect("in-memory tenancy")
    }

    #[test]
    fn record_and_lookup_round_trip() {
        let tenancy = tenancy();
        tenancy
            .record("idx-1", "tenant-a", 1_700_000_000, Some(1_700_086_400))
            .expect("record");
        let found = tenancy.lookup("idx-1").expect("lookup").expect("present");
        assert_eq!(
            found,
            IndexRecord {
                index_id: "idx-1".to_string(),
                owner_key_id: "tenant-a".to_string(),
                created_at: 1_700_000_000,
                expires_at: Some(1_700_086_400),
            }
        );
        assert!(tenancy.lookup("idx-2").expect("lookup").is_none());
    }

    #[test]
    fn record_without_expiry_never_expires() {
        let tenancy = tenancy();
        tenancy.record("idx-1", "tenant-a", 100, None).expect("record");
        let found = tenancy.lookup("idx-1").expect("lookup").expect("present");
        assert!(!found.is_expired(i64::MAX));
    }

    #[test]
    fn expiry_boundary_is_inclusive() {
        let record = IndexRecord {
            index_id: "idx".to_string(),
            owner_key_id: "tenant-a".to_string(),
            created_at: 100,
            expires_at: Some(200),
        };
        assert!(!record.is_expired(199));
        assert!(record.is_expired(200), "at expires_at the index is refused");
        assert!(record.is_expired(201));
    }

    #[test]
    fn count_active_for_owner_counts_only_live_indexes_of_that_owner() {
        let tenancy = tenancy();
        tenancy.record("live", "tenant-a", 10, None).expect("record");
        tenancy
            .record("fresh", "tenant-a", 10, Some(500))
            .expect("record");
        tenancy
            .record("stale", "tenant-a", 10, Some(50))
            .expect("record");
        tenancy
            .record("boundary", "tenant-a", 10, Some(100))
            .expect("record");
        tenancy.record("other", "tenant-b", 10, None).expect("record");

        // At now=100: "stale" expired at 50, and "boundary" expires
        // exactly at 100 (the expiry boundary is inclusive, as
        // `is_expired` pins) — neither counts.
        assert_eq!(
            tenancy.count_active_for_owner("tenant-a", 100).expect("count"),
            2
        );
        assert_eq!(
            tenancy.count_active_for_owner("tenant-b", 100).expect("count"),
            1
        );
        assert_eq!(
            tenancy.count_active_for_owner("ghost", 100).expect("count"),
            0
        );
    }

    #[test]
    fn remove_drops_the_record() {
        let tenancy = tenancy();
        tenancy.record("idx-1", "tenant-a", 100, None).expect("record");
        tenancy.remove("idx-1").expect("remove");
        assert!(tenancy.lookup("idx-1").expect("lookup").is_none());
        // Removing a missing record is not an error.
        tenancy.remove("idx-1").expect("remove again");
    }

    #[test]
    fn prune_expired_returns_only_the_expired() {
        let tenancy = tenancy();
        tenancy.record("old", "tenant-a", 10, Some(50)).expect("record");
        tenancy
            .record("fresh", "tenant-a", 10, Some(500))
            .expect("record");
        tenancy.record("forever", "tenant-b", 10, None).expect("record");

        let pruned = tenancy.prune_expired(100).expect("prune");
        assert_eq!(pruned, vec!["old".to_string()]);
        assert!(tenancy.lookup("old").expect("lookup").is_none());
        assert!(tenancy.lookup("fresh").expect("lookup").is_some());
        assert!(tenancy.lookup("forever").expect("lookup").is_some());
        // A second sweep finds nothing left to do.
        assert!(tenancy.prune_expired(100).expect("prune").is_empty());
    }

    #[test]
    fn a_file_store_survives_reopening() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tenancy.db");
        {
            let tenancy = Tenancy::open(&path).expect("open");
            tenancy
                .record("idx-1", "tenant-a", 100, None)
                .expect("record");
        }
        let reopened = Tenancy::open(&path).expect("reopen");
        let found = reopened.lookup("idx-1").expect("lookup").expect("present");
        assert_eq!(found.owner_key_id, "tenant-a");
    }
}
