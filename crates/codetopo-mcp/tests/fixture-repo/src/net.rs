// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

/// The leaf of the fixture chain: nothing calls `send`, so its `descendants`
/// set is empty while its `ancestors` set has every other node.
pub fn send(payload: &str) -> String {
    format!("sent:{payload}")
}

/// `connect` is called only by `query`, so it sits directly below it in the
/// chain. Kept as a separate function so the fixture has a path longer than one
/// hop — a shortest-path test that only ever needs one edge proves nothing.
pub fn connect() -> String {
    send("fixture")
}
