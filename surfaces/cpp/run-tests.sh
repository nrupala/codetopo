#!/bin/sh
# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
#
# Build and run the C/C++ face's proofs:
#   1. the shipped C header compiles standalone (C11, -Werror);
#   2. the C consumer smoke program (tests/consumer.c) — the
#      header + cdylib used exactly as an external consumer would;
#   3. the C++ conformance battery (tests/conformance.cpp) — the
#      F1–F5 known-answer fixtures through the C++ wrapper;
#   4. the minimal C++ example (examples/minimal.cpp) — compiled,
#      and run against the quarry fixture.
#
# The cdylib is located via CODETOPO_C_LIB (the directory holding
# libcodetopo_c.so / .dylib). Default: the workspace target/release
# directory of this checkout — build it first with:
#   cargo build -p codetopo-c --release
# A debug build works too: CODETOPO_C_LIB=$PWD/target/debug.
#
# Usage: surfaces/cpp/run-tests.sh [work_dir]
#   work_dir defaults to a fresh temporary directory.

set -eu

FACE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$FACE_DIR/../.." && pwd)
C_HEADER_DIR="$REPO_ROOT/crates/codetopo-c/include"

LIB_DIR=${CODETOPO_C_LIB:-"$REPO_ROOT/target/release"}
case "$(uname -s)" in
    Darwin) LIB_FILE="$LIB_DIR/libcodetopo_c.dylib" ;;
    *)      LIB_FILE="$LIB_DIR/libcodetopo_c.so" ;;
esac
if [ ! -f "$LIB_FILE" ]; then
    echo "run-tests: libcodetopo_c not found at $LIB_FILE" >&2
    echo "run-tests: build it with 'cargo build -p codetopo-c --release'," >&2
    echo "run-tests: or point CODETOPO_C_LIB at the directory holding it." >&2
    exit 2
fi

WORK=${1:-$(mktemp -d)}
mkdir -p "$WORK"
echo "run-tests: lib dir: $LIB_DIR"
echo "run-tests: work dir: $WORK"

CC=${CC:-cc}
CXX=${CXX:-c++}

echo "run-tests: [1/4] shipped C header compiles standalone (C11, -Werror)"
# A translation unit that does nothing but include the header,
# compiled on its own: the header is self-contained or this fails.
printf '#include "codetopo.h"\nint main(void) { return 0; }\n' > "$WORK/header-only.c"
"$CC" -std=c11 -Wall -Wextra -Werror -I "$C_HEADER_DIR" \
    "$WORK/header-only.c" -o "$WORK/header-only"

echo "run-tests: [2/4] C consumer smoke program"
"$CC" -std=c11 -Wall -Wextra -Werror -I "$C_HEADER_DIR" \
    "$FACE_DIR/tests/consumer.c" -o "$WORK/consumer" \
    -L "$LIB_DIR" -lcodetopo_c -lm \
    -Wl,-rpath,"$LIB_DIR"
"$WORK/consumer" "$REPO_ROOT" "$WORK/consumer-work"

echo "run-tests: [3/4] C++ conformance battery (F1-F5 through the wrapper)"
"$CXX" -std=c++17 -Wall -Wextra -Werror \
    -I "$C_HEADER_DIR" -I "$FACE_DIR/include" \
    "$FACE_DIR/tests/conformance.cpp" -o "$WORK/conformance" \
    -L "$LIB_DIR" -lcodetopo_c -lm \
    -Wl,-rpath,"$LIB_DIR"
"$WORK/conformance" "$REPO_ROOT" "$WORK/conformance-work"

echo "run-tests: [4/4] minimal C++ example"
"$CXX" -std=c++17 -Wall -Wextra -Werror \
    -I "$C_HEADER_DIR" -I "$FACE_DIR/include" \
    "$FACE_DIR/examples/minimal.cpp" -o "$WORK/minimal" \
    -L "$LIB_DIR" -lcodetopo_c -lm \
    -Wl,-rpath,"$LIB_DIR"
# The example indexes the quarry package files (the F2 package —
# the fixture directory itself would add its README.md as a
# Markdown node; see tests/conformance.cpp's F2 note), in a
# directory named `quarry` so the example's default package name
# (the directory name, the other surfaces' rule) is "quarry" —
# with any other package name the fixture's absolute
# `from quarry.shapes import ...` resolves to an external stub
# and the graph gains a node.
mkdir -p "$WORK/quarry"
for f in __init__.py core.py shapes.py; do
    cp "$REPO_ROOT/crates/codetopo-extract/tests/fixture-py/$f" "$WORK/quarry/$f"
done
"$WORK/minimal" "$WORK/quarry" \
    "$WORK/minimal-quarry.db" > "$WORK/minimal.out"
grep -q '"nodes":26' "$WORK/minimal.out"
grep -q '"total":26' "$WORK/minimal.out"

echo "run-tests: ALL PASS"
