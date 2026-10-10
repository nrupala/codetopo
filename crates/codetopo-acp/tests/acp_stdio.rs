// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! Wire-level probe of the `codetopo-acp` binary: the `initialize`
//! response is the agent's public identity claim, so it is tested
//! against reality — the advertised version must be this crate's real
//! version (it was frozen at "0.1.0" while the crates moved to 0.2.x),
//! and `protocolVersion` must be the public ACP spec's integer (1),
//! not the string "1.0" the dialect started with.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

struct Agent {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Agent {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codetopo-acp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn codetopo-acp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self { child, stdin, stdout }
    }

    fn request(&mut self, frame: &serde_json::Value) -> serde_json::Value {
        writeln!(self.stdin, "{frame}").expect("write frame");
        self.stdin.flush().expect("flush");
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read response");
        serde_json::from_str(&line).expect("response is JSON")
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn initialize_advertises_the_real_version_and_spec_protocol() {
    let mut agent = Agent::start();
    let resp = agent.request(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }));
    let result = &resp["result"];
    assert_eq!(
        result["agentInfo"]["version"],
        env!("CARGO_PKG_VERSION"),
        "agentInfo.version must be the crate version, never a frozen literal"
    );
    assert_eq!(result["agentInfo"]["name"], "codetopo");
    assert_eq!(
        result["protocolVersion"], 1,
        "protocolVersion is the ACP spec's integer, not a string"
    );
    assert_eq!(result["agentCapabilities"]["read_only"], true);
}
