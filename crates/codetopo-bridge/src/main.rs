// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! `codetopo-bridge` entry point.
//!
//! Startup is fail-fast, like the server's: a missing webhook secret,
//! price id, or claim key aborts before the listener is bound, because
//! a bridge that cannot verify signatures or cannot seal deliveries
//! must not look healthy while dropping purchases on the floor.
//!
//! The bridge opens the same `keys.db` the server reads (via the
//! server crate's own configuration, so the two processes cannot
//! disagree about the path), runs one sweep at startup, and then sweeps
//! every minute: due revocations and expired deliveries never wait for
//! the next purchase to be applied.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use codetopo_server::keys::KeyStore;
use tokio::net::TcpListener;

use codetopo_bridge::config::BridgeConfig;
use codetopo_bridge::http::{router, BridgeState};
use codetopo_bridge::service::{sweep, Policy};
use codetopo_bridge::store::BridgeStore;
use codetopo_bridge::BridgeError;

/// How often due revocations and expired deliveries are applied.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(err) = runtime.block_on(run()) {
        eprintln!("codetopo-bridge: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), BridgeError> {
    let config = BridgeConfig::from_env()?;
    tokio::fs::create_dir_all(&config.data_dir).await.map_err(|err| {
        BridgeError::Config(format!(
            "cannot create data dir {}: {err}",
            config.data_dir.display()
        ))
    })?;
    let keys = KeyStore::open(&config.keys_db_path)?;
    let store = BridgeStore::open(&config.keys_db_path)?;

    let startup = sweep(&store, &keys, now_unix_secs())?;
    if startup.revoked > 0 || startup.purged_deliveries > 0 {
        eprintln!(
            "codetopo-bridge: startup sweep revoked {} key(s), purged {} expired deliver(ies)",
            startup.revoked, startup.purged_deliveries
        );
    }

    let state = Arc::new(BridgeState {
        store,
        keys,
        policy: Policy {
            price_id: config.price_id.clone(),
            plan: config.plan.clone(),
            claim_key: config.claim_key,
        },
        webhook_secret: config.webhook_secret.clone(),
    });

    // The periodic sweep shares the state; the stores serialise
    // internally, so no additional locking is needed here.
    let sweep_state = Arc::clone(&state);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        loop {
            ticker.tick().await;
            if let Err(err) = sweep(&sweep_state.store, &sweep_state.keys, now_unix_secs()) {
                eprintln!("codetopo-bridge: sweep failed: {err}");
            }
        }
    });

    let listener = TcpListener::bind(config.bind).await.map_err(|err| {
        BridgeError::Config(format!("cannot bind {}: {err}", config.bind))
    })?;
    eprintln!(
        "codetopo-bridge: listening on http://{} (key store {}, price {})",
        config.bind,
        config.keys_db_path.display(),
        config.price_id,
    );
    axum::serve(listener, router(state))
        .await
        .map_err(|err| BridgeError::Config(format!("bridge stopped: {err}")))?;
    Ok(())
}

/// Current time as Unix seconds; 0 if the clock predates the epoch.
fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
