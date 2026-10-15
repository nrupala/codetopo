// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Bridge configuration, all of it from the environment.
//!
//! The data dir comes from the server crate's own [`Config`], so the
//! bridge and the server always name the same `keys.db` — there is no
//! second path variable to drift. Everything Stripe-shaped is required
//! and has no default: a bridge that boots without its webhook secret or
//! its price id would either reject every real event or mint keys for
//! the wrong product, and both failures are quieter than refusing to
//! start.

use std::net::SocketAddr;
use std::path::PathBuf;

use codetopo_server::config::Config as ServerConfig;
use codetopo_server::keys::DEFAULT_PLAN;

use crate::BridgeError;

/// Default bind for the bridge listener: loopback only, like the
/// server's. A deployment puts a reverse proxy in front (paid-path S5);
/// exposing the listener is that proxy's decision, not a default here.
pub const DEFAULT_BRIDGE_BIND: &str = "127.0.0.1:8788";

/// Resolved bridge configuration.
#[derive(Debug, Clone)]
pub struct BridgeConfig {
    /// Directory holding the key store (from the server's config).
    pub data_dir: PathBuf,
    /// Path of the shared runtime key store, `<data_dir>/keys.db`.
    pub keys_db_path: PathBuf,
    /// Address the bridge listener binds.
    pub bind: SocketAddr,
    /// Stripe webhook endpoint signing secret (`whsec_…`). Verifies
    /// webhook signatures; never logged, never defaulted.
    pub webhook_secret: String,
    /// The Stripe price id that maps to the hosted plan. Price ids are
    /// configuration, not code (the Conduit pattern): a new price is an
    /// env change, not a release.
    pub price_id: String,
    /// Plan recorded on keys the bridge issues.
    pub plan: String,
    /// AES-256 key sealing unclaimed key deliveries, as 64 hex chars
    /// decoded to 32 bytes. Exists only in the environment.
    pub claim_key: [u8; 32],
}

impl BridgeConfig {
    /// Reads configuration from the environment, failing closed.
    ///
    /// Recognised variables: the server's `CODETOPO_DATA_DIR` (and its
    /// siblings, via [`ServerConfig::from_env`]), plus
    /// `CODETOPO_BRIDGE_BIND`, `CODETOPO_STRIPE_WEBHOOK_SECRET`,
    /// `CODETOPO_STRIPE_PRICE_ID`, `CODETOPO_CLAIM_KEY`, and optionally
    /// `CODETOPO_PLAN` (default: the hosted plan).
    pub fn from_env() -> Result<Self, BridgeError> {
        let server = ServerConfig::from_env().map_err(|err| {
            BridgeError::Config(format!("server configuration is unusable: {err}"))
        })?;
        let bind_raw = env_string("CODETOPO_BRIDGE_BIND", DEFAULT_BRIDGE_BIND);
        let bind: SocketAddr = bind_raw.parse().map_err(|_| {
            BridgeError::Config(format!(
                "CODETOPO_BRIDGE_BIND must be a 'host:port' socket address, got '{bind_raw}'"
            ))
        })?;
        let webhook_secret = required("CODETOPO_STRIPE_WEBHOOK_SECRET")?;
        let price_id = required("CODETOPO_STRIPE_PRICE_ID")?;
        let plan = env_string("CODETOPO_PLAN", DEFAULT_PLAN);
        let claim_key = parse_claim_key(&required("CODETOPO_CLAIM_KEY")?)?;
        let keys_db_path = server.keys_db_path();
        Ok(BridgeConfig {
            data_dir: server.data_dir,
            keys_db_path,
            bind,
            webhook_secret,
            price_id,
            plan,
            claim_key,
        })
    }
}

/// A required variable: absent or empty is a configuration error.
fn required(name: &str) -> Result<String, BridgeError> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        _ => Err(BridgeError::Config(format!(
            "{name} must be set (no default exists)"
        ))),
    }
}

fn env_string(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => default.to_string(),
    }
}

/// Decodes the 64-hex-character claim key. Hand-rolled hex (one small
/// decoder, shared with [`crate::stripe`]) — a malformed key is a
/// startup error, never a silent truncation.
fn parse_claim_key(raw: &str) -> Result<[u8; 32], BridgeError> {
    let bytes = crate::stripe::hex_decode(raw).ok_or_else(|| {
        BridgeError::Config("CODETOPO_CLAIM_KEY must be 64 hex characters".to_string())
    })?;
    bytes.try_into().map_err(|_| {
        BridgeError::Config("CODETOPO_CLAIM_KEY must be 64 hex characters (32 bytes)".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_key_must_be_exactly_32_hex_bytes() {
        let good = "ab".repeat(32);
        assert_eq!(parse_claim_key(&good).expect("32 bytes").len(), 32);
        assert!(parse_claim_key("abcd").is_err(), "too short");
        assert!(parse_claim_key(&"zz".repeat(32)).is_err(), "not hex");
        assert!(parse_claim_key(&"ab".repeat(33)).is_err(), "too long");
    }
}
