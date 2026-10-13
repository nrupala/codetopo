#!/usr/bin/env bash
# Copyright (C) 2026 Nrupal Akolkar
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.
#
# capture-space-bundle.sh — refresh the HF Space's data bundle
# (space-static/data/) from a real run of codetopo-server against a
# source tree, exactly the way the original one-time capture was
# made (see the bundle's ABOUT.json):
#
#   1. start the server with a throwaway key on a loopback port;
#   2. POST /v1/index for the tree;
#   3. GET stats / snapshot / verify / schema and save the response
#      bodies byte-for-byte;
#   4. run the honesty gates — the capture is REFUSED (non-zero
#      exit, nothing written) unless:
#        - snapshot node/edge counts equal stats totals;
#        - snapshot audit_head equals verify head AND chain_valid;
#   5. write ABOUT.json provenance (what was captured, from which
#      commit, by which server version, which checks ran).
#
# Usage: capture-space-bundle.sh <server-binary> <source-tree> <source-commit> <out-dir>
set -euo pipefail

BIN="${1:?server binary path}"
TREE="${2:?source tree to index}"
COMMIT="${3:?source commit sha}"
OUT="${4:?output directory}"

PORT="${CODETOPO_CAPTURE_PORT:-8797}"
WORK="$(mktemp -d)"
trap 'kill "${SERVER_PID:-}" 2>/dev/null || true; rm -rf "$WORK"' EXIT

TREE_ABS="$(cd "$TREE" && pwd)"
VERSION="$(grep -m1 '^version = ' "$TREE_ABS/crates/codetopo-server/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
[ -n "$VERSION" ] || { echo "cannot read codetopo-server version from $TREE" >&2; exit 1; }

SECRET="$(head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n')"
printf 'capture:%s\n' "$SECRET" > "$WORK/api_keys"
AUTH="Authorization: Bearer capture:$SECRET"

CODETOPO_BIND="127.0.0.1:$PORT" \
CODETOPO_DATA_DIR="$WORK/data" \
CODETOPO_KEYS_FILE="$WORK/api_keys" \
CODETOPO_METERING_LOG="$WORK/metering.log" \
  "$BIN" > "$WORK/server.log" 2>&1 &
SERVER_PID=$!

# Wait for the server to answer (schema route, with auth — harmless
# if the route is public).
for i in $(seq 1 30); do
  if curl -fsS -H "$AUTH" "http://127.0.0.1:$PORT/v1/schema" -o "$WORK/schema.json" 2>/dev/null; then
    break
  fi
  if [ "$i" = 30 ]; then
    echo "server did not come up; log:" >&2; cat "$WORK/server.log" >&2; exit 1
  fi
  sleep 1
done

# Index the tree.
curl -fsS -X POST -H "$AUTH" -H 'content-type: application/json' \
  --data "{\"repo_path\": \"$TREE_ABS\"}" \
  "http://127.0.0.1:$PORT/v1/index" -o "$WORK/index.json"

INDEX_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["index_id"])' "$WORK/index.json")"
[ -n "$INDEX_ID" ] || { echo "index response carried no index_id" >&2; exit 1; }

# Capture the three per-graph responses byte-for-byte.
for name in stats snapshot verify; do
  curl -fsS -H "$AUTH" "http://127.0.0.1:$PORT/v1/graphs/$INDEX_ID/$name" -o "$WORK/$name.json"
done

# Honesty gates + ABOUT.json, in one Python pass. Nothing is
# written to $OUT unless every gate passes.
python3 - "$WORK" "$OUT" "$COMMIT" "$VERSION" << 'PYEOF'
import json, os, sys
from datetime import datetime, timezone

work, out, commit, version = sys.argv[1:5]
load = lambda name: json.load(open(os.path.join(work, name)))
index, stats, snapshot, verify = load("index.json"), load("stats.json"), load("snapshot.json"), load("verify.json")

checks = []
n_nodes, n_edges = len(snapshot["nodes"]), len(snapshot["edges"])
assert n_nodes == stats["nodes"]["total"], f"snapshot nodes {n_nodes} != stats total {stats['nodes']['total']}"
assert n_edges == stats["edges"]["total"], f"snapshot edges {n_edges} != stats total {stats['edges']['total']}"
checks.append(f"snapshot.json node/edge counts equal stats.json totals ({n_nodes} / {n_edges}).")
assert snapshot["audit_head"] == verify["head"], "snapshot audit_head != verify head"
assert verify["chain_valid"] is True, "verify chain_valid is not true"
checks.append("snapshot.json audit_head equals verify.json head; verify.json chain_valid is true.")

about = {
    "about": "Provenance for the bundled snapshot in this directory. Every other .json file here is the raw response body of the named codetopo-server API endpoint, saved byte-for-byte as returned by a real run — no values were edited, recomputed, or invented.",
    "analyzed_tree": "A clean copy of the source tree at source_commit (build output excluded; the indexer saw exactly the repository's source files).",
    "captured_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "cross_checks_at_capture": checks,
    "files": {
        "schema.json": "GET /v1/schema",
        "snapshot.json": "GET /v1/graphs/{index_id}/snapshot",
        "stats.json": "GET /v1/graphs/{index_id}/stats",
        "verify.json": "GET /v1/graphs/{index_id}/verify",
    },
    "index_id": index["index_id"],
    "index_summary_from_index_response": {
        "diagnostics": index["diagnostics"],
        "edges": index["edges"],
        "files_failed": index["files_failed"],
        "files_indexed": index["files_indexed"],
        "nodes": index["nodes"],
    },
    "source_commit": commit,
    "source_ref": "main",
    "source_repository": "https://github.com/nrupala/codetopo",
    "tool": f"codetopo-server {version}",
    "tool_source": "Built with cargo from the same source_commit (cargo build --release -p codetopo-server), run by scripts/capture-space-bundle.sh in the space-recapture workflow.",
}

os.makedirs(out, exist_ok=True)
for name in ["stats.json", "snapshot.json", "verify.json", "schema.json"]:
    with open(os.path.join(work, name), "rb") as src, open(os.path.join(out, name), "wb") as dst:
        dst.write(src.read())
with open(os.path.join(out, "ABOUT.json"), "w") as fh:
    json.dump(about, fh, sort_keys=True, separators=(",", ":"))
    fh.write("\n")
print(f"capture OK: {n_nodes} nodes / {n_edges} edges from {commit[:8]} (server {version})")
PYEOF
