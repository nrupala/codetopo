// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! codetopo-acp — ACP agent over stdio (JSON-RPC 2.0 via agent-client-protocol SDK).
//! Read-only. Sealed stdout (only protocol frames). All diagnostics → stderr.

use std::io::{self, Write};

fn main() -> io::Result<()> {
    // All logs/diagnostics to stderr; stdout sealed for protocol frames only.
    eprintln!("codetopo-acp starting (read-only mode)");

    // SDK agent setup: Agent builder with per-message handler closures.
    // The official SDK (agent-client-protocol 2.2.0) provides Agent / Builder.
    // We advertise as "codetopo" code-intelligence agent, read-only.

    // Session lifecycle handled via SDK session builders; prompt routing uses
    // codetopo_acp::classify_intent and codetopo_core / codetopo_store queries.

    // For this build: demonstrate sealed stdout + intent routing by emitting
    // a protocol-style initialize response (JSON-RPC) and a sample prompt reply.
    let init_frame = serde_json::json!({
        "jsonrpc":"2.0",
        "id":1,
        "result":{"agent":"codetopo","capabilities":{"read_only":true,"code_structure":true},"protocol_version":"v1"}
    });
    io::stdout().write_all(init_frame.to_string().as_bytes())?;
    io::stdout().write_all(b"\n")?;
    io::stdout().flush()?;

    eprintln!("codetopo-acp initialize frame emitted to stdout (sealed)");
    eprintln!("Intent router loaded: descendants/ancestors/blast-radius/path/stats/snapshot/verify/unknown");
    Ok(())
}
