/* Copyright (C) 2026 2351485 ALBERTA INC.
 * SPDX-License-Identifier: AGPL-3.0-or-later
 * Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
 *
 * C conformance program for the codetopo C ABI.
 *
 * Exercises the known-answer battery (F1-F5) through the actual C
 * ABI — the built cdylib, the hand-written header, real C callers'
 * ownership discipline — never through Rust:
 *
 *   F1  oracle goldens (compass, ring, cycle-trap): rank answers at
 *       the core's own tolerances (PageRank 1e-6, betweenness 1e-9,
 *       closeness 1e-9, eigenvector 1e-6, Katz 1e-6), eigenvector's
 *       absence on the compass DAG, topo-sort groups/cycle flags
 *       exact.
 *   F2  the quarry package indexed via codetopo_index:
 *       26 nodes / 33 edges / 2 diagnostics.
 *   F3  the Aarona sibling tripwire (0.9.1 regression pin): imports
 *       7/7 and calls 7/7 internal (no external stubs), topo-sort
 *       cyclic_groups == 2, and rank with a non-`by` metric key
 *       errors naming the key.
 *   F4  lifecycle: index -> snapshot -> load -> snapshot, compared
 *       by GRAPH CONTENT (nodes/edges payloads — snapshots embed
 *       wall-clock audit timestamps, so byte-identity is the known
 *       flake, not the contract); chains checked via `verify`.
 *   F5  determinism: repeated queries byte-identical.
 *
 * Usage: conformance <repo_root> <work_dir>
 *   <repo_root>  the codetopo repository checkout (fixtures live in it)
 *   <work_dir>   a writable scratch directory (created if needed)
 *
 * Every expected number below is a known answer: the oracle goldens
 * committed in crates/codetopo-core/tests/fixtures/oracle/ (produced
 * by networkx / python-igraph as validation oracles), the quarry
 * README's hand-derived counts, and the sibling fixture's ground
 * truth as pinned by crates/codetopo-cli/tests/py_sibling_fixture.rs.
 */

#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "codetopo.h"

static int checks = 0;
static int failures = 0;

static void check(int cond, const char *what) {
    checks++;
    if (!cond) {
        failures++;
        printf("FAIL: %s\n", what);
    }
}

static void check_close(const char *what, double got, double want, double tol) {
    checks++;
    if (fabs(got - want) > tol) {
        failures++;
        printf("FAIL: %s: got %.12g want %.12g (tol %g)\n", what, got, want, tol);
    }
}

/* ---- small JSON-by-substring helpers (answers are compact JSON) ---- */

/* Parse the integer following `"key":` at or after `start`. */
static long json_int_after(const char *json, const char *start, const char *key) {
    const char *p = start ? strstr(json, start) : json;
    if (!p) return -1;
    p = strstr(p, key);
    if (!p) return -1;
    p = strchr(p, ':');
    if (!p) return -1;
    return strtol(p + 1, NULL, 10);
}

/* Parse the double in a rank entry: find `"id":"<id>"`, then its `"value":`. */
static double rank_value(const char *json, const char *id) {
    char pat[256];
    snprintf(pat, sizeof pat, "\"id\":\"%s\"", id);
    const char *p = strstr(json, pat);
    if (!p) return NAN;
    p = strstr(p, "\"value\":");
    if (!p) return NAN;
    return strtod(strchr(p, ':') + 1, NULL);
}

/* Byte-compare the segment of two documents between two markers. */
static int segment_equal(const char *a, const char *b,
                         const char *from, const char *to) {
    const char *a1 = strstr(a, from), *b1 = strstr(b, from);
    if (!a1 || !b1) return 0;
    const char *a2 = strstr(a1, to), *b2 = strstr(b1, to);
    if (!a2 || !b2) return 0;
    size_t la = (size_t)(a2 - a1), lb = (size_t)(b2 - b1);
    return la == lb && memcmp(a1, b1, la) == 0;
}

static char *read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc((size_t)n + 1);
    if (fread(buf, 1, (size_t)n, f) != (size_t)n) { fclose(f); free(buf); return NULL; }
    buf[n] = '\0';
    fclose(f);
    return buf;
}

static int write_file(const char *path, const char *text) {
    FILE *f = fopen(path, "wb");
    if (!f) return 0;
    fputs(text, f);
    fclose(f);
    return 1;
}

static void path_join(char *out, size_t cap, const char *a, const char *b) {
    snprintf(out, cap, "%s/%s", a, b);
}

/* Query expecting success; returns the answer (caller frees). */
static char *query_ok(codetopo_graph *g, const char *op, const char *args,
                      const char *what) {
    char *err = NULL;
    char *ans = codetopo_query(g, op, args, &err);
    checks++;
    if (!ans) {
        failures++;
        printf("FAIL: %s: query %s failed: %s\n", what, op, err ? err : "(no error)");
    }
    if (err) {
        failures++;
        printf("FAIL: %s: query %s set err_out on success\n", what, op);
        codetopo_free_string(err);
    }
    return ans;
}

static void check_rank_against(codetopo_graph *g, const char *metric,
                               const char *const *ids, const double *want,
                               int n, double tol, const char *fixture) {
    char args[128], what[256];
    snprintf(args, sizeof args, "{\"by\": \"%s\", \"limit\": 1000}", metric);
    char *ans = query_ok(g, "rank", args, fixture);
    if (!ans) return;
    for (int i = 0; i < n; i++) {
        snprintf(what, sizeof what, "%s rank by=%s [%s]", fixture, metric, ids[i]);
        check_close(what, rank_value(ans, ids[i]), want[i], tol);
    }
    codetopo_free_string(ans);
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: conformance <repo_root> <work_dir>\n");
        return 2;
    }
    const char *root = argv[1];
    const char *work = argv[2];
    char p1[2048], p2[2048], p3[2048];
    mkdir(work, 0777);

    /* ---- versions ------------------------------------------------ */
    const char *version = codetopo_version();
    check(version && strncmp(version, "0.9", 3) == 0, "codetopo_version is the core's 0.9.x");
    const char *schema = codetopo_schema_version();
    check(schema && strcmp(schema, "1.2") == 0, "codetopo_schema_version is 1.2");
    printf("versions: core=%s schema=%s\n", version, schema);

    /* ---- F2: quarry through codetopo_index ----------------------- */
    /* The known-answer package is exactly the fixture's three `.py`
     * modules — what known_answer_py.rs feeds the builder. Indexing
     * the fixture directory wholesale would also index its README.md
     * (Markdown is a supported language) and pyproject.toml, giving
     * 29/36 instead of the package's 26/33/2, so the three modules
     * are copied into a clean repo dir first. */
    path_join(p1, sizeof p1, work, "quarry-src");
    mkdir(p1, 0777);
    {
        static const char *mods[] = {"__init__.py", "core.py", "shapes.py"};
        int copied = 1;
        for (int i = 0; i < 3; i++) {
            char src[2048], dst[2048];
            path_join(src, sizeof src, root, "crates/codetopo-extract/tests/fixture-py");
            size_t len = strlen(src);
            snprintf(src + len, sizeof src - len, "/%s", mods[i]);
            path_join(dst, sizeof dst, p1, mods[i]);
            char *body = read_file(src);
            copied = body && write_file(dst, body) && copied;
            free(body);
        }
        check(copied, "F2: fixture modules copied");
    }
    path_join(p2, sizeof p2, work, "quarry.db");
    {
        char *err = NULL;
        char *report = codetopo_index(p1, p2, "quarry", &err);
        check(report != NULL, "F2: codetopo_index(fixture-py) succeeds");
        if (report) {
            check(json_int_after(report, NULL, "\"nodes\"") == 26, "F2: report nodes == 26");
            check(json_int_after(report, NULL, "\"edges\"") == 33, "F2: report edges == 33");
            check(json_int_after(report, NULL, "\"diagnostics\"") == 2, "F2: report diagnostics == 2");
            codetopo_free_string(report);
        }
        if (err) { check(0, "F2: index set err_out on success"); codetopo_free_string(err); }
    }
    codetopo_graph *quarry = NULL;
    {
        char *err = NULL;
        quarry = codetopo_open_db(p2, &err);
        check(quarry != NULL, "F2: codetopo_open_db(quarry.db)");
        if (err) codetopo_free_string(err);
    }
    if (quarry) {
        char *stats = query_ok(quarry, "stats", "{}", "F2");
        if (stats) {
            check(json_int_after(stats, "\"nodes\"", "\"total\"") == 26, "F2: stats nodes == 26");
            check(json_int_after(stats, "\"edges\"", "\"total\"") == 33, "F2: stats edges == 33");
            codetopo_free_string(stats);
        }
        char *diags = query_ok(quarry, "diagnostics", "{}", "F2");
        if (diags) {
            check(json_int_after(diags, NULL, "\"total\"") == 2, "F2: diagnostics total == 2");
            codetopo_free_string(diags);
        }
    }
    printf("F2 quarry: done\n");

    /* ---- F1: oracle goldens -------------------------------------- */
    static const char *compass_ids[] = {"a", "b", "c", "d", "e", "f", "g", "h"};
    static const double compass_pr[] = {0.050366533824645295, 0.0717724799737871,
        0.0717724799737871, 0.14187699226038397, 0.17096200118402893,
        0.1956833061815353, 0.21669618663194073, 0.08087001996989152};
    static const double compass_bc[] = {0.0, 0.07142857142857142, 0.047619047619047616,
        0.21428571428571427, 0.19047619047619047, 0.11904761904761904, 0.0, 0.0};
    static const double compass_cc[] = {0.0, 0.14285714285714285, 0.14285714285714285,
        0.3214285714285714, 0.2857142857142857, 0.27472527472527475,
        0.2706766917293233, 0.19047619047619047};
    static const double compass_kz[] = {0.3182844374972241, 0.35011288124694656,
        0.35011288124694656, 0.3883070137466134, 0.35711513887188545,
        0.3539959513844127, 0.35368403263566545, 0.3532957256219188};

    path_join(p1, sizeof p1, root, "crates/codetopo-core/tests/fixtures/oracle/compass.json");
    char *compass_json = read_file(p1);
    check(compass_json != NULL, "F1: compass fixture reads");
    codetopo_graph *compass = NULL;
    if (compass_json) {
        char *err = NULL;
        compass = codetopo_load_snapshot(compass_json, &err);
        check(compass != NULL, "F1: codetopo_load_snapshot(compass)");
        if (err) codetopo_free_string(err);
    }
    if (compass) {
        check_rank_against(compass, "pagerank", compass_ids, compass_pr, 8, 1e-6, "compass");
        check_rank_against(compass, "betweenness", compass_ids, compass_bc, 8, 1e-9, "compass");
        check_rank_against(compass, "closeness", compass_ids, compass_cc, 8, 1e-9, "compass");
        check_rank_against(compass, "katz", compass_ids, compass_kz, 8, 1e-6, "compass");
        /* Eigenvector on a DAG is ABSENT (nilpotent adjacency) — the
         * answer is measured:false with a reason, never zeros. */
        char *eig = query_ok(compass, "rank", "{\"by\": \"eigenvector\"}", "compass");
        if (eig) {
            check(strstr(eig, "\"measured\":false") != NULL,
                  "F1: compass eigenvector is absent (measured:false)");
            codetopo_free_string(eig);
        }
        /* F5: repeated float queries byte-identical. */
        char *r1 = query_ok(compass, "rank", "{\"by\": \"pagerank\", \"limit\": 1000}", "F5");
        char *r2 = query_ok(compass, "rank", "{\"by\": \"pagerank\", \"limit\": 1000}", "F5");
        if (r1 && r2) check(strcmp(r1, r2) == 0, "F5: repeated rank answers byte-identical");
        if (r1) codetopo_free_string(r1);
        if (r2) codetopo_free_string(r2);
        codetopo_graph_free(compass);
    }
    free(compass_json);

    static const char *ring_ids[] = {"w", "x", "y", "z"};
    static const double ring_pr[] = {0.2813632148631319, 0.276658031180747,
        0.15508018934150383, 0.28689856461461727};
    static const double ring_bc[] = {0.5, 0.5, 0.0, 0.5};
    static const double ring_cc[] = {0.6, 0.5, 0.5, 0.75};
    static const double ring_ev[] = {0.5262015076917937, 0.43104970112724544,
        0.3531051686825981, 0.6423588314257299};
    static const double ring_kz[] = {0.4909351618601384, 0.48655572748762066,
        0.486117784050369, 0.5347295493315389};

    path_join(p1, sizeof p1, root, "crates/codetopo-core/tests/fixtures/oracle/ring.json");
    char *ring_json = read_file(p1);
    check(ring_json != NULL, "F1: ring fixture reads");
    codetopo_graph *ring = NULL;
    if (ring_json) {
        char *err = NULL;
        ring = codetopo_load_snapshot(ring_json, &err);
        check(ring != NULL, "F1: codetopo_load_snapshot(ring)");
        if (err) codetopo_free_string(err);
    }
    if (ring) {
        check_rank_against(ring, "pagerank", ring_ids, ring_pr, 4, 1e-6, "ring");
        check_rank_against(ring, "betweenness", ring_ids, ring_bc, 4, 1e-9, "ring");
        check_rank_against(ring, "closeness", ring_ids, ring_cc, 4, 1e-9, "ring");
        check_rank_against(ring, "eigenvector", ring_ids, ring_ev, 4, 1e-6, "ring");
        check_rank_against(ring, "katz", ring_ids, ring_kz, 4, 1e-6, "ring");
        codetopo_graph_free(ring);
    }
    free(ring_json);

    /* cycle-trap: topo-sort groups exact — lib, the flagged cycle
     * {x,y,z}, mid, app; 4 groups, 6 nodes, 1 cyclic group. */
    path_join(p1, sizeof p1, root, "crates/codetopo-core/tests/fixtures/oracle/cycle-trap.json");
    char *trap_json = read_file(p1);
    check(trap_json != NULL, "F1: cycle-trap fixture reads");
    codetopo_graph *trap = NULL;
    if (trap_json) {
        char *err = NULL;
        trap = codetopo_load_snapshot(trap_json, &err);
        check(trap != NULL, "F1: codetopo_load_snapshot(cycle-trap)");
        if (err) codetopo_free_string(err);
    }
    if (trap) {
        char *topo = query_ok(trap, "topo_sort", "{}", "cycle-trap");
        if (topo) {
            check(json_int_after(topo, NULL, "\"total_groups\"") == 4, "F1: cycle-trap total_groups == 4");
            check(json_int_after(topo, NULL, "\"total_nodes\"") == 6, "F1: cycle-trap total_nodes == 6");
            check(json_int_after(topo, NULL, "\"cyclic_groups\"") == 1, "F1: cycle-trap cyclic_groups == 1");
            check(strstr(topo, "\"cyclic\":true") != NULL, "F1: cycle-trap cycle group flagged");
            const char *lib = strstr(topo, "\"lib\"");
            const char *x = strstr(topo, "\"x\"");
            const char *mid = strstr(topo, "\"mid\"");
            const char *app = strstr(topo, "\"app\"");
            check(lib && x && mid && app && lib < x && x < mid && mid < app,
                  "F1: cycle-trap group order lib < cycle < mid < app");
            codetopo_free_string(topo);
        }
        codetopo_graph_free(trap);
    }
    free(trap_json);
    printf("F1 oracle goldens: done\n");

    /* ---- F3: Aarona sibling tripwire ------------------------------ */
    path_join(p1, sizeof p1, work, "sibling");
    mkdir(p1, 0777);
    path_join(p2, sizeof p2, p1, "repo");
    mkdir(p2, 0777);
    static const char *sibling_names[] = {"a.py", "b.py", "c.py", "d.py", "e.py"};
    static const char *sibling_bodies[] = {
        "import b\nimport c\n\n\ndef run():\n    b.work()\n    c.work()\n",
        "import c\n\n\ndef work():\n    c.work()\n",
        "import d\n\n\ndef work():\n    d.work()\n",
        "import e\n\n\ndef work():\n    e.work()\n",
        "import a\nimport c\n\n\ndef work():\n    a.run()\n    c.work()\n",
    };
    int wrote = 1;
    for (int i = 0; i < 5; i++) {
        path_join(p3, sizeof p3, p2, sibling_names[i]);
        wrote = write_file(p3, sibling_bodies[i]) && wrote;
    }
    check(wrote, "F3: sibling fixture files written");
    path_join(p3, sizeof p3, p1, "sibling.db");
    codetopo_graph *sibling = NULL;
    {
        char *err = NULL;
        char *report = codetopo_index(p2, p3, "repo", &err);
        check(report != NULL, "F3: codetopo_index(sibling fixture)");
        if (report) codetopo_free_string(report);
        if (err) codetopo_free_string(err);
        err = NULL;
        sibling = codetopo_open_db(p3, &err);
        check(sibling != NULL, "F3: codetopo_open_db(sibling.db)");
        if (err) codetopo_free_string(err);
    }
    if (sibling) {
        char *stats = query_ok(sibling, "stats", "{}", "F3");
        if (stats) {
            check(json_int_after(stats, "\"nodes\"", "\"total\"") == 16, "F3: stats nodes == 16");
            check(json_int_after(stats, "\"edges\"", "\"imports\"") == 7, "F3: imports 7/7 internal");
            check(json_int_after(stats, "\"edges\"", "\"calls\"") == 7, "F3: calls 7/7 internal");
            const char *nodes_seg = strstr(stats, "\"nodes\"");
            check(!nodes_seg || !strstr(nodes_seg, "\"external\""),
                  "F3: no external stub nodes");
            codetopo_free_string(stats);
        }
        char *topo = query_ok(sibling, "topo_sort", "{}", "F3");
        if (topo) {
            check(json_int_after(topo, NULL, "\"cyclic_groups\"") == 2,
                  "F3: topo_sort cyclic_groups == 2");
            codetopo_free_string(topo);
        }
        /* rank discipline: a non-`by` metric key errors, naming the key. */
        char *err = NULL;
        char *ans = codetopo_query(sibling, "rank", "{\"metric\": \"pagerank\"}", &err);
        check(ans == NULL, "F3: rank with 'metric' key returns NULL");
        check(err && strstr(err, "\"code\":\"invalid_param\"") != NULL,
              "F3: rank 'metric' error code is invalid_param");
        check(err && strstr(err, "metric") != NULL && strstr(err, "'by'") != NULL,
              "F3: rank 'metric' error names the key and 'by'");
        if (ans) codetopo_free_string(ans);
        if (err) codetopo_free_string(err);
        codetopo_graph_free(sibling);
    }
    printf("F3 sibling tripwire: done\n");

    /* ---- F4: lifecycle, compared by graph content ----------------- */
    if (quarry) {
        char *snap1 = query_ok(quarry, "snapshot", "{}", "F4");
        codetopo_graph *loaded = NULL;
        if (snap1) {
            char *err = NULL;
            loaded = codetopo_load_snapshot(snap1, &err);
            check(loaded != NULL, "F4: snapshot loads back into a handle");
            if (err) codetopo_free_string(err);
        }
        if (loaded) {
            char *verify = query_ok(loaded, "verify", "{}", "F4");
            if (verify) {
                check(strstr(verify, "\"chain_valid\":true") != NULL,
                      "F4: verify op confirms the snapshot's audit chain");
                codetopo_free_string(verify);
            }
            char *snap2 = query_ok(loaded, "snapshot", "{}", "F4");
            if (snap1 && snap2) {
                check(segment_equal(snap1, snap2, "\"nodes\":", "\"edges\":"),
                      "F4: snapshot round-trip — nodes payload identical");
                check(segment_equal(snap1, snap2, "\"edges\":", "\"audit_head\""),
                      "F4: snapshot round-trip — edges payload identical");
            }
            if (snap2) codetopo_free_string(snap2);
            codetopo_graph_free(loaded);
        }
        if (snap1) codetopo_free_string(snap1);

        /* F5 on the indexed graph too. */
        char *s1 = query_ok(quarry, "stats", "{}", "F5");
        char *s2 = query_ok(quarry, "stats", "{}", "F5");
        if (s1 && s2) check(strcmp(s1, s2) == 0, "F5: repeated stats byte-identical");
        if (s1) codetopo_free_string(s1);
        if (s2) codetopo_free_string(s2);
        codetopo_graph_free(quarry);
    }
    printf("F4 lifecycle + F5 determinism: done\n");

    /* ---- error shape ---------------------------------------------- */
    {
        char *err = NULL;
        codetopo_graph *missing = codetopo_open_db("/nonexistent/no-such-index.db", &err);
        check(missing == NULL, "errors: open_db on a missing db returns NULL");
        check(err && strstr(err, "\"code\":\"unknown_index\"") != NULL,
              "errors: missing db error code is unknown_index");
        if (err) codetopo_free_string(err);

        err = NULL;
        codetopo_graph *future = codetopo_load_snapshot(
            "{\"schema_version\": \"9.0\", \"nodes\": [], \"edges\": []}", &err);
        check(future == NULL, "errors: schema 9.0 snapshot returns NULL");
        check(err && strstr(err, "\"code\":\"unsupported_schema_version\"") != NULL,
              "errors: schema mismatch code is unsupported_schema_version");
        if (err) codetopo_free_string(err);
    }

    printf("conformance: %d checks, %d failures\n", checks, failures);
    if (failures == 0) printf("CONFORMANCE PASS\n");
    return failures == 0 ? 0 : 1;
}
