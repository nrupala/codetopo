#!/usr/bin/env python3
"""Generate the deterministic codetopo scale-gate fixture.

Usage: gen_scale_fixture.py <output-dir> [file-count]

Layout: <count> source files in 100-file directories (dir0000,
dir0001, ...), 60% Rust / 40% TypeScript. Every file defines two
functions, `stepNNNNN` and `helpNNNNN`:

- `step{i}` calls `step{i+1}` — one call chain spanning the whole
  fixture (depth == file count: the recursion stress for cycle
  detection and the long-pole query for descendants/ancestors).
- Every 997 files, `step{i}` also calls `step{i-997}` — a back edge
  that closes a large cycle so SCC detection has real work.
- `step{i}` calls `help{i}` (intra-file), and every file imports the
  next file's module.

Deterministic: same count in, byte-identical tree out.
"""
import os
import sys


def rust_file(i: int, n: int) -> str:
    lines = []
    if i + 1 < n:
        lines.append(f"use crate::dir{(i + 1) // 100:04d}::file{i + 1:05d}::step{i + 1:05d};")
    lines.append(f"pub fn step{i:05d}() {{")
    if i + 1 < n:
        lines.append(f"    step{i + 1:05d}();")
    if i >= 997 and i % 997 == 0:
        lines.append(f"    step{i - 997:05d}();")
    lines.append(f"    help{i:05d}();")
    lines.append("}")
    lines.append(f"pub fn help{i:05d}() {{}}")
    return "\n".join(lines) + "\n"


def ts_file(i: int, n: int) -> str:
    lines = []
    if i + 1 < n:
        lines.append(
            f"import {{ step{i + 1:05d} }} from '../dir{(i + 1) // 100:04d}/file{i + 1:05d}';"
        )
    lines.append(f"export function step{i:05d}(): void {{")
    if i + 1 < n:
        lines.append(f"  step{i + 1:05d}();")
    if i >= 997 and i % 997 == 0:
        lines.append(f"  step{i - 997:05d}();")
    lines.append(f"  help{i:05d}();")
    lines.append("}")
    lines.append(f"export function help{i:05d}(): void {{}}")
    return "\n".join(lines) + "\n"


def main() -> None:
    out = sys.argv[1]
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 10_000
    for i in range(n):
        d = os.path.join(out, f"dir{i // 100:04d}")
        os.makedirs(d, exist_ok=True)
        rust = i % 5 < 3
        name = f"file{i:05d}.rs" if rust else f"file{i:05d}.ts"
        text = rust_file(i, n) if rust else ts_file(i, n)
        with open(os.path.join(d, name), "w") as fh:
            fh.write(text)
    print(f"wrote {n} files under {out}")


if __name__ == "__main__":
    main()
