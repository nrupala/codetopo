// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! ACP conformance over the real stdio wire: spawn the shipped
//! `codetopo-acp` binary and speak the Agent Client Protocol to it
//! the way an editor client (Zed and peers) does — integer
//! protocolVersion, content-block prompts, `session/update`
//! message chunks, stopReasons. The agent previously spoke a
//! home-grown dialect (string "1.0", string prompts, custom result
//! fields) that no real client could drive; this test is the
//! contract that keeps it honest.

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
        let mut child = Command::new(env!("CARGO_BIN_EXE_codetopo-acp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn codetopo-acp");
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

    fn send_raw(&mut self, line: &str) {
        writeln!(self.stdin, "{}", line).expect("write frame");
        self.stdin.flush().expect("flush");
    }

    fn send(&mut self, frame: &Value) {
        self.send_raw(&frame.to_string());
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

    /// Read frames until the response for `id` arrives; return it
    /// together with every notification seen on the way.
    fn request_collecting(&mut self, id: u64, method: &str, params: Value) -> (Value, Vec<Value>) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut notes = Vec::new();
        loop {
            let frame = self.next();
            if frame.get("id").and_then(Value::as_u64) == Some(id) {
                return (frame, notes);
            }
            notes.push(frame);
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn prompt(client: &mut Client, id: u64, session_id: &str, text: &str) -> (Value, Vec<Value>) {
    client.request_collecting(
        id,
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
    )
}

fn chunk_texts(notes: &[Value]) -> Vec<String> {
    notes
        .iter()
        .filter(|n| n["method"] == "session/update")
        .filter(|n| n["params"]["update"]["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|n| n["params"]["update"]["content"]["text"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn acp_stdio_conformance_session() {
    let mut client = Client::spawn();

    // initialize: protocolVersion is the INTEGER 1, capabilities
    // are the spec's shape, agentInfo carries the crate version.
    let init = client.request(
        1,
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {"name": "conformance-test", "version": "0"},
        }),
    );
    assert_eq!(init["result"]["protocolVersion"], json!(1));
    assert_eq!(init["result"]["agentInfo"]["name"], "codetopo");
    assert_eq!(
        init["result"]["agentInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(init["result"]["agentCapabilities"]["loadSession"], json!(false));

    // Fixture repo for the session to index.
    let dir = std::env::temp_dir().join(format!("codetopo-acp-stdio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::write(dir.join("a.rs"), "pub fn root() { helper(); }\nfn helper() {}\n").expect("fixture");

    // session/new: the agent mints the id; the result is exactly
    // { sessionId }.
    let created = client.request(
        2,
        "session/new",
        json!({"cwd": dir.to_string_lossy(), "mcpServers": []}),
    );
    let session_id = created["result"]["sessionId"]
        .as_str()
        .expect("sessionId string")
        .to_string();
    assert!(!session_id.is_empty());

    // A stats prompt: the answer streams as an agent_message_chunk
    // session/update, and the turn ends with stopReason end_turn.
    let (resp, notes) = prompt(&mut client, 3, &session_id, "stats");
    let chunks = chunk_texts(&notes);
    assert!(
        chunks.iter().any(|t| t.contains("Graph stats")),
        "answer streamed as a message chunk: {chunks:?}"
    );
    assert_eq!(resp["result"]["stopReason"], "end_turn");

    // An edit request is refused — in the spec's terms: the
    // refusal text streams, the turn ends with stopReason refusal.
    let (resp, notes) = prompt(&mut client, 4, &session_id, "rename root to main");
    let chunks = chunk_texts(&notes);
    assert!(
        chunks.iter().any(|t| t.contains("read-only")),
        "refusal streamed as a message chunk: {chunks:?}"
    );
    assert_eq!(resp["result"]["stopReason"], "refusal");

    // session/cancel is a notification: NO frame may answer it.
    // The next prompt's frames prove it — the first frame after
    // the cancel belongs to the new prompt.
    client.send(&json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session_id}}));
    let (resp, notes) = prompt(&mut client, 5, &session_id, "stats");
    assert_eq!(resp["result"]["stopReason"], "end_turn");
    assert!(
        notes.iter().all(|n| n["method"] == "session/update"),
        "only the new prompt's updates follow the cancel: {notes:?}"
    );

    // Unknown session: a server error, not a hang or a panic.
    let (resp, _) = prompt(&mut client, 6, "sess-does-not-exist", "stats");
    assert_eq!(resp["error"]["code"], -32001);

    // Unknown method: -32601. Malformed line: -32700 with null id.
    let unknown = client.request(7, "session/definitely-not-a-method", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);
    client.send_raw("this is not json");
    let parse = client.next();
    assert_eq!(parse["error"]["code"], -32700);
    assert!(parse["id"].is_null());

    let _ = std::fs::remove_dir_all(&dir);
}
