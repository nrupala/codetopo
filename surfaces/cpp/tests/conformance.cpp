// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
//
// C++ conformance program for the codetopo C++ face.
//
// Exercises the known-answer battery (F1–F5) through the C++
// wrapper (surfaces/cpp/include/codetopo.hpp) — RAII handles,
// std::string answers, codetopo::Error exceptions — never through
// the raw C ABI and never through Rust:
//
//   F1  oracle goldens (compass, ring, cycle-trap): snapshot load
//       via Graph::load_snapshot, rank answers at the core's own
//       tolerances (PageRank 1e-6, betweenness 1e-9, closeness
//       1e-9, eigenvector 1e-6 after sign alignment, Katz 1e-6),
//       eigenvector's absence on the compass DAG, topo-sort
//       groups/cycle flags exact.
//   F2  the quarry package indexed via codetopo::index:
//       26 nodes / 33 edges / 2 diagnostics over the package
//       files; the full fixture directory (README.md indexed as
//       Markdown) asserted separately at 29 / 36 / 2.
//   F3  the Aarona sibling tripwire (0.9.1 regression pin):
//       imports 7/7 and calls 7/7 internal (no external stubs),
//       topo-sort cyclic_groups == 2, and rank with a non-`by`
//       metric key throwing an Error naming the key.
//   F4  lifecycle: index -> snapshot -> load -> snapshot,
//       compared by GRAPH CONTENT (nodes/edges payloads —
//       snapshots embed wall-clock audit timestamps, so
//       byte-identity is the known flake, not the contract);
//       chains checked via the verify op.
//   F5  determinism: repeated queries byte-identical.
//
// Usage: conformance <repo_root> <work_dir>
//   <repo_root>  the codetopo repository checkout (fixtures live in it)
//   <work_dir>   a writable scratch directory (created if needed)
//
// JSON READING IN THIS FILE: the numeric checks below read the
// wrapper's JSON answers with small targeted-extraction helpers
// (find a key, parse the number after it). They are test-only
// scaffolding, documented as such — the same technique the C
// conformance program uses — and exist precisely because this
// face carries NO third-party JSON dependency for tests to lean
// on either. A production C++ consumer parses answers with its
// own JSON library.

#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#include "codetopo.hpp"

namespace fs = std::filesystem;

static int checks = 0;
static int failures = 0;

static void check(bool cond, const std::string& what) {
    ++checks;
    if (!cond) {
        ++failures;
        std::printf("FAIL: %s\n", what.c_str());
    }
}

static void check_close(const std::string& what, double got, double want, double tol) {
    ++checks;
    if (!(std::fabs(got - want) <= tol)) {
        ++failures;
        std::printf("FAIL: %s: got %.12g want %.12g (tol %g)\n", what.c_str(), got, want, tol);
    }
}

/* ---- targeted JSON extraction helpers (test-only, see header) ---- */

/* The integer following `"key":` at or after `anchor` ("" = start). */
static long json_int_after(const std::string& json, const std::string& anchor,
                           const std::string& key) {
    std::size_t pos = anchor.empty() ? 0 : json.find(anchor);
    if (pos == std::string::npos) return -1;
    pos = json.find(key, pos);
    if (pos == std::string::npos) return -1;
    pos = json.find(':', pos);
    if (pos == std::string::npos) return -1;
    return std::strtol(json.c_str() + pos + 1, nullptr, 10);
}

/* The rank value for one node id: find `"id":"<id>"`, then its
 * `"value":`. NaN when the entry or its value is absent — for the
 * float metrics an absent value is a failed check, never a zero. */
static double rank_value(const std::string& json, const std::string& id) {
    const std::string pattern = "\"id\":\"" + id + "\"";
    std::size_t pos = json.find(pattern);
    if (pos == std::string::npos) return NAN;
    pos = json.find("\"value\":", pos);
    if (pos == std::string::npos) return NAN;
    pos = json.find(':', pos);
    if (pos == std::string::npos) return NAN;
    return std::strtod(json.c_str() + pos + 1, nullptr);
}

/* Extract the balanced [...] array text following `"key":` —
 * the graph-content comparison F4 requires, independent of the
 * snapshot object's key order. Bracket depth is tracked outside
 * string literals (with backslash escapes honoured). Returns ""
 * when the key or its array is absent. */
static std::string json_array_text(const std::string& json, const std::string& key) {
    const std::string pattern = "\"" + key + "\"";
    std::size_t pos = json.find(pattern);
    if (pos == std::string::npos) return std::string();
    pos = json.find('[', pos + pattern.size());
    if (pos == std::string::npos) return std::string();
    const std::size_t start = pos;
    int depth = 0;
    bool in_string = false;
    bool escaped = false;
    for (; pos < json.size(); ++pos) {
        const char c = json[pos];
        if (in_string) {
            if (escaped) {
                escaped = false;
            } else if (c == '\\') {
                escaped = true;
            } else if (c == '"') {
                in_string = false;
            }
            continue;
        }
        if (c == '"') {
            in_string = true;
        } else if (c == '[') {
            ++depth;
        } else if (c == ']') {
            if (--depth == 0) return json.substr(start, pos - start + 1);
        }
    }
    return std::string();
}

static std::string read_file(const fs::path& path) {
    std::ifstream in(path, std::ios::binary);
    if (!in) return std::string();
    std::ostringstream ss;
    ss << in.rdbuf();
    return ss.str();
}

static bool write_file(const fs::path& path, const std::string& text) {
    std::ofstream out(path, std::ios::binary | std::ios::trunc);
    if (!out) return false;
    out << text;
    return static_cast<bool>(out);
}

/* Query expecting success; a thrown Error is a failed check. */
static std::string query_ok(const codetopo::Graph& graph, const std::string& op,
                            const std::string& args, const std::string& what) {
    ++checks;
    try {
        return graph.query(op, args);
    } catch (const codetopo::Error& e) {
        ++failures;
        std::printf("FAIL: %s: query %s threw %s: %s\n", what.c_str(), op.c_str(),
                    e.code().c_str(), e.message().c_str());
        return std::string();
    }
}

/* Rank one metric and compare every expected value at `tol`.
 * Eigenvector centrality is defined only up to sign, so with
 * `align_sign` the whole vector is flipped first when its dot
 * product with the golden is negative. */
static void check_rank_against(const codetopo::Graph& graph, const std::string& metric,
                               const std::vector<std::string>& ids,
                               const std::vector<double>& want, double tol,
                               const std::string& fixture, bool align_sign = false) {
    const std::string args = "{\"by\": \"" + metric + "\", \"limit\": 1000}";
    const std::string ans = query_ok(graph, "rank", args, fixture);
    if (ans.empty()) return;
    std::vector<double> got;
    got.reserve(ids.size());
    for (const std::string& id : ids) got.push_back(rank_value(ans, id));
    double flip = 1.0;
    if (align_sign) {
        double dot = 0.0;
        for (std::size_t i = 0; i < ids.size(); ++i) dot += got[i] * want[i];
        if (dot < 0.0) flip = -1.0;
    }
    for (std::size_t i = 0; i < ids.size(); ++i) {
        check_close(fixture + " rank by=" + metric + " [" + ids[i] + "]",
                    flip * got[i], want[i], tol);
    }
}

int main(int argc, char** argv) {
    if (argc != 3) {
        std::fprintf(stderr, "usage: conformance <repo_root> <work_dir>\n");
        return 2;
    }
    const fs::path root = argv[1];
    const fs::path work = argv[2];
    std::error_code ec;
    fs::create_directories(work, ec);

    /* ---- versions ------------------------------------------------ */
    check(codetopo::version().rfind("0.9", 0) == 0,
          "codetopo::version is the core's 0.9.x");
    check(codetopo::schema_version() == "2.0", "codetopo::schema_version is 2.0");
    std::printf("versions: core=%s schema=%s\n", codetopo::version().c_str(),
                codetopo::schema_version().c_str());

    /* ---- F2: quarry through codetopo::index ------------------------
     * The fixture README's known answers (26 nodes / 33 edges /
     * 2 diagnostics) are derived over the package's three Python
     * files. Indexing the fixture DIRECTORY also indexes its
     * README.md as Markdown — the honest full-directory answer is
     * 29 nodes / 36 edges / 2 diagnostics over 4 files. Both are
     * asserted: the package-files copy at the README's numbers,
     * the full directory at its own. */
    const fs::path fixture_dir = root / "crates/codetopo-extract/tests/fixture-py";
    const fs::path quarry_repo = work / "quarry-pkg";
    fs::create_directories(quarry_repo, ec);
    {
        bool copied = true;
        for (const char* name : {"__init__.py", "core.py", "shapes.py"}) {
            std::error_code copy_ec;
            fs::copy_file(fixture_dir / name, quarry_repo / name,
                          fs::copy_options::overwrite_existing, copy_ec);
            copied = copied && !copy_ec;
        }
        check(copied, "F2: quarry package files copied");
    }
    const fs::path quarry_db = work / "quarry.db";
    {
        std::string report;
        bool threw = false;
        try {
            report = codetopo::index(quarry_repo.string(), quarry_db.string(), "quarry");
        } catch (const codetopo::Error& e) {
            threw = true;
            std::printf("FAIL: F2: index threw %s: %s\n", e.code().c_str(),
                        e.message().c_str());
        }
        check(!threw && !report.empty(), "F2: codetopo::index(quarry package) succeeds");
        if (!report.empty()) {
            check(json_int_after(report, "", "\"nodes\"") == 26, "F2: report nodes == 26");
            check(json_int_after(report, "", "\"edges\"") == 33, "F2: report edges == 33");
            check(json_int_after(report, "", "\"diagnostics\"") == 2,
                  "F2: report diagnostics == 2");
        }
    }
    {
        std::string report;
        bool threw = false;
        try {
            report = codetopo::index(fixture_dir.string(), (work / "quarry-full.db").string(),
                                     "quarry");
        } catch (const codetopo::Error&) {
            threw = true;
        }
        check(!threw && !report.empty(), "F2: codetopo::index(fixture dir) succeeds");
        if (!report.empty()) {
            check(json_int_after(report, "", "\"files_indexed\"") == 4,
                  "F2: full-dir files_indexed == 4 (3 py + README.md)");
            check(json_int_after(report, "", "\"nodes\"") == 29,
                  "F2: full-dir nodes == 29 (README.md indexed as Markdown)");
            check(json_int_after(report, "", "\"edges\"") == 36, "F2: full-dir edges == 36");
            check(json_int_after(report, "", "\"diagnostics\"") == 2,
                  "F2: full-dir diagnostics == 2");
        }
    }
    codetopo::Graph quarry;
    {
        bool threw = false;
        try {
            quarry = codetopo::open_db(quarry_db.string());
        } catch (const codetopo::Error&) {
            threw = true;
        }
        check(!threw && quarry.valid(), "F2: codetopo::open_db(quarry.db)");
    }
    if (quarry) {
        const std::string stats = query_ok(quarry, "stats", "{}", "F2");
        if (!stats.empty()) {
            check(json_int_after(stats, "\"nodes\"", "\"total\"") == 26,
                  "F2: stats nodes == 26");
            check(json_int_after(stats, "\"edges\"", "\"total\"") == 33,
                  "F2: stats edges == 33");
        }
        const std::string diags = query_ok(quarry, "diagnostics", "{}", "F2");
        if (!diags.empty()) {
            check(json_int_after(diags, "", "\"total\"") == 2,
                  "F2: diagnostics total == 2");
        }
        const std::string verify = query_ok(quarry, "verify", "{}", "F2");
        if (!verify.empty()) {
            check(verify.find("\"chain_valid\":true") != std::string::npos,
                  "F2: verify confirms the indexed audit chain");
        }
        /* Move semantics: the moved-to handle owns the graph, the
         * moved-from handle is empty and refuses queries. */
        codetopo::Graph moved = std::move(quarry);
        check(moved.valid() && !quarry.valid(), "wrapper: Graph move transfers ownership");
        bool empty_threw = false;
        try {
            (void)quarry.query("stats", "{}");
        } catch (const codetopo::Error& e) {
            empty_threw = e.code() == "invalid_param";
        }
        check(empty_threw, "wrapper: query on a moved-from Graph throws invalid_param");
        quarry = std::move(moved);
    }
    std::printf("F2 quarry: done\n");

    /* ---- F1: oracle goldens -------------------------------------- */
    const std::vector<std::string> compass_ids = {"a", "b", "c", "d", "e", "f", "g", "h"};
    const std::vector<double> compass_pr = {0.050366533824645295, 0.0717724799737871,
        0.0717724799737871, 0.14187699226038397, 0.17096200118402893,
        0.1956833061815353, 0.21669618663194073, 0.08087001996989152};
    const std::vector<double> compass_bc = {0.0, 0.07142857142857142, 0.047619047619047616,
        0.21428571428571427, 0.19047619047619047, 0.11904761904761904, 0.0, 0.0};
    const std::vector<double> compass_cc = {0.0, 0.14285714285714285, 0.14285714285714285,
        0.3214285714285714, 0.2857142857142857, 0.27472527472527475,
        0.2706766917293233, 0.19047619047619047};
    const std::vector<double> compass_kz = {0.3182844374972241, 0.35011288124694656,
        0.35011288124694656, 0.3883070137466134, 0.35711513887188545,
        0.3539959513844127, 0.35368403263566545, 0.3532957256219188};

    {
        const std::string compass_json =
            read_file(root / "crates/codetopo-core/tests/fixtures/oracle/compass.json");
        check(!compass_json.empty(), "F1: compass fixture reads");
        codetopo::Graph compass;
        bool threw = false;
        try {
            compass = codetopo::load_snapshot(compass_json);
        } catch (const codetopo::Error&) {
            threw = true;
        }
        check(!threw && compass.valid(), "F1: Graph::load_snapshot(compass)");
        if (compass) {
            check_rank_against(compass, "pagerank", compass_ids, compass_pr, 1e-6, "compass");
            check_rank_against(compass, "betweenness", compass_ids, compass_bc, 1e-9, "compass");
            check_rank_against(compass, "closeness", compass_ids, compass_cc, 1e-9, "compass");
            check_rank_against(compass, "katz", compass_ids, compass_kz, 1e-6, "compass");
            /* Eigenvector on a DAG is ABSENT (nilpotent adjacency) —
             * measured:false with a reason, never zeros. */
            const std::string eig =
                query_ok(compass, "rank", "{\"by\": \"eigenvector\"}", "compass");
            if (!eig.empty()) {
                check(eig.find("\"measured\":false") != std::string::npos,
                      "F1: compass eigenvector is absent (measured:false)");
            }
            /* F5: repeated float queries byte-identical. */
            const std::string r1 =
                query_ok(compass, "rank", "{\"by\": \"pagerank\", \"limit\": 1000}", "F5");
            const std::string r2 =
                query_ok(compass, "rank", "{\"by\": \"pagerank\", \"limit\": 1000}", "F5");
            check(!r1.empty() && r1 == r2, "F5: repeated rank answers byte-identical");
        }
    }

    const std::vector<std::string> ring_ids = {"w", "x", "y", "z"};
    const std::vector<double> ring_pr = {0.2813632148631319, 0.276658031180747,
        0.15508018934150383, 0.28689856461461727};
    const std::vector<double> ring_bc = {0.5, 0.5, 0.0, 0.5};
    const std::vector<double> ring_cc = {0.6, 0.5, 0.5, 0.75};
    const std::vector<double> ring_ev = {0.5262015076917937, 0.43104970112724544,
        0.3531051686825981, 0.6423588314257299};
    const std::vector<double> ring_kz = {0.4909351618601384, 0.48655572748762066,
        0.486117784050369, 0.5347295493315389};
    {
        const std::string ring_json =
            read_file(root / "crates/codetopo-core/tests/fixtures/oracle/ring.json");
        check(!ring_json.empty(), "F1: ring fixture reads");
        codetopo::Graph ring;
        bool threw = false;
        try {
            ring = codetopo::Graph::load_snapshot(ring_json);
        } catch (const codetopo::Error&) {
            threw = true;
        }
        check(!threw && ring.valid(), "F1: Graph::load_snapshot(ring)");
        if (ring) {
            check_rank_against(ring, "pagerank", ring_ids, ring_pr, 1e-6, "ring");
            check_rank_against(ring, "betweenness", ring_ids, ring_bc, 1e-9, "ring");
            check_rank_against(ring, "closeness", ring_ids, ring_cc, 1e-9, "ring");
            check_rank_against(ring, "eigenvector", ring_ids, ring_ev, 1e-6, "ring",
                               /*align_sign=*/true);
            check_rank_against(ring, "katz", ring_ids, ring_kz, 1e-6, "ring");
        }
    }

    /* cycle-trap: topo-sort groups exact — lib, the flagged cycle
     * {x,y,z}, mid, app; 4 groups, 6 nodes, 1 cyclic group. */
    {
        const std::string trap_json =
            read_file(root / "crates/codetopo-core/tests/fixtures/oracle/cycle-trap.json");
        check(!trap_json.empty(), "F1: cycle-trap fixture reads");
        codetopo::Graph trap;
        bool threw = false;
        try {
            trap = codetopo::load_snapshot(trap_json);
        } catch (const codetopo::Error&) {
            threw = true;
        }
        check(!threw && trap.valid(), "F1: Graph::load_snapshot(cycle-trap)");
        if (trap) {
            const std::string topo = query_ok(trap, "topo_sort", "{}", "cycle-trap");
            if (!topo.empty()) {
                check(json_int_after(topo, "", "\"total_groups\"") == 4,
                      "F1: cycle-trap total_groups == 4");
                check(json_int_after(topo, "", "\"total_nodes\"") == 6,
                      "F1: cycle-trap total_nodes == 6");
                check(json_int_after(topo, "", "\"cyclic_groups\"") == 1,
                      "F1: cycle-trap cyclic_groups == 1");
                check(topo.find("\"cyclic\":true") != std::string::npos,
                      "F1: cycle-trap cycle group flagged");
                const std::size_t lib = topo.find("\"lib\"");
                const std::size_t x = topo.find("\"x\"");
                const std::size_t mid = topo.find("\"mid\"");
                const std::size_t app = topo.find("\"app\"");
                check(lib != std::string::npos && x != std::string::npos &&
                          mid != std::string::npos && app != std::string::npos &&
                          lib < x && x < mid && mid < app,
                      "F1: cycle-trap group order lib < cycle < mid < app");
            }
        }
    }
    std::printf("F1 oracle goldens: done\n");

    /* ---- F3: Aarona sibling tripwire ------------------------------ */
    {
        const fs::path sibling_dir = work / "sibling";
        const fs::path sibling_repo = sibling_dir / "repo";
        fs::create_directories(sibling_repo, ec);
        const std::vector<std::string> names = {"a.py", "b.py", "c.py", "d.py", "e.py"};
        const std::vector<std::string> bodies = {
            "import b\nimport c\n\n\ndef run():\n    b.work()\n    c.work()\n",
            "import c\n\n\ndef work():\n    c.work()\n",
            "import d\n\n\ndef work():\n    d.work()\n",
            "import e\n\n\ndef work():\n    e.work()\n",
            "import a\nimport c\n\n\ndef work():\n    a.run()\n    c.work()\n",
        };
        bool wrote = true;
        for (std::size_t i = 0; i < names.size(); ++i) {
            wrote = write_file(sibling_repo / names[i], bodies[i]) && wrote;
        }
        check(wrote, "F3: sibling fixture files written");
        const fs::path sibling_db = sibling_dir / "sibling.db";
        bool index_threw = false;
        try {
            (void)codetopo::index(sibling_repo.string(), sibling_db.string(), "repo");
        } catch (const codetopo::Error&) {
            index_threw = true;
        }
        check(!index_threw, "F3: codetopo::index(sibling fixture)");
        codetopo::Graph sibling;
        bool open_threw = false;
        try {
            sibling = codetopo::open_db(sibling_db.string());
        } catch (const codetopo::Error&) {
            open_threw = true;
        }
        check(!open_threw && sibling.valid(), "F3: codetopo::open_db(sibling.db)");
        if (sibling) {
            const std::string stats = query_ok(sibling, "stats", "{}", "F3");
            if (!stats.empty()) {
                check(json_int_after(stats, "\"nodes\"", "\"total\"") == 16,
                      "F3: stats nodes == 16");
                check(json_int_after(stats, "\"edges\"", "\"imports\"") == 7,
                      "F3: imports 7/7 internal");
                check(json_int_after(stats, "\"edges\"", "\"calls\"") == 7,
                      "F3: calls 7/7 internal");
                const std::size_t nodes_seg = stats.find("\"nodes\"");
                check(nodes_seg == std::string::npos ||
                          stats.find("\"external\"", nodes_seg) == std::string::npos,
                      "F3: no external stub nodes");
            }
            const std::string topo = query_ok(sibling, "topo_sort", "{}", "F3");
            if (!topo.empty()) {
                check(json_int_after(topo, "", "\"cyclic_groups\"") == 2,
                      "F3: topo_sort cyclic_groups == 2");
            }
            /* rank discipline: a non-`by` metric key throws,
             * naming the key — the Error carries code + message. */
            bool rank_threw = false;
            try {
                (void)sibling.query("rank", "{\"metric\": \"pagerank\"}");
            } catch (const codetopo::Error& e) {
                rank_threw = true;
                check(e.code() == "invalid_param",
                      "F3: rank 'metric' error code is invalid_param");
                check(e.message().find("metric") != std::string::npos &&
                          e.message().find("'by'") != std::string::npos,
                      "F3: rank 'metric' error names the key and 'by'");
            }
            check(rank_threw, "F3: rank with 'metric' key throws");
            /* An unknown op throws too — never a silent default. */
            bool op_threw = false;
            try {
                (void)sibling.query("nope_op", "{}");
            } catch (const codetopo::Error& e) {
                op_threw = e.code() == "invalid_param";
            }
            check(op_threw, "F3: unknown op throws invalid_param");
        }
    }
    std::printf("F3 sibling tripwire: done\n");

    /* ---- F4: lifecycle, compared by graph content ----------------- */
    if (quarry) {
        const std::string snap1 = query_ok(quarry, "snapshot", "{}", "F4");
        codetopo::Graph loaded;
        bool load_threw = false;
        try {
            if (!snap1.empty()) loaded = codetopo::load_snapshot(snap1);
        } catch (const codetopo::Error&) {
            load_threw = true;
        }
        check(!load_threw && loaded.valid(), "F4: snapshot loads back into a handle");
        if (loaded) {
            const std::string verify = query_ok(loaded, "verify", "{}", "F4");
            if (!verify.empty()) {
                check(verify.find("\"chain_valid\":true") != std::string::npos,
                      "F4: verify op confirms the snapshot's audit chain");
            }
            const std::string snap2 = query_ok(loaded, "snapshot", "{}", "F4");
            if (!snap1.empty() && !snap2.empty()) {
                const std::string nodes1 = json_array_text(snap1, "nodes");
                const std::string nodes2 = json_array_text(snap2, "nodes");
                check(!nodes1.empty() && nodes1 == nodes2,
                      "F4: snapshot round-trip — nodes payload identical");
                const std::string edges1 = json_array_text(snap1, "edges");
                const std::string edges2 = json_array_text(snap2, "edges");
                check(!edges1.empty() && edges1 == edges2,
                      "F4: snapshot round-trip — edges payload identical");
            }
        }
        /* F5 on the indexed graph too. */
        const std::string s1 = query_ok(quarry, "stats", "{}", "F5");
        const std::string s2 = query_ok(quarry, "stats", "{}", "F5");
        check(!s1.empty() && s1 == s2, "F5: repeated stats byte-identical");
    }
    std::printf("F4 lifecycle + F5 determinism: done\n");

    /* ---- error shape ---------------------------------------------- */
    {
        bool threw = false;
        try {
            (void)codetopo::open_db("/nonexistent/no-such-index.db");
        } catch (const codetopo::Error& e) {
            threw = true;
            check(e.code() == "unknown_index",
                  "errors: missing db error code is unknown_index");
            check(!e.message().empty(), "errors: missing db error carries a message");
        }
        check(threw, "errors: open_db on a missing db throws");

        threw = false;
        try {
            (void)codetopo::load_snapshot(
                "{\"schema_version\": \"9.0\", \"nodes\": [], \"edges\": []}");
        } catch (const codetopo::Error& e) {
            threw = true;
            check(e.code() == "unsupported_schema_version",
                  "errors: schema mismatch code is unsupported_schema_version");
        }
        check(threw, "errors: schema 9.0 snapshot throws");

        /* The CString guard itself: owns, moves, and empties. */
        codetopo::CString empty_guard;
        check(empty_guard.empty() && empty_guard.str().empty(),
              "wrapper: default CString is empty");
    }

    std::printf("conformance: %d checks, %d failures\n", checks, failures);
    if (failures == 0) std::printf("CONFORMANCE PASS\n");
    return failures == 0 ? 0 : 1;
}
