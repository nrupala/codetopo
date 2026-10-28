// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! HTTP surface for codetopo (Phase 2 design §3, §4).

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod ingest;
pub mod keys;
pub mod limits;
pub mod metering;
pub mod tenancy;

pub use handlers::{router, AppState};

/// Builds the served application: routes, then auth + metering around them.
///
/// The layering order is the one §4/§5 require — [`auth::metered`] is a
/// `route_layer`, so a request is authenticated *before* it can reach a
/// handler and a rejected one is answered without touching a database or
/// appending a metering record.
pub fn app(
    config: std::sync::Arc<config::Config>,
    keys: std::sync::Arc<keys::KeyStore>,
    metering: std::sync::Arc<metering::Metering>,
    tenancy: std::sync::Arc<tenancy::Tenancy>,
) -> axum::Router {
    let auth = std::sync::Arc::new(auth::AuthMetering::new(std::sync::Arc::clone(&keys), metering));
    auth::metered(router(AppState::new(config, keys, tenancy)), auth)
}
