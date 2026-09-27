// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Fixture entry point: one call chain four deep, so the closure routes have a
//! non-trivial answer to agree with `codetopo_core`.

mod alpha;
mod beta;

fn main() {
    alpha::entry();
    beta::entry();
}
