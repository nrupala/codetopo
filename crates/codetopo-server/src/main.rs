// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! `codetopo-server` entry point.
//!
//! Startup is deliberately fail-fast: no usable key in the key store, an
//! unparseable bind address, or a data dir that cannot be created all abort
//! before the listener is bound. Serving an API that cannot authenticate, or
//! that would attribute every request to nobody, is worse than not serving.
//!
//! ## The admin path is a subcommand, not an endpoint
//!
//! Key administration (`keys issue|rotate|revoke|list`) runs as a CLI
//! subcommand against the same key store file the server reads. The
//! alternative — an HTTP admin endpoint — would add the network surface
//! this server deliberately does not have (there is no unauthenticated
//! route, not even health): a second listener, a second credential, and
//! a remotely reachable way to mint keys. A subcommand needs shell
//! access to the host, which is already the trust boundary for the data
//! dir itself, and the store is read on every request, so changes take
//! effect on the running server with no restart and no signalling.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::net::TcpListener;

use codetopo_server::config::Config;
use codetopo_server::error::ApiError;
use codetopo_server::keys::{KeyStore, DEFAULT_PLAN};
use codetopo_server::limits::plan_limits;
use codetopo_server::metering::{current_month, now_rfc3339_millis, Metering};
use codetopo_server::tenancy::Tenancy;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("keys") {
        std::process::exit(match admin_keys(&args[2..]) {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("codetopo-server keys: {err}");
                1
            }
        });
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    if let Err(err) = runtime.block_on(run()) {
        eprintln!("codetopo-server: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), ApiError> {
    let config = Config::from_env()?;
    let (keys, tenancy) = config.prepare().await?;
    let keys = Arc::new(keys);
    let tenancy = Arc::new(tenancy);
    let metering = Arc::new(Metering::new(&config.metering_log));

    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|err| ApiError::internal(format!("cannot bind {}: {err}", config.bind)))?;

    let app = codetopo_server::app(Arc::new(config.clone()), keys, metering, tenancy);
    eprintln!(
        "codetopo-server: listening on http://{} (data dir {}, key store {}, metering {})",
        config.bind,
        config.data_dir.display(),
        config.keys_db_path().display(),
        config.metering_log.display(),
    );

    axum::serve(listener, app)
        .await
        .map_err(|err| ApiError::internal(format!("server stopped: {err}")))?;

    Ok(())
}

const KEYS_USAGE: &str = "usage: codetopo-server keys <issue|rotate|revoke|list> [key_id] [--plan <name>]";

/// Administers keys in the runtime store, using the same environment
/// configuration as the server so both always name the same data dir.
///
/// `issue` and `rotate` print the new credential once, as
/// `key_id:secret` — the form a client pastes into its bearer token —
/// because the store keeps only a hash and the secret cannot be shown
/// again. `list` never prints a secret or a hash.
fn admin_keys(args: &[String]) -> Result<(), ApiError> {
    let config = Config::from_env()?;
    std::fs::create_dir_all(&config.data_dir).map_err(|err| {
        ApiError::Internal(format!(
            "cannot create data dir {}: {err}",
            config.data_dir.display()
        ))
    })?;
    let keys = KeyStore::open(&config.keys_db_path())?;
    // Same bootstrap as the server: a data dir that only ever had the
    // legacy flat file imports it first, so `keys list` shows the keys
    // the server would serve with.
    if keys.active_len() == 0 && config.keys_file.is_file() {
        keys.import_legacy_file(&config.keys_file)?;
    }

    let command = args.first().map(String::as_str);
    let key_id = args.get(1).map(String::as_str);
    match (command, key_id) {
        (Some("issue"), Some(key_id)) => {
            let secret = keys.issue(key_id, plan_arg(args))?;
            println!("{key_id}:{secret}");
        }
        (Some("rotate"), Some(key_id)) => {
            let secret = keys.rotate(key_id)?;
            println!("{key_id}:{secret}");
        }
        (Some("revoke"), Some(key_id)) => {
            keys.revoke(key_id)?;
            println!("revoked {key_id}");
        }
        (Some("list"), _) => {
            // Usage beside the limits it is measured against
            // (paid-path S3): the key's active indexes from the
            // tenancy store and its metered queries for the current
            // UTC month from the metering log — the same numbers the
            // envelope enforces, so an operator reads the tenant's
            // position exactly as the server computes it. This is the
            // operator's window into the envelope; the HTTP surface
            // deliberately gains no admin route for it (see the
            // module docs on why administration is a subcommand).
            let tenancy = Tenancy::open(&config.tenancy_db_path())?;
            let metering = Metering::new(&config.metering_log);
            let month = current_month();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            for summary in keys.list()? {
                let rotated = summary
                    .rotated_at
                    .map(|secs| format!("\trotated {}", rfc3339(secs)))
                    .unwrap_or_default();
                let limits = plan_limits(&summary.plan);
                let indexes = tenancy
                    .count_active_for_owner(&summary.key_id, now)
                    .unwrap_or(0);
                let usage = metering.usage_in_month(&summary.key_id, &month);
                println!(
                    "{}\t{}\t{}\tcreated {}{}\tindexes {} (limit {})\tqueries {} (limit {}, {month} UTC)",
                    summary.key_id,
                    summary.plan,
                    summary.status,
                    rfc3339(summary.created_at),
                    rotated,
                    indexes,
                    limit_text(limits.max_active_indexes),
                    usage.queries,
                    limit_text(limits.max_queries_per_month),
                );
            }
        }
        _ => return Err(ApiError::internal(KEYS_USAGE)),
    }
    Ok(())
}

/// A plan limit as `keys list` prints it: the number, or
/// `unlimited` when the plan carries none.
fn limit_text<T: std::fmt::Display>(limit: Option<T>) -> String {
    limit
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unlimited".to_string())
}

/// The `--plan <name>` value in an admin command, or the default plan.
fn plan_arg(args: &[String]) -> &str {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--plan" {
            if let Some(value) = iter.next() {
                return value;
            }
        }
    }
    DEFAULT_PLAN
}

/// Formats Unix seconds with the metering log's RFC 3339 formatter.
fn rfc3339(secs: i64) -> String {
    let time = if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        UNIX_EPOCH
    };
    now_rfc3339_millis(time)
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

    /// `--plan` is read from anywhere in the arguments; absent, the
    /// default plan applies so the common command stays short.
    #[test]
    fn plan_arg_defaults_and_reads_the_flag() {
        let plain: Vec<String> = vec!["issue".into(), "tenant-a".into()];
        assert_eq!(plan_arg(&plain), DEFAULT_PLAN);
        let flagged: Vec<String> = vec![
            "issue".into(),
            "tenant-a".into(),
            "--plan".into(),
            "trial".into(),
        ];
        assert_eq!(plan_arg(&flagged), "trial");
    }
}
