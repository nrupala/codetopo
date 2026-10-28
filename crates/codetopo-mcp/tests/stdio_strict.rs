// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! Strict-client conformance over the real stdio wire: spawn the
//! shipped `codetopo-mcp` binary and speak MCP to it exactly the
//! way an official SDK client does. This test exists because the
//! in-process tests were green while successful `tools/call`
//! results lacked the `content` array the spec (and the SDK result
//! schemas) require — a defect only a wire-level client can catch.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

struct Client {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

impl Client {
    fn spawn() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codetopo-mcp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn codetopo-mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self { child, stdin, rx }
    }

    fn send(&mut self, frame: &Value) {
        writeln!(self.stdin, "{}", frame).expect("write frame");
        self.stdin.flush().expect("flush");
    }

    fn next(&self) -> Value {
        let line = self
            .rx
            .recv_timeout(Duration::from_secs(60))
            .expect("a frame within 60s");
        serde_json::from_str(&line).expect("every stdout frame is JSON")
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        self.next()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn strict_stdio_client_session() {
    let mut client = Client::spawn();

    // initialize: the negotiation a real client performs first.
    let init = client.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "strict-conformance-test", "version": "0"},
        }),
    );
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(init["result"]["serverInfo"]["name"], "codetopo");
    assert_eq!(
        init["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // A notification is answered with silence: the next frame on
    // the wire must be the ping response, proving nothing was
    // emitted for the notification in between.
    client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let pong = client.request(2, "ping", json!({}));
    assert_eq!(pong["id"], 2, "first frame after the notification is the ping reply");

    // tools/list: every tool carries a name, a description, and an
    // object input schema — the fields SDK clients validate.
    let list = client.request(3, "tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 17);
    for tool in tools {
        assert!(tool["name"].is_string(), "tool name: {tool}");
        assert!(tool["description"].is_string(), "tool description: {tool}");
        assert_eq!(tool["inputSchema"]["type"], "object", "tool schema: {tool}");
    }

    // Index a fixture through the wire, then read it back.
    let dir = std::env::temp_dir().join(format!("codetopo-mcp-stdio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::write(dir.join("a.rs"), "pub fn root() { helper(); }\nfn helper() {}\n").expect("fixture");
    let db = dir.join("graph.db");

    let indexed = client.request(
        4,
        "tools/call",
        json!({"name": "codetopo_index", "arguments": {
            "repo_path": dir.to_string_lossy(),
            "db_path": db.to_string_lossy(),
        }}),
    );
    let result = &indexed["result"];
    assert_eq!(result["isError"], false, "index call succeeds: {indexed}");
    // The spec shape: content[0] is a text block whose text is the
    // result object as JSON, and structuredContent carries the
    // same object — the two must agree exactly.
    let text = result["content"][0]["text"]
        .as_str()
        .expect("success results carry a text content block");
    assert_eq!(result["content"][0]["type"], "text");
    let body: Value = serde_json::from_str(text).expect("content text is the result JSON");
    assert_eq!(body, result["structuredContent"]);
    let nodes = body["nodes"].as_u64().expect("node count");
    assert!(nodes >= 3, "fixture indexes its symbols: {body}");

    let stats = client.request(
        5,
        "tools/call",
        json!({"name": "codetopo_stats", "arguments": {"db_path": db.to_string_lossy()}}),
    );
    let stats_text = stats["result"]["content"][0]["text"]
        .as_str()
        .expect("stats content");
    let stats_body: Value = serde_json::from_str(stats_text).expect("stats JSON");
    assert_eq!(stats_body["nodes"].as_u64(), Some(nodes));

    // A failing tool is a RESULT with isError, never a transport
    // error — the model must see the prose and retry.
    let bad = client.request(
        6,
        "tools/call",
        json!({"name": "codetopo_nope", "arguments": {}}),
    );
    assert_eq!(bad["result"]["isError"], true);
    assert!(bad.get("error").is_none(), "tool errors are not transport errors: {bad}");

    // An unknown METHOD is a transport error: -32601.
    let unknown = client.request(7, "resources/list", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);

    let _ = std::fs::remove_dir_all(&dir);
}
