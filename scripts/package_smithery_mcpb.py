# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.
"""Package codetopo-mcp as a Smithery MCPB bundle.

Smithery distributes local stdio servers as a single .mcpb file with
a 25 MB upload limit. Five stripped codetopo-mcp binaries are ~37 MB,
so the bundle does not vendor them: it carries a zero-dependency Node
launcher (server/launcher.js) plus a pin file (server/pins.json)
with, per platform, the release archive URL and the SHA-256 this
script verified against the release's published sidecar. On first
run the launcher downloads its platform's archive from the pinned
GitHub release, refuses to proceed unless the bytes match the pin,
caches the binary under the user's cache directory, and execs it
with stdio inherited — the same fetch-at-launch pattern other
Smithery bundles use (e.g. uv-launched PyPI bundles). Every later
run is fully offline against the cache.

Inputs (all from artifacts this repo already publishes):
  - the codetopo-mcp release archives of a GitHub release tag
    (built by .github/workflows/release-binaries.yml), each verified
    against its published .sha256 sidecar at pack time;
  - the tool catalogue in schemas/mcp-tools.json (drift-gated
    against the live server), embedded in the manifest with the
    inputSchema Smithery requires on every tool.

Usage:
    python3 scripts/package_smithery_mcpb.py [release-tag]
        [--out dist/codetopo-<mcp-version>.mcpb]

The bundle version is the codetopo-mcp crate version, read from its
manifest — never typed twice.
"""

import argparse
import hashlib
import json
import re
import sys
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
// codetopo-mcp launcher.
//
// The bundle does not vendor the server binaries (five platforms at
// ~8 MB each exceed Smithery's 25 MB bundle limit). Instead, on
// first run this zero-dependency script downloads the pinned
// platform archive from the codetopo GitHub release, verifies its
// SHA-256 against the pin baked into pins.json at pack time, caches
// the extracted binary, and starts it with stdio inherited so the
// MCP wire passes through untouched. Later runs are offline.
'use strict';
const { spawn, spawnSync } = require('child_process');
const crypto = require('crypto');
const fs = require('fs');
const https = require('https');
const os = require('os');
const path = require('path');

const pins = require('./pins.json');
const key = `${process.platform}-${process.arch}`;
const pin = pins.platforms[key];

function fail(msg) {
  process.stderr.write(`codetopo: ${msg}\n`);
  process.exit(1);
}

if (!pin) {
  fail(`no codetopo-mcp build for this platform (${key}). ` +
    'Supported: darwin-arm64, darwin-x64, linux-arm64, linux-x64, win32-x64. ' +
    'Alternatively install from crates.io: cargo install codetopo-mcp');
}

function cacheDir() {
  const base =
    process.env.CODETOPO_MCPB_CACHE ||
    (process.platform === 'darwin'
      ? path.join(os.homedir(), 'Library', 'Caches', 'codetopo-mcpb')
      : process.platform === 'win32'
        ? path.join(process.env.LOCALAPPDATA || os.tmpdir(), 'codetopo-mcpb')
        : path.join(process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache'), 'codetopo-mcpb'));
  return path.join(base, pins.version, key);
}

function download(url, dest) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { 'User-Agent': 'codetopo-mcpb-launcher' } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume();
        download(res.headers.location, dest).then(resolve, reject);
        return;
      }
      if (res.statusCode !== 200) {
        res.resume();
        reject(new Error(`download failed: HTTP ${res.statusCode} for ${url}`));
        return;
      }
      const out = fs.createWriteStream(dest);
      res.pipe(out);
      out.on('finish', () => out.close(resolve));
      out.on('error', reject);
    }).on('error', reject);
  });
}

function sha256(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function extract(archive, dir) {
  if (archive.endsWith('.zip')) {
    const r = spawnSync('powershell', ['-NoProfile', '-Command',
      `Expand-Archive -Force -Path '${archive}' -DestinationPath '${dir}'`],
      { stdio: 'inherit' });
    if (r.status !== 0) throw new Error('Expand-Archive failed');
  } else {
    const r = spawnSync('tar', ['--no-same-owner', '-xzf', archive, '-C', dir], { stdio: 'inherit' });
    if (r.status !== 0) throw new Error('tar extraction failed');
  }
}

async function ensureBinary() {
  const dir = cacheDir();
  const exePath = path.join(dir, pin.exe);
  if (fs.existsSync(exePath)) return exePath;
  fs.mkdirSync(dir, { recursive: true });
  const archiveName = pin.url.split('/').pop();
  const archivePath = path.join(dir, archiveName);
  process.stderr.write(`codetopo: first run — downloading codetopo-mcp ${pins.version} for ${key}...\n`);
  await download(pin.url, archivePath);
  const digest = sha256(archivePath);
  if (digest !== pin.sha256) {
    fs.rmSync(archivePath, { force: true });
    fail(`downloaded archive failed its SHA-256 check (expected ${pin.sha256}, got ${digest}). Refusing to run it.`);
  }
  extract(archivePath, dir);
  fs.rmSync(archivePath, { force: true });
  if (!fs.existsSync(exePath)) fail(`archive did not contain ${pin.exe}.`);
  if (process.platform !== 'win32') fs.chmodSync(exePath, 0o755);
  return exePath;
}

ensureBinary().then((bin) => {
  const child = spawn(bin, process.argv.slice(2), { stdio: 'inherit' });
  child.on('error', (err) => fail(`failed to start the server binary: ${err.message}`));
  child.on('exit', (code, signal) => {
    if (signal) process.kill(process.pid, signal);
    else process.exit(code === null ? 1 : code);
  });
}).catch((err) => fail(err.message));
"""


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "codetopo-mcpb-packager"})
    with urllib.request.urlopen(req, timeout=120) as resp:
        return resp.read()


def mcp_version() -> str:
    text = (REPO / "crates" / "codetopo-mcp" / "Cargo.toml").read_text()
    return re.search(r'^version\s*=\s*"([^"]+)"', text, re.M).group(1)


def build_manifest(version: str) -> dict:
    catalogue = json.loads((REPO / "schemas" / "mcp-tools.json").read_text())
    tools = []
    for t in catalogue["tools"]:
        tool = {
            "name": t["name"],
            "description": t["description"],
            "inputSchema": t["inputSchema"],
        }
        # Directory conformance fields ride along when the catalogue
        # carries them (codetopo-mcp >= 0.3.5).
        if "title" in t:
            tool["title"] = t["title"]
        if "annotations" in t:
            tool["annotations"] = t["annotations"]
        tools.append(tool)
    return {
        "manifest_version": "0.3",
        "name": "codetopo",
        "version": version,
        "description": "Domain-blind code graphs for AI agents: symbols, calls, blast radius, proof-checked snapshots. Indexes a repository into a deterministic graph and answers structural questions over MCP — read-only, nothing executed.",
        "author": {"name": "Nrupal Akolkar"},
        "repository": {"type": "git", "url": "https://github.com/nrupala/codetopo"},
        "homepage": "https://aimlds.org/codetopo",
        "icon": "icon.png",
        "privacy_policies": ["https://github.com/nrupala/codetopo#privacy-policy"],
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
    ap.add_argument("tag", nargs="?", default=None,
                    help="GitHub release tag holding the codetopo-mcp archives "
                         "(default: v<crate version>)")
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    version = mcp_version()
    tag = args.tag or f"v{version}"
    out = Path(args.out) if args.out else REPO / "dist" / f"codetopo-{version}.mcpb"
    out.parent.mkdir(parents=True, exist_ok=True)
    base = f"https://github.com/{OWNER_REPO}/releases/download/{tag}"

    pins = {"version": version, "platforms": {}}
    for key, (platform, ext, exe) in PLATFORMS.items():
        fname = f"codetopo-mcp-{platform}.{ext}"
        url = f"{base}/{fname}"
        print(f"{key}: verifying {fname} against its published sidecar")
        archive = fetch(url)
        sidecar = fetch(f"{url}.sha256").decode().split()[0]
        digest = hashlib.sha256(archive).hexdigest()
        if digest != sidecar:
            print(f"  SHA-256 MISMATCH (published {sidecar}, got {digest}) — aborting")
            return 1
        pins["platforms"][key] = {"url": url, "sha256": digest, "exe": exe}
        print(f"  pinned sha256 {digest[:16]}...")

    manifest = build_manifest(version)
    icon_path = REPO / "assets" / "icon-512.png"
    if not icon_path.exists():
        print(f"missing bundle icon: {icon_path} — aborting")
        return 1
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("manifest.json", json.dumps(manifest, indent=2))
        zf.write(icon_path, "icon.png")
        zf.writestr("server/launcher.js", LAUNCHER_JS)
        zf.writestr("server/pins.json", json.dumps(pins, indent=2))
    size = out.stat().st_size
    print(f"bundle: {out} ({size} bytes, sha256 {hashlib.sha256(out.read_bytes()).hexdigest()})")
    if size > 25 * 1024 * 1024:
        print("WARNING: exceeds Smithery's 25 MB upload limit")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
