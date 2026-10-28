// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Fixture entry point: one call chain four deep, so the closure routes have a
//! non-trivial answer to agree with `codetopo_core`.

mod alpha;
mod beta;

fn main() {
    alpha::entry();
    beta::entry();
}
