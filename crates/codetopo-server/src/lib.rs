// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! HTTP surface for codetopo (Phase 2 design §3, §4).

pub mod auth;
pub mod config;
pub mod error;
pub mod handlers;
pub mod keys;
pub mod metering;

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
) -> axum::Router {
    let auth = std::sync::Arc::new(auth::AuthMetering::new(keys, metering));
    auth::metered(router(AppState::new(config)), auth)
}
