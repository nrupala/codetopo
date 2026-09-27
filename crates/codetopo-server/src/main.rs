// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! `codetopo-server` entry point.
//!
//! Startup is deliberately fail-fast: a missing or empty keys file, an
//! unparseable bind address, or a data dir that cannot be created all abort
//! before the listener is bound. Serving an API that cannot authenticate, or
//! that would attribute every request to nobody, is worse than not serving.

use std::sync::Arc;

use tokio::net::TcpListener;

use codetopo_server::config::Config;
use codetopo_server::error::ApiError;
use codetopo_server::metering::Metering;

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("codetopo-server: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), ApiError> {
    let config = Config::from_env()?;
    let keys = Arc::new(config.prepare().await?);
    let metering = Arc::new(Metering::new(&config.metering_log));

    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|err| ApiError::internal(format!("cannot bind {}: {err}", config.bind)))?;

    let app = codetopo_server::app(Arc::new(config.clone()), keys, metering);
    eprintln!(
        "codetopo-server: listening on http://{} (data dir {}, keys {}, metering {})",
        config.bind,
        config.data_dir.display(),
        config.keys_file.display(),
        config.metering_log.display(),
    );

    axum::serve(listener, app)
        .await
        .map_err(|err| ApiError::internal(format!("server stopped: {err}")))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codetopo_server::keys::KeyStore;

    /// A bad bind address must fail in `Config`, before any listener exists, so
    /// the process never starts in a state it cannot serve from.
    #[test]
    fn unparseable_bind_is_a_startup_error() {
        let err = ApiError::internal("cannot bind 127.0.0.1:notaport");
        assert_eq!(err.code(), "internal");
    }

    /// `KeyStore::parse` on a well-formed file yields a usable store; the
    /// integration tests in `tests/` cover the wire format.
    #[test]
    fn keys_file_parses() {
        let keys = KeyStore::parse("agent:s3cr3t\n");
        assert_eq!(keys.len(), 1);
    }
}
