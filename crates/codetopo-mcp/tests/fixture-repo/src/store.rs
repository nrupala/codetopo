// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

use crate::net;

/// The middle of the fixture chain: called by `api::handle`, calls
/// `net::connect`. Two hops on each side of it, so `path(handle, send)` is a
/// four-step walk rather than a direct edge.
pub fn query() -> String {
    net::connect()
}
