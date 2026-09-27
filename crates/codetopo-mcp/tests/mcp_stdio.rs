// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! The stdio contract, exercised through the real binary.
//!
//! The unit tests in `src/stdio.rs` share one process with the code under
//! test, so they can only prove the framing. They cannot prove the seal:
//! redirecting fd 1 would silence the test harness's own output. This suite
//! spawns `codetopo-mcp` as a child, so the redirect lands in a process
//! nobody is printing test results from — and if a tool's `println!` reaches
//! the pipe, the frame this suite is reading is not the frame it expected.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use codetopo_core::NodeKind;
use serde_json::{json, Value};

/// The fixture repository: one chain, `handle -> query -> connect -> send`.
fn fixture_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture-repo")
}

/// A running `codetopo-mcp`, spoken to over the pipes a real client uses.
struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Client {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_codetopo-mcp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // The child has no test harness to report through, so its stderr is
            // discarded rather than inherited: an extractor warning must not
            // read as a failure of this suite.
            .stderr(Stdio::null())
            .spawn()
            .expect("the server binary was built by cargo test");
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout was piped"));
        Client {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, frame: &Value) {
        writeln!(self.stdin, "{frame}").expect("the server is still reading");
        self.stdin.flush().expect("the pipe is writable");
    }

    /// Read one frame.
    ///
    /// Panics on anything that is not JSON, naming the offending bytes. That is
    /// the failure this suite exists to catch: an unsealed `println!` from a
    /// tool arrives here as a parse error instead of silently corrupting a
    /// client's stream.
    fn recv(&mut self) -> Value {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .expect("the server is still writing");
        assert!(read > 0, "the server closed stdout without answering");
        serde_json::from_str(line.trim())
            .unwrap_or_else(|err| panic!("stdout carried a non-JSON frame {line:?}: {err}"))
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let response = self.recv();
        assert_eq!(
            response["id"],
            json!(id),
            "a response must answer its own id"
        );
        response
    }

    /// Close the client's end, wait for the server, and hand back whatever is
    /// still on stdout.
    fn close(mut self) -> (Option<i32>, String) {
        drop(self.stdin);
        let code = self.child.wait().expect("the server exits at EOF").code();
        let mut trailing = String::new();
        self.stdout
            .read_to_string(&mut trailing)
            .expect("stdout reads to EOF");
        (code, trailing)
    }
}

/// The id of the fixture node labelled `label`, read straight from the store.
fn id_of(db_path: &Path, label: &str) -> String {
    let graph = codetopo_cli::load_graph_from_db(db_path).expect("the fixture index loads");
    let id = graph
        .nodes()
        .find(|node| node.kind == NodeKind::Function && node.label == label)
        .unwrap_or_else(|| panic!("the fixture defines a function named {label}"))
        .id
        .clone();
    id
}

#[test]
fn stdout_is_json_only_while_a_tool_prints() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = dir.path().join("seal.db");

    let mut client = Client::start();
    client.request(1, "initialize", json!({}));
    client.request(2, "tools/list", json!({}));

    // `codetopo_index` walks the fixture and prints a seven-line aggregate
    // with `println!`, onto the very stdout this client is reading. With fd 1
    // sealed those lines go to /dev/null; unsealed, they would land ahead of
    // the response and every real client would hang parsing them.
    let indexed = client.request(
        3,
        "tools/call",
        json!({
            "name": "codetopo_index",
            "arguments": {"repo_path": fixture_repo(), "db_path": db},
        }),
    );
    let report = &indexed["result"];
    assert!(
        report["files_indexed"].as_u64().expect("a count") > 0,
        "the fixture indexed no files: {indexed}"
    );
    assert!(
        report["nodes"].as_u64().expect("a count") > 0,
        "no nodes: {indexed}"
    );

    // Nothing follows the last response either: the seal is a redirect, not a
    // filter that happens to be quiet on the way out.
    let (code, trailing) = client.close();
    assert_eq!(
        trailing, "",
        "stdout carried bytes after the final response"
    );
    assert_eq!(code, Some(0), "the server exits cleanly at EOF");
}

#[test]
fn a_session_answers_every_step_of_the_read_only_recipe() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = dir.path().join("session.db");
    let db = db.to_str().expect("a UTF-8 temp path");

    let mut client = Client::start();

    let hello = client.request(1, "initialize", json!({}));
    assert_eq!(hello["result"]["serverInfo"]["name"], json!("codetopo"));
    assert!(
        hello["result"]["capabilities"]["tools"].is_object(),
        "{hello}"
    );

    let catalogue = client.request(2, "tools/list", json!({}));
    assert_eq!(
        catalogue["result"]["tools"]
            .as_array()
            .expect("an array")
            .len(),
        8
    );

    let indexed = client.request(
        3,
        "tools/call",
        json!({
            "name": "codetopo_index",
            "arguments": {"repo_path": fixture_repo(), "db_path": db},
        }),
    );
    assert_ne!(indexed["result"]["isError"], json!(true), "{indexed}");

    let stats = client.request(
        4,
        "tools/call",
        json!({"name": "codetopo_stats", "arguments": {"db_path": db}}),
    );
    assert!(
        stats["result"]["nodes"].as_u64().expect("a count") > 0,
        "{stats}"
    );

    // The fixture is a single chain with no cycles, so the call graph has
    // exactly one answer here: three steps from the top to the leaf.
    let chain = client.request(
        5,
        "tools/call",
        json!({
            "name": "codetopo_path",
            "arguments": {
                "db_path": db,
                "from": id_of(Path::new(db), "handle"),
                "to": id_of(Path::new(db), "send"),
            },
        }),
    );
    let steps = chain["result"]["path"].as_array().expect("an array");
    assert_eq!(
        steps.len(),
        3,
        "expected handle->query->connect->send: {chain}"
    );

    let verified = client.request(
        6,
        "tools/call",
        json!({"name": "codetopo_verify", "arguments": {"db_path": db}}),
    );
    assert_eq!(verified["result"]["chain_valid"], json!(true), "{verified}");

    // The export is the last step of the read-only recipe: it must carry the
    // whole graph and be bound to the audit head the previous call just
    // verified, or a consumer cannot tell which chain it came from.
    let snapshot = client.request(
        7,
        "tools/call",
        json!({"name": "codetopo_snapshot", "arguments": {"db_path": db}}),
    );
    assert_ne!(snapshot["result"]["isError"], json!(true), "{snapshot}");
    assert_eq!(
        snapshot["result"]["audit_head"], verified["result"]["head"],
        "the snapshot must bind the verified head: {snapshot}"
    );
    assert_eq!(
        snapshot["result"]["nodes"]
            .as_array()
            .expect("an array")
            .len() as u64,
        stats["result"]["nodes"].as_u64().expect("a count"),
        "the snapshot must carry every node stats counted: {snapshot}"
    );

    let (code, trailing) = client.close();
    assert_eq!(code, Some(0));
    assert_eq!(trailing, "");
}

#[test]
fn a_notification_is_answered_with_silence() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = dir.path().join("notify.db");

    let mut client = Client::start();
    client.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    // The request that follows stands in for the client's next question: if the
    // notification had been answered, that answer would arrive here instead and
    // the id assertion in `request` would fail.
    let answer = client.request(1, "ping", json!({}));
    assert_eq!(answer["result"], json!({}));

    let indexed = client.request(
        2,
        "tools/call",
        json!({
            "name": "codetopo_index",
            "arguments": {"repo_path": fixture_repo(), "db_path": db},
        }),
    );
    assert_ne!(indexed["result"]["isError"], json!(true), "{indexed}");

    let (code, trailing) = client.close();
    assert_eq!(code, Some(0));
    assert_eq!(trailing, "", "the notification drew a reply of its own");
}

#[test]
fn a_malformed_frame_is_answered_rather_than_dropped() {
    let mut client = Client::start();
    // Not JSON at all. A client that sent this by mistake is waiting for
    // something, and silence would leave it waiting forever.
    client
        .stdin
        .write_all(b"{not json\n")
        .expect("the pipe is writable");
    client.stdin.flush().expect("the pipe is writable");

    let response = client.recv();
    assert_eq!(response["id"], Value::Null, "{response}");
    assert_eq!(response["error"]["code"], json!(-32700), "{response}");

    // The server is still serving after a bad frame.
    let pong = client.request(1, "ping", json!({}));
    assert_eq!(pong["result"], json!({}));

    let (code, _) = client.close();
    assert_eq!(code, Some(0));
}
