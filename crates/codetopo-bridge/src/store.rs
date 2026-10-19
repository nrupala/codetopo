// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! The bridge's own state, in tables alongside `api_keys`.
//!
//! Four records, all keyed by Stripe ids:
//!
//! - `bridge_processed_events` — every event id the bridge has applied,
//!   the replay ledger. An id present here is never applied again.
//! - `bridge_entitlements` — one row per subscription: which key it
//!   holds (and which issuance sequence that key is), its lifecycle
//!   status, when a pending revocation falls due, and the `created`
//!   of the last event applied to it (the ordering guard).
//! - `key_deliveries` + `claim_tokens` — the claim flow's state: one
//!   delivery per issued key (sealed plaintext, destroyed at claim or
//!   expiry) and the bearer tokens that address it.
//! - `bridge_pending_claims` — checkout sessions that arrived before
//!   their subscription's entitlement existed: a session id waiting to
//!   become a claim token at issuance. Swept on the claim horizon so
//!   abandoned (or other products') checkouts cannot grow it forever.
//! - `bridge_buyer_emails` — one row per subscription: the buyer
//!   address its checkout session carried (key-delivery email, see
//!   [`crate::mail`]), and the record a support resend would work
//!   from. Delivery rows themselves gain `emailed_at`: when the key
//!   email for that issuance went out, so a replay or a second
//!   triggering event can never send it twice.
//!
//! The tables live in the same `keys.db` file the server reads, so one
//! file is the whole billing-and-keys state a backup must capture. The
//! server reads only `api_keys`; these tables are inert to it. Same
//! discipline as the key store itself: one connection behind a mutex,
//! WAL mode with a busy timeout so the server, the admin subcommand,
//! and the bridge can share the file.

use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension};

use crate::crypto::Sealed;
use crate::BridgeError;

/// Entitlement status: the subscription is paid up; no revocation is
/// pending.
pub const STATUS_ACTIVE: &str = "active";
/// Entitlement status: payment failed; the key lives until the grace
/// deadline in `revoke_at`.
pub const STATUS_PAST_DUE: &str = "past_due";
/// Entitlement status: cancellation is scheduled; the key lives until
/// `revoke_at` (the cancellation's effective end).
pub const STATUS_CANCEL_SCHEDULED: &str = "cancel_scheduled";
/// Entitlement status: the key has been revoked. Terminal until a
/// restore event replaces the key.
pub const STATUS_REVOKED: &str = "revoked";

/// One subscription's entitlement row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlement {
    /// The Stripe subscription id.
    pub subscription_id: String,
    /// The key this subscription currently holds.
    pub key_id: String,
    /// Issuance sequence for this subscription: 1 for the first key,
    /// +1 per replacement after a revocation.
    pub key_seq: i64,
    /// One of the `STATUS_*` constants.
    pub status: String,
    /// Unix seconds when a pending revocation falls due, if one is
    /// pending (`past_due` grace end or cancellation effective end).
    pub revoke_at: Option<i64>,
    /// `occurred_at` of the last event applied to this subscription.
    pub last_event_at: i64,
}

/// One key delivery: the sealed form of a freshly issued secret.
#[derive(Debug, Clone)]
pub struct Delivery {
    /// The key this delivery can reveal.
    pub key_id: String,
    /// Sealed plaintext; `None` once claimed or purged — the secret is
    /// then unrecoverable from the store, by design.
    pub sealed: Option<Sealed>,
    /// Unix seconds the delivery was created.
    pub created_at: i64,
    /// Unix seconds after which the delivery is purged unclaimed.
    pub expires_at: i64,
    /// Unix seconds the delivery was claimed, if it was.
    pub claimed_at: Option<i64>,
    /// Unix seconds the key email for this issuance was sent, if it
    /// was. The send marker lives on the delivery — one per issued
    /// key — so a replacement key gets its own send and the first
    /// key's marker can never suppress or duplicate it.
    pub emailed_at: Option<i64>,
}

/// The bridge store: one SQLite connection, serialised behind a mutex.
#[derive(Debug)]
pub struct BridgeStore {
    conn: Mutex<Connection>,
}

impl BridgeStore {
    /// Opens (creating if needed) the bridge tables in the database at
    /// `path` — the server key store's file.
    pub fn open(path: &Path) -> Result<Self, BridgeError> {
        let conn = Connection::open(path).map_err(|err| {
            BridgeError::Store(format!(
                "cannot open bridge store {}: {err}",
                path.display()
            ))
        })?;
        Self::init(conn)
    }

    /// An in-memory store, for tests.
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, BridgeError> {
        let conn = Connection::open_in_memory()
            .map_err(|err| BridgeError::Store(format!("cannot open in-memory store: {err}")))?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, BridgeError> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS bridge_processed_events (
                 event_id    TEXT PRIMARY KEY,
                 event_type  TEXT NOT NULL,
                 occurred_at INTEGER NOT NULL,
                 processed_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS bridge_entitlements (
                 subscription_id TEXT PRIMARY KEY,
                 key_id          TEXT NOT NULL,
                 key_seq         INTEGER NOT NULL,
                 status          TEXT NOT NULL,
                 revoke_at       INTEGER,
                 last_event_at   INTEGER NOT NULL,
                 updated_at      INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS key_deliveries (
                 key_id     TEXT PRIMARY KEY,
                 ciphertext BLOB,
                 nonce      BLOB,
                 created_at INTEGER NOT NULL,
                 expires_at INTEGER NOT NULL,
                 claimed_at INTEGER
             );
             CREATE TABLE IF NOT EXISTS claim_tokens (
                 token  TEXT PRIMARY KEY,
                 key_id TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS bridge_pending_claims (
                 token           TEXT PRIMARY KEY,
                 subscription_id TEXT NOT NULL,
                 created_at      INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS bridge_buyer_emails (
                 subscription_id TEXT PRIMARY KEY,
                 email           TEXT NOT NULL,
                 updated_at      INTEGER NOT NULL
             );",
        )
        .map_err(|err| BridgeError::Store(format!("cannot initialise bridge store: {err}")))?;
        // Migration for stores created before key-delivery email:
        // `key_deliveries` gains the per-issuance send marker.
        let has_emailed_at = {
            let mut statement = conn
                .prepare("PRAGMA table_info(key_deliveries)")
                .map_err(|err| BridgeError::Store(format!("cannot inspect deliveries: {err}")))?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|err| BridgeError::Store(format!("cannot inspect deliveries: {err}")))?;
            let mut found = false;
            for column in columns {
                let column = column
                    .map_err(|err| BridgeError::Store(format!("cannot inspect deliveries: {err}")))?;
                if column == "emailed_at" {
                    found = true;
                }
            }
            found
        };
        if !has_emailed_at {
            conn.execute(
                "ALTER TABLE key_deliveries ADD COLUMN emailed_at INTEGER",
                [],
            )
            .map_err(|err| BridgeError::Store(format!("cannot migrate deliveries: {err}")))?;
        }
        Ok(BridgeStore {
            conn: Mutex::new(conn),
        })
    }

    // -- processed events (the replay ledger) -------------------------

    /// True when this event id has already been applied.
    pub fn event_processed(&self, event_id: &str) -> Result<bool, BridgeError> {
        let conn = self.lock()?;
        let found = conn
            .query_row(
                "SELECT 1 FROM bridge_processed_events WHERE event_id = ?1",
                rusqlite::params![event_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|err| BridgeError::Store(format!("cannot read processed events: {err}")))?;
        Ok(found.is_some())
    }

    /// Records an event as applied. Called only after its handler has
    /// run: a handler that failed leaves no record, so Stripe's retry
    /// re-applies (every handler is idempotent, so converging is safe).
    pub fn record_event(
        &self,
        event_id: &str,
        event_type: &str,
        occurred_at: i64,
        now: i64,
    ) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO bridge_processed_events
                 (event_id, event_type, occurred_at, processed_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![event_id, event_type, occurred_at, now],
        )
        .map_err(|err| BridgeError::Store(format!("cannot record event: {err}")))?;
        Ok(())
    }

    // -- entitlements --------------------------------------------------

    /// One subscription's entitlement, if the bridge has ever granted it.
    pub fn entitlement(&self, subscription_id: &str) -> Result<Option<Entitlement>, BridgeError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT subscription_id, key_id, key_seq, status, revoke_at, last_event_at
             FROM bridge_entitlements WHERE subscription_id = ?1",
            rusqlite::params![subscription_id],
            |row| {
                Ok(Entitlement {
                    subscription_id: row.get(0)?,
                    key_id: row.get(1)?,
                    key_seq: row.get(2)?,
                    status: row.get(3)?,
                    revoke_at: row.get(4)?,
                    last_event_at: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|err| BridgeError::Store(format!("cannot read entitlement: {err}")))
    }

    /// Inserts or replaces a subscription's entitlement row.
    pub fn save_entitlement(&self, ent: &Entitlement, now: i64) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO bridge_entitlements
                 (subscription_id, key_id, key_seq, status, revoke_at, last_event_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(subscription_id) DO UPDATE SET
                 key_id = excluded.key_id,
                 key_seq = excluded.key_seq,
                 status = excluded.status,
                 revoke_at = excluded.revoke_at,
                 last_event_at = excluded.last_event_at,
                 updated_at = excluded.updated_at",
            rusqlite::params![
                ent.subscription_id,
                ent.key_id,
                ent.key_seq,
                ent.status,
                ent.revoke_at,
                ent.last_event_at,
                now
            ],
        )
        .map_err(|err| BridgeError::Store(format!("cannot save entitlement: {err}")))?;
        Ok(())
    }

    /// Entitlements whose pending revocation has fallen due.
    pub fn due_revocations(&self, now: i64) -> Result<Vec<Entitlement>, BridgeError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT subscription_id, key_id, key_seq, status, revoke_at, last_event_at
                 FROM bridge_entitlements
                 WHERE revoke_at IS NOT NULL AND revoke_at <= ?1
                   AND status IN (?2, ?3)
                 ORDER BY subscription_id",
            )
            .map_err(|err| BridgeError::Store(format!("cannot read entitlements: {err}")))?;
        let rows = statement
            .query_map(rusqlite::params![now, STATUS_PAST_DUE, STATUS_CANCEL_SCHEDULED], |row| {
                Ok(Entitlement {
                    subscription_id: row.get(0)?,
                    key_id: row.get(1)?,
                    key_seq: row.get(2)?,
                    status: row.get(3)?,
                    revoke_at: row.get(4)?,
                    last_event_at: row.get(5)?,
                })
            })
            .map_err(|err| BridgeError::Store(format!("cannot read entitlements: {err}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|err| BridgeError::Store(format!("cannot read entitlements: {err}")))?,
            );
        }
        Ok(out)
    }

    // -- deliveries and claim tokens -----------------------------------

    /// Stores a freshly sealed delivery for `key_id`, replacing any
    /// previous row for that key (a replacement key has a new key id,
    /// so replacement never happens in practice; the upsert keeps a
    /// retried issuance from failing on its own first attempt).
    pub fn create_delivery(
        &self,
        key_id: &str,
        sealed: &Sealed,
        created_at: i64,
        expires_at: i64,
    ) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO key_deliveries (key_id, ciphertext, nonce, created_at, expires_at, claimed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)
             ON CONFLICT(key_id) DO UPDATE SET
                 ciphertext = excluded.ciphertext,
                 nonce = excluded.nonce,
                 created_at = excluded.created_at,
                 expires_at = excluded.expires_at,
                 claimed_at = NULL,
                 emailed_at = NULL",
            rusqlite::params![key_id, sealed.ciphertext, sealed.nonce, created_at, expires_at],
        )
        .map_err(|err| BridgeError::Store(format!("cannot save delivery: {err}")))?;
        Ok(())
    }

    /// One key's delivery, if it has one.
    pub fn delivery(&self, key_id: &str) -> Result<Option<Delivery>, BridgeError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT key_id, ciphertext, nonce, created_at, expires_at, claimed_at, emailed_at
             FROM key_deliveries WHERE key_id = ?1",
            rusqlite::params![key_id],
            |row| {
                let ciphertext: Option<Vec<u8>> = row.get(1)?;
                let nonce: Option<Vec<u8>> = row.get(2)?;
                Ok(Delivery {
                    key_id: row.get(0)?,
                    sealed: match (ciphertext, nonce) {
                        (Some(ciphertext), Some(nonce)) => Some(Sealed { ciphertext, nonce }),
                        _ => None,
                    },
                    created_at: row.get(3)?,
                    expires_at: row.get(4)?,
                    claimed_at: row.get(5)?,
                    emailed_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(|err| BridgeError::Store(format!("cannot read delivery: {err}")))
    }

    /// Stamps the key email for one issuance as sent. Called only
    /// after the transport accepted the message; a failed send leaves
    /// the stamp unset, so a later event for the same subscription
    /// may retry while the delivery still lives.
    pub fn mark_delivery_emailed(&self, key_id: &str, now: i64) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE key_deliveries SET emailed_at = ?2 WHERE key_id = ?1",
            rusqlite::params![key_id, now],
        )
        .map_err(|err| BridgeError::Store(format!("cannot stamp delivery emailed: {err}")))?;
        Ok(())
    }

    // -- buyer emails (key-delivery email, per subscription) -----------

    /// Records the buyer address a checkout session carried for a
    /// subscription. The newest checkout wins: a buyer who re-buys
    /// with a corrected address gets their key at the new one. The
    /// record outlives the entitlement — it is what a support resend
    /// would work from — and it is never an authority for anything.
    pub fn record_buyer_email(
        &self,
        subscription_id: &str,
        email: &str,
        now: i64,
    ) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO bridge_buyer_emails (subscription_id, email, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(subscription_id) DO UPDATE SET
                 email = excluded.email,
                 updated_at = excluded.updated_at",
            rusqlite::params![subscription_id, email, now],
        )
        .map_err(|err| BridgeError::Store(format!("cannot record buyer email: {err}")))?;
        Ok(())
    }

    /// The recorded buyer address for a subscription, if any.
    pub fn buyer_email(&self, subscription_id: &str) -> Result<Option<String>, BridgeError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT email FROM bridge_buyer_emails WHERE subscription_id = ?1",
            rusqlite::params![subscription_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| BridgeError::Store(format!("cannot read buyer email: {err}")))
    }

    /// Registers a claim token for a key. Idempotent: a token already
    /// pointing at this key is left as it is; a token pointing at a
    /// *different* key is left untouched too (tokens are Stripe ids —
    /// a collision would mean the same purchase naming two keys, which
    /// must surface as an operator problem, not a silent retarget).
    pub fn add_claim_token(&self, token: &str, key_id: &str) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO claim_tokens (token, key_id) VALUES (?1, ?2)",
            rusqlite::params![token, key_id],
        )
        .map_err(|err| BridgeError::Store(format!("cannot save claim token: {err}")))?;
        Ok(())
    }

    /// The key a claim token addresses, if the token is registered.
    pub fn key_for_token(&self, token: &str) -> Result<Option<String>, BridgeError> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT key_id FROM claim_tokens WHERE token = ?1",
            rusqlite::params![token],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| BridgeError::Store(format!("cannot read claim token: {err}")))
    }

    // -- pending claims (checkout sessions awaiting a subscription) ----

    /// Records a checkout session against the subscription it created,
    /// for a subscription whose entitlement does not exist yet.
    /// Idempotent, like [`BridgeStore::add_claim_token`]: the first
    /// record for a session id stands.
    pub fn add_pending_claim(
        &self,
        token: &str,
        subscription_id: &str,
        now: i64,
    ) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT OR IGNORE INTO bridge_pending_claims (token, subscription_id, created_at)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![token, subscription_id, now],
        )
        .map_err(|err| BridgeError::Store(format!("cannot save pending claim: {err}")))?;
        Ok(())
    }

    /// The checkout sessions recorded against a subscription, in
    /// token order (deterministic for tests and logs).
    pub fn pending_claims(&self, subscription_id: &str) -> Result<Vec<String>, BridgeError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT token FROM bridge_pending_claims
                 WHERE subscription_id = ?1 ORDER BY token",
            )
            .map_err(|err| BridgeError::Store(format!("cannot read pending claims: {err}")))?;
        let rows = statement
            .query_map(rusqlite::params![subscription_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|err| BridgeError::Store(format!("cannot read pending claims: {err}")))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|err| {
                    BridgeError::Store(format!("cannot read pending claims: {err}"))
                })?,
            );
        }
        Ok(out)
    }

    /// Retires every pending record for a subscription (called when
    /// its entitlement is issued and the sessions have been attached
    /// as claim tokens).
    pub fn clear_pending_claims(&self, subscription_id: &str) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "DELETE FROM bridge_pending_claims WHERE subscription_id = ?1",
            rusqlite::params![subscription_id],
        )
        .map_err(|err| BridgeError::Store(format!("cannot clear pending claims: {err}")))?;
        Ok(())
    }

    /// Drops pending records recorded at or before `before` — sessions
    /// whose subscription never materialised inside the claim horizon.
    /// Returns how many were dropped.
    pub fn purge_stale_pending_claims(&self, before: i64) -> Result<usize, BridgeError> {
        let conn = self.lock()?;
        let dropped = conn
            .execute(
                "DELETE FROM bridge_pending_claims WHERE created_at <= ?1",
                rusqlite::params![before],
            )
            .map_err(|err| BridgeError::Store(format!("cannot purge pending claims: {err}")))?;
        Ok(dropped)
    }

    /// Marks a delivery claimed and destroys its plaintext and every
    /// token that addresses it: the one claim is the only claim.
    pub fn complete_claim(&self, key_id: &str, now: i64) -> Result<(), BridgeError> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE key_deliveries SET ciphertext = NULL, nonce = NULL, claimed_at = ?2
             WHERE key_id = ?1",
            rusqlite::params![key_id, now],
        )
        .map_err(|err| BridgeError::Store(format!("cannot complete claim: {err}")))?;
        conn.execute(
            "DELETE FROM claim_tokens WHERE key_id = ?1",
            rusqlite::params![key_id],
        )
        .map_err(|err| BridgeError::Store(format!("cannot retire claim tokens: {err}")))?;
        Ok(())
    }

    /// Destroys the plaintext of every expired, unclaimed delivery and
    /// retires its tokens. Returns how many deliveries were purged.
    /// The delivery row stays (claimed-at/expiry bookkeeping); the hash
    /// in `api_keys` was never touched.
    pub fn purge_expired_deliveries(&self, now: i64) -> Result<usize, BridgeError> {
        let conn = self.lock()?;
        let mut statement = conn
            .prepare(
                "SELECT key_id FROM key_deliveries
                 WHERE ciphertext IS NOT NULL AND expires_at <= ?1",
            )
            .map_err(|err| BridgeError::Store(format!("cannot read deliveries: {err}")))?;
        let rows = statement
            .query_map(rusqlite::params![now], |row| row.get::<_, String>(0))
            .map_err(|err| BridgeError::Store(format!("cannot read deliveries: {err}")))?;
        let key_ids: Vec<String> = rows
            .collect::<Result<_, _>>()
            .map_err(|err| BridgeError::Store(format!("cannot read deliveries: {err}")))?;
        drop(statement);
        let mut purged = 0;
        for key_id in &key_ids {
            conn.execute(
                "UPDATE key_deliveries SET ciphertext = NULL, nonce = NULL WHERE key_id = ?1",
                rusqlite::params![key_id],
            )
            .map_err(|err| BridgeError::Store(format!("cannot purge delivery: {err}")))?;
            conn.execute(
                "DELETE FROM claim_tokens WHERE key_id = ?1",
                rusqlite::params![key_id],
            )
            .map_err(|err| BridgeError::Store(format!("cannot retire claim tokens: {err}")))?;
            purged += 1;
        }
        Ok(purged)
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, BridgeError> {
        self.conn
            .lock()
            .map_err(|_| BridgeError::Store("bridge store lock poisoned".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> BridgeStore {
        BridgeStore::in_memory().expect("in-memory store")
    }

    fn ent(status: &str, revoke_at: Option<i64>) -> Entitlement {
        Entitlement {
            subscription_id: "sub_1".to_string(),
            key_id: "sub_1".to_string(),
            key_seq: 1,
            status: status.to_string(),
            revoke_at,
            last_event_at: 100,
        }
    }

    #[test]
    fn events_are_recorded_once() {
        let store = store();
        assert!(!store.event_processed("evt_1").expect("read"));
        store.record_event("evt_1", "subscription.activated", 100, 100).expect("record");
        assert!(store.event_processed("evt_1").expect("read"));
        store.record_event("evt_1", "subscription.activated", 100, 100).expect("idempotent");
    }

    #[test]
    fn entitlements_round_trip_and_due_revocations_filter() {
        let store = store();
        store.save_entitlement(&ent(STATUS_PAST_DUE, Some(500)), 100).expect("save");
        let read = store.entitlement("sub_1").expect("read").expect("present");
        assert_eq!(read.status, STATUS_PAST_DUE);
        assert_eq!(read.revoke_at, Some(500));
        assert!(store.due_revocations(499).expect("due").is_empty());
        assert_eq!(store.due_revocations(500).expect("due").len(), 1);
        // A revoked entitlement is not "due" — it is done.
        store.save_entitlement(&ent(STATUS_REVOKED, Some(400)), 100).expect("save");
        assert!(store.due_revocations(600).expect("due").is_empty());
    }

    #[test]
    fn deliveries_claim_and_purge() {
        let store = store();
        let sealed = Sealed { ciphertext: vec![1, 2, 3], nonce: vec![4; 12] };
        store.create_delivery("k1", &sealed, 100, 200).expect("create");
        store.add_claim_token("txn_1", "k1").expect("token");
        store.add_claim_token("sub_1", "k1").expect("token");
        assert_eq!(store.key_for_token("txn_1").expect("read").as_deref(), Some("k1"));

        store.complete_claim("k1", 150).expect("claim");
        let delivery = store.delivery("k1").expect("read").expect("present");
        assert!(delivery.sealed.is_none(), "plaintext destroyed at claim");
        assert_eq!(delivery.claimed_at, Some(150));
        assert!(store.key_for_token("txn_1").expect("read").is_none(), "tokens retired");

        store.create_delivery("k2", &sealed, 100, 200).expect("create");
        store.add_claim_token("txn_2", "k2").expect("token");
        assert_eq!(store.purge_expired_deliveries(199).expect("purge"), 0);
        assert_eq!(store.purge_expired_deliveries(200).expect("purge"), 1);
        assert!(store.delivery("k2").expect("read").expect("present").sealed.is_none());
    }

    #[test]
    fn buyer_emails_round_trip_and_the_newest_wins() {
        let store = store();
        assert!(store.buyer_email("sub_1").expect("read").is_none());
        store.record_buyer_email("sub_1", "first@example.com", 100).expect("record");
        assert_eq!(
            store.buyer_email("sub_1").expect("read").as_deref(),
            Some("first@example.com")
        );
        store.record_buyer_email("sub_1", "second@example.com", 200).expect("record");
        assert_eq!(
            store.buyer_email("sub_1").expect("read").as_deref(),
            Some("second@example.com"),
            "a later checkout corrects the address"
        );
        assert!(store.buyer_email("sub_2").expect("read").is_none(), "scoped per subscription");
    }

    #[test]
    fn deliveries_carry_the_emailed_marker() {
        let store = store();
        let sealed = Sealed { ciphertext: vec![1, 2, 3], nonce: vec![4; 12] };
        store.create_delivery("k1", &sealed, 100, 200).expect("create");
        let delivery = store.delivery("k1").expect("read").expect("present");
        assert_eq!(delivery.emailed_at, None);
        store.mark_delivery_emailed("k1", 150).expect("stamp");
        let delivery = store.delivery("k1").expect("read").expect("present");
        assert_eq!(delivery.emailed_at, Some(150));
        // A re-created delivery (a retried issuance) starts un-emailed.
        store.create_delivery("k1", &sealed, 160, 260).expect("recreate");
        let delivery = store.delivery("k1").expect("read").expect("present");
        assert_eq!(delivery.emailed_at, None);
    }

    #[test]
    fn a_store_created_before_key_email_migrates_on_open() {
        // The pre-email schema: key_deliveries without emailed_at.
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("keys.db");
        {
            let conn = Connection::open(&path).expect("old store");
            conn.execute_batch(
                "CREATE TABLE key_deliveries (
                     key_id     TEXT PRIMARY KEY,
                     ciphertext BLOB,
                     nonce      BLOB,
                     created_at INTEGER NOT NULL,
                     expires_at INTEGER NOT NULL,
                     claimed_at INTEGER
                 );
                 INSERT INTO key_deliveries VALUES ('k1', X'0102', X'0405', 100, 200, NULL);",
            )
            .expect("old schema");
        }
        let store = BridgeStore::open(&path).expect("migrated store");
        let delivery = store.delivery("k1").expect("read").expect("present");
        assert_eq!(delivery.emailed_at, None);
        store.mark_delivery_emailed("k1", 150).expect("stamp works after migration");
        assert_eq!(
            store.delivery("k1").expect("read").expect("present").emailed_at,
            Some(150)
        );
        // Opening again does not re-run the migration into an error.
        let store = BridgeStore::open(&path).expect("second open");
        assert_eq!(
            store.delivery("k1").expect("read").expect("present").emailed_at,
            Some(150)
        );
    }

    #[test]
    fn pending_claims_round_trip_clear_and_purge() {
        let store = store();
        store.add_pending_claim("cs_2", "sub_1", 100).expect("record");
        store.add_pending_claim("cs_1", "sub_1", 100).expect("record");
        store.add_pending_claim("cs_9", "sub_2", 100).expect("record");
        store.add_pending_claim("cs_1", "sub_1", 100).expect("idempotent");
        assert_eq!(
            store.pending_claims("sub_1").expect("read"),
            ["cs_1", "cs_2"],
            "ordered, deduplicated, and scoped to the subscription"
        );
        // The stale purge drops by recording time, across subscriptions.
        assert_eq!(store.purge_stale_pending_claims(99).expect("purge"), 0);
        assert_eq!(store.purge_stale_pending_claims(100).expect("purge"), 3);
        assert!(store.pending_claims("sub_1").expect("read").is_empty());

        store.add_pending_claim("cs_3", "sub_1", 200).expect("record");
        store.clear_pending_claims("sub_1").expect("clear");
        assert!(store.pending_claims("sub_1").expect("read").is_empty());
    }
}
