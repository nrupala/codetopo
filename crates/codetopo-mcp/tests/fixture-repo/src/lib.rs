// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Fixture repository for the codetopo Phase 2 API tests.
//!
//! The shape is deliberate: a single call chain
//! `api::handle -> store::query -> net::connect -> net::send` with no cycles,
//! so a Phase 2 test can assert exact closure sizes instead of "at least N".

pub mod net;
pub mod store;

/// `api::handle` is the ancestor of every other node in the fixture, which makes
/// it the one node whose `ancestors` and `descendants` sets are both non-empty.
pub fn handle() -> String {
    store::query()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_is_a_single_unbroken_chain() {
        assert_eq!(net::send(), format!("sent:{}", store::query()));
    }
}
