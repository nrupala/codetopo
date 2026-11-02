// Copyright (C) 2026 2351485 Alberta Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! F4 (MCP leg) for the Python extractor: the known-answer fixture
//! package indexed and queried through the real `codetopo-mcp`
//! binary over stdio, with the answers asserted identical to the
//! CLI's view of the same database (`load_graph_from_db` +
//! `rank_value`), in the pattern of `tests/mcp_stdio.rs`. Full-
//! directory counts for the fixture (4 files, 29 nodes, 36 edges)
//! are derived in the fixture's README.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

fn fixture_py() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../codetopo-extract/tests/fixture-py")
}

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
        assert_eq!(response["id"], json!(id), "a response must answer its own id");
        response
    }

    fn call(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        let resp = self.request(id, "tools/call", json!({"name": name, "arguments": arguments}));
        assert_ne!(resp["result"]["isError"], json!(true), "{name} failed: {resp}");
        resp["result"]["structuredContent"].clone()
    }

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

#[test]
fn py_fixture_parity_over_stdio() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = dir.path().join("py.db");
    let db = db.to_str().expect("a UTF-8 temp path");

    let mut client = Client::start();
    let hello = client.request(1, "initialize", json!({}));
    assert_eq!(hello["result"]["serverInfo"]["name"], json!("codetopo"));
    let catalogue = client.request(2, "tools/list", json!({}));
    assert_eq!(
        catalogue["result"]["tools"]
            .as_array()
            .expect("an array")
            .len(),
        20,
        "the tool catalogue carries the twenty tools of the analytics build"
    );

    // Index the fixture through the tool itself.
    let report = client.call(
        3,
        "codetopo_index",
        json!({"repo_path": fixture_py(), "db_path": db}),
    );
    assert_eq!(report["files_indexed"], json!(4));
    assert_eq!(report["files_failed"], json!(0));
    assert_eq!(report["nodes"], json!(29));
    assert_eq!(report["edges"], json!(36));
    assert_eq!(report["diagnostics"], json!(2));

    // The CLI's view of the same database is the comparison target.
    let graph = codetopo_cli::load_graph_from_db(Path::new(db)).expect("the index loads");

    // Stats parity.
    let stats = client.call(4, "codetopo_stats", json!({"db_path": db}));
    assert_eq!(stats["nodes"], json!(graph.node_count()));
    assert_eq!(stats["edges"], json!(graph.edge_count()));
    assert_eq!(stats["node_kinds"]["method"], json!(7));
    assert_eq!(stats["node_kinds"]["interface"], json!(1));

    // Symbol dossier parity for a Python class.
    let dossier = client.call(
        5,
        "codetopo_get_symbol",
        json!({"db_path": db, "node": "quarry::shapes::Circle"}),
    );
    let circle = graph
        .nodes()
        .find(|n| n.id == "quarry::shapes::Circle")
        .expect("Circle indexed");
    assert_eq!(dossier["node"]["id"], json!(circle.id));
    assert_eq!(dossier["node"]["label"], json!(circle.label));
    assert_eq!(dossier["node"]["kind"], json!("class"));

    // Rank parity for the fan-in metric (the shared rank_value).
    let rank = client.call(
        6,
        "codetopo_rank",
        json!({"db_path": db, "by": "fan_in", "limit": 20}),
    );
    assert_eq!(
        rank,
        codetopo_cli::rank_value(&graph, codetopo_core::RankMetric::FanIn, 20),
        "MCP rank == CLI rank for fan_in"
    );

    // Diagnostics parity: the fixture's known pair.
    let diags = client.call(7, "codetopo_diagnostics", json!({"db_path": db}));
    assert_eq!(
        diags,
        codetopo_cli::diagnostics_value(Path::new(db)).expect("cli diagnostics value"),
        "MCP diagnostics == CLI diagnostics"
    );

    let (code, trailing) = client.close();
    assert_eq!(trailing, "", "stdout carried bytes after the final response");
    assert_eq!(code, Some(0), "the server exits cleanly at EOF");
}
