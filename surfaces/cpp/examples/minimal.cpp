// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
//
// Minimal codetopo C++ usage: index a repository, open the
// index, ask two questions, print the answers. Errors arrive as
// codetopo::Error exceptions carrying the C ABI's code + message.
//
//   minimal <repo_path> <db_path>
//
// Build (see surfaces/cpp/run-tests.sh for the full battery):
//   g++ -std=c++17 -I crates/codetopo-c/include -I surfaces/cpp/include
//       surfaces/cpp/examples/minimal.cpp
//       -L <dir holding libcodetopo_c> -lcodetopo_c
//       -Wl,-rpath,<dir holding libcodetopo_c> -o minimal

#include <cstdio>
#include <string>

#include "codetopo.hpp"

int main(int argc, char** argv) {
    if (argc != 3) {
        std::fprintf(stderr, "usage: minimal <repo_path> <db_path>\n");
        return 2;
    }
    try {
        std::printf("core %s, schema %s\n", codetopo::version().c_str(),
                    codetopo::schema_version().c_str());
        const std::string report = codetopo::index(argv[1], argv[2]);
        std::printf("index: %s\n", report.c_str());
        codetopo::Graph graph = codetopo::open_db(argv[2]);
        std::printf("stats: %s\n", graph.query("stats").c_str());
        std::printf("topo:  %s\n", graph.query("topo_sort").c_str());
        // `graph` releases its handle here, via its destructor.
    } catch (const codetopo::Error& e) {
        std::fprintf(stderr, "codetopo error [%s]: %s\n", e.code().c_str(),
                     e.message().c_str());
        return 1;
    }
    return 0;
}
