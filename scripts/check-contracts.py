#!/usr/bin/env python3
# Copyright (C) 2026 Nrupal Akolkar
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.
"""Contract drift gate.

The files under schemas/ are contracts other software builds against.
They drift the moment an implementation changes and nobody regenerates
them — which is exactly how schemas/mcp-tools.json came to record a
server version of 0.1.0 while the binary reported 0.2.3. This gate
fails CI when the contracts and the running code disagree:

1. MCP — schemas/mcp-tools.json must equal what the built
   `codetopo-mcp` binary actually serves over stdio (tools array,
   serverInfo, protocolVersion), byte-for-byte after JSON parsing.
2. HTTP — every route registered in the server must appear in
   schemas/openapi.yaml's paths, and every documented path must be a
   registered route.
3. ACP — the built `codetopo-acp` binary's initialize response must
   advertise this crate's real version (from its Cargo.toml) and the
   ACP spec's integer protocolVersion (1).

Usage: python3 scripts/check-contracts.py [mcp_binary] [acp_binary]
Defaults: target/debug/codetopo-mcp, target/debug/codetopo-acp.
Exit code 0 = no drift; 1 = drift (differences printed).
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
failures = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'} {name}" + (f" — {detail}" if detail and not ok else ""))
    if not ok:
        failures.append(name)


def run_stdio(binary, frames):
    proc = subprocess.run(
        [str(binary)],
        input="\n".join(frames) + "\n",
        capture_output=True,
        text=True,
        timeout=60,
    )
    lines = [ln for ln in proc.stdout.splitlines() if ln.strip()]
    return [json.loads(ln) for ln in lines]


# ---------------------------------------------------------------- MCP
def check_mcp(binary):
    frames = [
        json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        json.dumps({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    ]
    responses = run_stdio(binary, frames)
    init = next(r for r in responses if r.get("id") == 1)["result"]
    live_tools = next(r for r in responses if r.get("id") == 2)["result"]["tools"]

    contract = json.loads((ROOT / "schemas/mcp-tools.json").read_text())
    check("mcp: tools array matches the live binary",
          contract["tools"] == live_tools,
          "regenerate schemas/mcp-tools.json (recipe in its $comment)")
    check("mcp: recorded serverInfo matches the live binary",
          contract["source"]["serverInfo"] == init["serverInfo"],
          f"file says {contract['source']['serverInfo']}, binary says {init['serverInfo']}")
    check("mcp: recorded protocolVersion matches the live binary",
          contract["source"]["protocolVersion"] == init["protocolVersion"],
          f"file says {contract['source']['protocolVersion']}, binary says {init['protocolVersion']}")


# --------------------------------------------------------------- HTTP
def check_openapi():
    handlers = (ROOT / "crates/codetopo-server/src/handlers.rs").read_text()
    routes = set()
    for raw in re.findall(r'\.route\("([^"]+)"', handlers):
        routes.add(re.sub(r":([A-Za-z_]+)", r"{\1}", raw))
    yaml = (ROOT / "schemas/openapi.yaml").read_text()
    documented = set(re.findall(r"^  (/[^ :]+):$", yaml, flags=re.MULTILINE))
    check("http: every registered route is documented",
          routes <= documented,
          f"undocumented: {sorted(routes - documented)}")
    check("http: every documented path is a registered route",
          documented <= routes,
          f"phantom paths: {sorted(documented - routes)}")


# ---------------------------------------------------------------- ACP
def check_acp(binary):
    frames = [json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})]
    responses = run_stdio(binary, frames)
    result = responses[0]["result"]
    cargo = (ROOT / "crates/codetopo-acp/Cargo.toml").read_text()
    version = re.search(r'^version\s*=\s*"([^"]+)"', cargo, flags=re.MULTILINE).group(1)
    check("acp: agentInfo.version is the crate version",
          result["agentInfo"]["version"] == version,
          f"binary says {result['agentInfo']['version']}, Cargo.toml says {version}")
    check("acp: protocolVersion is the spec integer 1",
          result["protocolVersion"] == 1,
          f"binary says {result['protocolVersion']!r}")


def main():
    mcp_bin = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target/debug/codetopo-mcp"
    acp_bin = Path(sys.argv[2]) if len(sys.argv) > 2 else ROOT / "target/debug/codetopo-acp"
    for label, path in [("codetopo-mcp", mcp_bin), ("codetopo-acp", acp_bin)]:
        if not path.exists():
            print(f"FAIL {label} binary not found at {path} — build it first")
            return 1
    check_mcp(mcp_bin)
    check_openapi()
    check_acp(acp_bin)
    if failures:
        print(f"\nDRIFT: {len(failures)} check(s) failed")
        return 1
    print("\nno drift: contracts match the running code")
    return 0


if __name__ == "__main__":
    sys.exit(main())
