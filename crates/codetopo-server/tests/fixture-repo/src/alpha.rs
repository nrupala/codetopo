// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! One side of the fixture call graph: `entry -> helper_a -> helper_b`.

pub fn entry() {
    helper_a();
}

fn helper_a() {
    helper_b();
}

fn helper_b() {}
