// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Runtime configuration, all of it from the environment.

use std::net::SocketAddr;
use std::path::PathBuf;

use crate::error::ApiError;
use crate::keys::KeyStore;

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
    /// File the bearer keys are read from at startup.
    pub keys_file: PathBuf,
    /// File metering records are appended to, one JSON object per line.
    pub metering_log: PathBuf,
    /// Address the HTTP server listens on.
    pub bind: SocketAddr,
}

impl Config {
    /// Reads configuration from the environment, applying documented defaults.
    ///
    /// Recognised variables: `CODETOPO_DATA_DIR`, `CODETOPO_KEYS_FILE`,
    /// `CODETOPO_METERING_LOG`, `CODETOPO_BIND`. An empty value is treated as
    /// unset so `CODETOPO_BIND=` behaves like an absent variable.
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
        Ok(Config {
            data_dir,
            keys_file,
            metering_log,
            bind,
        })
    }

    /// Database path for one index id.
    pub fn index_db_path(&self, index_id: &str) -> PathBuf {
        self.data_dir.join(format!("{index_id}.db"))
    }

    /// Creates `data_dir` if absent and loads the keys file.
    pub async fn prepare(&self) -> Result<KeyStore, ApiError> {
        tokio::fs::create_dir_all(&self.data_dir)
            .await
            .map_err(|err| {
                ApiError::Internal(format!(
                    "cannot create data dir {}: {err}",
                    self.data_dir.display()
                ))
            })?;
        KeyStore::load(&self.keys_file)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            data_dir: PathBuf::from(DEFAULT_DATA_DIR),
            keys_file: PathBuf::from(DEFAULT_KEYS_FILE),
            metering_log: PathBuf::from(DEFAULT_METERING_LOG),
            bind: DEFAULT_BIND.parse().expect("default bind address is valid"),
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
