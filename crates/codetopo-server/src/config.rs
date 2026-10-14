// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Runtime configuration, all of it from the environment.

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::ApiError;
use crate::keys::{now_unix_secs, KeyStore};
use crate::tenancy::Tenancy;

/// Default bind address: loopback only. Exposing `codetopo-server` beyond the
/// host is an explicit operator decision, so the default is the safe one.
pub const DEFAULT_BIND: &str = "127.0.0.1:8080";
/// Default directory holding one `<index_id>.db` per indexed repository.
pub const DEFAULT_DATA_DIR: &str = "./data";
/// Default newline-delimited `key_id:secret` file.
pub const DEFAULT_KEYS_FILE: &str = "./api_keys";
/// Default append-only metering log.
pub const DEFAULT_METERING_LOG: &str = "./metering.log";

/// Resolved paths and bind address for one server process.
#[derive(Debug, Clone)]
pub struct Config {
    /// Directory where index databases live. One `<index_id>.db` per index.
    pub data_dir: PathBuf,
    /// Legacy flat `key_id:secret` file. No longer the key store — the
    /// store is `<data_dir>/keys.db` — but at startup its entries are
    /// imported when the store holds no active key, so an existing
    /// deployment keeps working (see [`crate::keys`]).
    pub keys_file: PathBuf,
    /// File metering records are appended to, one JSON object per line.
    pub metering_log: PathBuf,
    /// Address the HTTP server listens on.
    pub bind: SocketAddr,
    /// Lifetime of a newly created index, in seconds, if the operator
    /// sets one. `None` means indexes do not expire.
    pub index_ttl_secs: Option<u64>,
}

impl Config {
    /// Reads configuration from the environment, applying documented defaults.
    ///
    /// Recognised variables: `CODETOPO_DATA_DIR`, `CODETOPO_KEYS_FILE`,
    /// `CODETOPO_METERING_LOG`, `CODETOPO_BIND`, `CODETOPO_INDEX_TTL_SECS`.
    /// An empty value is treated as unset so `CODETOPO_BIND=` behaves like an
    /// absent variable. A TTL that is present but not a non-negative integer
    /// is a startup error, like a bad bind address: guessing a lifetime for
    /// customer data would be worse than refusing to boot.
    pub fn from_env() -> Result<Self, ApiError> {
        let data_dir = env_path("CODETOPO_DATA_DIR", DEFAULT_DATA_DIR);
        let keys_file = env_path("CODETOPO_KEYS_FILE", DEFAULT_KEYS_FILE);
        let metering_log = env_path("CODETOPO_METERING_LOG", DEFAULT_METERING_LOG);
        let bind = env_string("CODETOPO_BIND", DEFAULT_BIND);
        let bind: SocketAddr = bind.parse().map_err(|_| {
            ApiError::Internal(format!(
                "CODETOPO_BIND must be a 'host:port' socket address, got '{bind}'"
            ))
        })?;
        let index_ttl_secs = match std::env::var("CODETOPO_INDEX_TTL_SECS") {
            Ok(value) if !value.trim().is_empty() => {
                Some(value.trim().parse::<u64>().map_err(|_| {
                    ApiError::Internal(format!(
                        "CODETOPO_INDEX_TTL_SECS must be a non-negative integer, got '{value}'"
                    ))
                })?)
            }
            _ => None,
        };
        Ok(Config {
            data_dir,
            keys_file,
            metering_log,
            bind,
            index_ttl_secs,
        })
    }

    /// Database path for one index id.
    pub fn index_db_path(&self, index_id: &str) -> PathBuf {
        self.data_dir.join(format!("{index_id}.db"))
    }

    /// Path of the runtime key store inside the data dir.
    pub fn keys_db_path(&self) -> PathBuf {
        self.data_dir.join(crate::keys::STORE_FILE_NAME)
    }

    /// Path of the tenancy store inside the data dir.
    pub fn tenancy_db_path(&self) -> PathBuf {
        self.data_dir.join(crate::tenancy::STORE_FILE_NAME)
    }

    /// Creates `data_dir` if absent and opens the runtime stores.
    ///
    /// Key bootstrap, in order: open `<data_dir>/keys.db`; if it holds no
    /// active key and the legacy keys file exists, import it. Fails closed
    /// (Phase 2 design §4) when no active key results: starting with zero
    /// usable keys would serve `401` for every request while looking
    /// healthy, which is harder to diagnose than refusing to boot.
    ///
    /// Also sweeps expired indexes: their ownership records are pruned
    /// and their database files deleted, so a data dir cannot grow on
    /// indexes nobody may read any more. The same refusal is enforced
    /// lazily per request (see [`crate::handlers`]).
    pub async fn prepare(&self) -> Result<(KeyStore, Tenancy), ApiError> {
        tokio::fs::create_dir_all(&self.data_dir)
            .await
            .map_err(|err| {
                ApiError::Internal(format!(
                    "cannot create data dir {}: {err}",
                    self.data_dir.display()
                ))
            })?;
        let keys = KeyStore::open(&self.keys_db_path())?;
        if keys.active_len() == 0 && self.keys_file.is_file() {
            keys.import_legacy_file(&self.keys_file)?;
        }
        if keys.active_len() == 0 {
            return Err(ApiError::Internal(format!(
                "no active API keys: issue one with `codetopo-server keys issue <key_id>` \
                 or provide a keys file at {}",
                self.keys_file.display()
            )));
        }
        let tenancy = Tenancy::open(&self.tenancy_db_path())?;
        for index_id in tenancy.prune_expired(now_unix_secs())? {
            let db = self.index_db_path(&index_id);
            if let Err(err) = tokio::fs::remove_file(&db).await {
                if err.kind() != std::io::ErrorKind::NotFound {
                    eprintln!(
                        "warning: cannot delete expired index {}: {err}",
                        db.display()
                    );
                }
            }
        }
        Ok((keys, tenancy))
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            data_dir: PathBuf::from(DEFAULT_DATA_DIR),
            keys_file: PathBuf::from(DEFAULT_KEYS_FILE),
            metering_log: PathBuf::from(DEFAULT_METERING_LOG),
            bind: DEFAULT_BIND.parse().expect("default bind address is valid"),
            index_ttl_secs: None,
        }
    }
}

fn env_string(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => default.to_string(),
    }
}

fn env_path(key: &str, default: &str) -> PathBuf {
    PathBuf::from(env_string(key, default))
}
