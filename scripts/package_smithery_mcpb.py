# Copyright (C) 2026 Nrupal Akolkar
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.
"""Package codetopo-mcp as a Smithery MCPB bundle.

Smithery distributes local stdio servers as a single .mcpb file. The
MCPB manifest has an OS field but no CPU-architecture field, so the
bundle carries the codetopo-mcp binary for every platform and a
zero-dependency Node launcher (server/launcher.js) picks the native
one at startup — the pattern compiled MCP servers ship on Smithery.

Inputs (all from artifacts this repo already publishes):
  - the codetopo-mcp release archives of a GitHub release tag
    (built by .github/workflows/release-binaries.yml), each verified
    against its published .sha256 sidecar before it enters the bundle;
  - the tool catalogue in schemas/mcp-tools.json (drift-gated against
    the live server), embedded in the manifest with the inputSchema
    Smithery requires on every tool.

Usage:
    python3 scripts/package_smithery_mcpb.py [release-tag]
        [--out dist/codetopo-<mcp-version>.mcpb]

The bundle version is the codetopo-mcp crate version, read from its
manifest — never typed twice.
"""

import argparse
import hashlib
import io
import json
import re
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OWNER_REPO = "nrupala/codetopo"

# Bundle bin directory key -> (release platform name, archive ext, exe name)
PLATFORMS = {
    "darwin-arm64": ("darwin-aarch64", "tar.gz", "codetopo-mcp"),
    "darwin-x64": ("darwin-x86_64", "tar.gz", "codetopo-mcp"),
    "linux-arm64": ("linux-aarch64", "tar.gz", "codetopo-mcp"),
    "linux-x64": ("linux-x86_64", "tar.gz", "codetopo-mcp"),
    "win32-x64": ("windows-x86_64", "zip", "codetopo-mcp.exe"),
}

LAUNCHER_JS = r"""#!/usr/bin/env node
// codetopo-mcp launcher: the MCPB manifest has no CPU-architecture
// field, so the bundle carries one native binary per platform and
// this zero-dependency script starts the right one, inheriting
// stdio so the MCP wire passes through untouched.
'use strict';
const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

const key = `${process.platform}-${process.arch}`;
const exe = process.platform === 'win32' ? 'codetopo-mcp.exe' : 'codetopo-mcp';
const bin = path.join(__dirname, 'bin', key, exe);

if (!fs.existsSync(bin)) {
  process.stderr.write(`codetopo: no bundled binary for this platform (${key}).\n`);
  process.stderr.write('Supported: darwin-arm64, darwin-x64, linux-arm64, linux-x64, win32-x64.\n');
  process.stderr.write('Alternatively install from crates.io: cargo install codetopo-mcp\n');
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit' });
child.on('error', (err) => {
  process.stderr.write(`codetopo: failed to start ${bin}: ${err.message}\n`);
  process.exit(1);
});
child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code === null ? 1 : code);
  }
});
"""


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "codetopo-mcpb-packager"})
    with urllib.request.urlopen(req, timeout=120) as resp:
        return resp.read()


def mcp_version() -> str:
    text = (REPO / "crates" / "codetopo-mcp" / "Cargo.toml").read_text()
    return re.search(r'^version\s*=\s*"([^"]+)"', text, re.M).group(1)


def extract_binary(archive: bytes, ext: str, exe: str) -> bytes:
    if ext == "tar.gz":
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tf:
            member = next(m for m in tf.getmembers() if Path(m.name).name == exe)
            return tf.extractfile(member).read()
    with zipfile.ZipFile(io.BytesIO(archive)) as zf:
        name = next(n for n in zf.namelist() if Path(n).name == exe)
        return zf.read(name)


def build_manifest(version: str) -> dict:
    catalogue = json.loads((REPO / "schemas" / "mcp-tools.json").read_text())
    tools = [
        {
            "name": t["name"],
            "description": t["description"],
            "inputSchema": t["inputSchema"],
        }
        for t in catalogue["tools"]
    ]
    return {
        "manifest_version": "0.3",
        "name": "codetopo",
        "version": version,
        "description": "Domain-blind code graphs for AI agents: symbols, calls, blast radius, proof-checked snapshots. Indexes a repository into a deterministic graph and answers structural questions over MCP — read-only, nothing executed.",
        "author": {"name": "Nrupal Akolkar"},
        "repository": {"type": "git", "url": "https://github.com/nrupala/codetopo"},
        "homepage": "https://aimlds.org/codetopo",
        "license": "AGPL-3.0-or-later",
        "keywords": ["code-graph", "static-analysis", "mcp", "symbols", "call-graph"],
        "server": {
            "type": "node",
            "entry_point": "server/launcher.js",
            "mcp_config": {
                "command": "node",
                "args": ["${__dirname}/server/launcher.js"],
                "env": {},
            },
        },
        "tools": tools,
        "compatibility": {"platforms": ["darwin", "win32", "linux"]},
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("tag", nargs="?", default="v0.3.4",
                    help="GitHub release tag holding the codetopo-mcp archives")
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    version = mcp_version()
    out = Path(args.out) if args.out else REPO / "dist" / f"codetopo-{version}.mcpb"
    out.parent.mkdir(parents=True, exist_ok=True)
    base = f"https://github.com/{OWNER_REPO}/releases/download/{args.tag}"

    binaries: dict[str, bytes] = {}
    for key, (platform, ext, exe) in PLATFORMS.items():
        fname = f"codetopo-mcp-{platform}.{ext}"
        print(f"{key}: downloading {fname}")
        archive = fetch(f"{base}/{fname}")
        sidecar = fetch(f"{base}/{fname}.sha256").decode().split()[0]
        digest = hashlib.sha256(archive).hexdigest()
        if digest != sidecar:
            print(f"  SHA-256 MISMATCH (published {sidecar}, got {digest}) — aborting")
            return 1
        binaries[key] = extract_binary(archive, ext, exe)
        print(f"  sha256 verified; binary {len(binaries[key])} bytes")

    manifest = build_manifest(version)
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("manifest.json", json.dumps(manifest, indent=2))
        info = zipfile.ZipInfo("server/launcher.js")
        info.external_attr = 0o644 << 16
        zf.writestr(info, LAUNCHER_JS)
        for key, data in binaries.items():
            exe = "codetopo-mcp.exe" if key.startswith("win32") else "codetopo-mcp"
            info = zipfile.ZipInfo(f"server/bin/{key}/{exe}")
            info.external_attr = 0o755 << 16
            zf.writestr(info, data)
    size = out.stat().st_size
    print(f"bundle: {out} ({size} bytes, sha256 {hashlib.sha256(out.read_bytes()).hexdigest()})")
    if size > 25 * 1024 * 1024:
        print("WARNING: exceeds Smithery's 25 MB upload limit")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
