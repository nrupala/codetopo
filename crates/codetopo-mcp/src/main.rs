// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! `codetopo-mcp` — an MCP server exposing codetopo over stdio.
//!
//! The whole process is three steps: move stdout aside so progress output
//! cannot corrupt the protocol, loop over stdin handing each line to the
//! dispatcher in the library, and stop when the client closes the pipe. There
//! is no configuration and no argument parsing on purpose — an MCP client
//! spawns the server with a fixed command line, and every tool argument is
//! carried in the JSON-RPC frame instead.

use std::io::{self, BufReader};
use std::process::ExitCode;

use codetopo_mcp::stdio::{seal_stdout, serve, Sealed};

fn main() -> ExitCode {
    // Sealed before anything else can print: the first tool call may well be
    // `codetopo_index`, which reports progress on stdout.
    let mut sealed = match seal_stdout() {
        Ok(sealed) => sealed,
        Err(err) => {
            // Nothing has been read or written yet, so this is the only
            // chance to say why the server is about to be unsafe to talk to.
            // Reported on stderr precisely because fd 1 is in question.
            eprintln!("codetopo-mcp: cannot separate stdout from tool output: {err}");
            return ExitCode::FAILURE;
        }
    };

    match run(&mut sealed) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
            // The client hung up mid-frame. Its exit, not a server fault.
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("codetopo-mcp: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Serve requests until the client closes stdin.
fn run(sealed: &mut Sealed) -> io::Result<()> {
    let stdin = io::stdin();
    // The dispatcher hands back JSON; the transport wants text. Serializing
    // here rather than inside `serve` keeps the transport free of any JSON
    // type, so it stays a reusable loop and the protocol stays in one place.
    serve(BufReader::new(stdin.lock()), sealed, |line| {
        codetopo_mcp::handle_line(line).map(|response| response.to_string())
    })
}
