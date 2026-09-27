// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! One side of the fixture call graph: `entry -> helper_a -> helper_b`.

pub fn entry() {
    helper_a();
}

fn helper_a() {
    helper_b();
}

fn helper_b() {}
