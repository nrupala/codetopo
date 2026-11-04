// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join).

//! The `cycle-trap` fixture (SPEC §7): a planted 3-cycle inside a
//! DAG. The cycle must come back as one `cyclic: true` group, the
//! rest in valid dependency order — verified first by the order
//! property (checked mechanically over every edge), then by the
//! exact expected groups.

use std::collections::HashMap;
use std::path::PathBuf;

use codetopo_core::{AnalyticsProjection, Snapshot};

#[test]
fn cycle_trap_orders_dependencies_first_and_flags_the_cycle() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/oracle");
    let snap = Snapshot::from_json(
        &std::fs::read_to_string(dir.join("cycle-trap.json")).expect("fixture reads"),
    )
    .expect("fixture parses");
    let graph = snap.to_graph();
    let order = graph.topo_sort();

    // The order property over every projected edge.
    let proj = AnalyticsProjection::build(&graph);
    let mut pos: HashMap<&str, usize> = HashMap::new();
    for (i, group) in order.groups.iter().enumerate() {
        for m in &group.members {
            pos.insert(m.as_str(), i);
        }
    }
    for &(a, b) in proj.directed_edges() {
        let (ga, gb) = (pos[proj.node_id(a)], pos[proj.node_id(b)]);
        if ga != gb {
            assert!(
                gb < ga,
                "dependency {} (group {gb}) must precede {} (group {ga})",
                proj.node_id(b),
                proj.node_id(a)
            );
        }
    }

    // The exact answer: lib first (the cycle and mid both need it),
    // the cycle as one flagged group, then mid, then app.
    let shape: Vec<(Vec<&str>, bool)> = order
        .groups
        .iter()
        .map(|g| (g.members.iter().map(|s| s.as_str()).collect(), g.cyclic))
        .collect();
    assert_eq!(
        shape,
        vec![
            (vec!["lib"], false),
            (vec!["x", "y", "z"], true),
            (vec!["mid"], false),
            (vec!["app"], false),
        ]
    );
    assert_eq!(order.total_groups, 4);
    assert_eq!(order.total_nodes, 6);
    assert_eq!(order.cyclic_groups, 1);

    // Run-twice determinism, byte-identical (SPEC §7).
    assert_eq!(
        serde_json::to_vec(&order).unwrap(),
        serde_json::to_vec(&graph.topo_sort()).unwrap()
    );
}
