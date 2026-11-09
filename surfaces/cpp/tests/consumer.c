/* Copyright (C) 2026 2351485 ALBERTA INC.
 * SPDX-License-Identifier: AGPL-3.0-or-later
 * Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
 *
 * Consumer smoke program for the shipped C header.
 *
 * This program is the product test of crates/codetopo-c's header:
 * it includes ONLY <codetopo.h> — the header as shipped in
 * crates/codetopo-c/include/ — and links ONLY against the built
 * libcodetopo_c, exactly as an external C consumer would. No
 * repo-internal header, no Rust, no helper library. If the header
 * were not self-contained, or the ABI not usable without the
 * source tree, this file would not compile or run.
 *
 * What it proves, end to end:
 *   - the version functions answer (static strings, never freed);
 *   - codetopo_index indexes the quarry fixture's package files
 *     (F2's package: the three .py files — indexing the fixture
 *     directory itself would also index its README.md as
 *     Markdown, a different, larger graph) and its report
 *     carries the known-answer counts;
 *   - codetopo_open_db + codetopo_query answer stats/verify;
 *   - the snapshot op's output loads back via
 *     codetopo_load_snapshot (the portable form);
 *   - an error comes back as NULL + a {"code","message"} JSON
 *     object, freed with codetopo_free_string like any answer;
 *   - every returned string is released with
 *     codetopo_free_string and every handle with
 *     codetopo_graph_free — the ownership discipline the header
 *     documents, exercised, not just described.
 *
 * Usage: consumer <repo_root> <work_dir>
 *   <repo_root>  the codetopo repository checkout (fixtures live in it)
 *   <work_dir>   a writable scratch directory (created if needed)
 */

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

/* The integer following `"key":` in a compact-JSON answer. Test
 * scaffolding only — consumers parse answers with a real parser. */
static long json_int_after(const char *json, const char *key) {
    const char *p = strstr(json, key);
    if (!p) return -1;
    p = strchr(p, ':');
    if (!p) return -1;
    return strtol(p + 1, NULL, 10);
}

static int copy_file(const char *from, const char *to) {
    FILE *in = fopen(from, "rb");
    if (!in) return 0;
    FILE *out = fopen(to, "wb");
    if (!out) { fclose(in); return 0; }
    char buf[8192];
    size_t n;
    int ok = 1;
    while ((n = fread(buf, 1, sizeof buf, in)) > 0) {
        if (fwrite(buf, 1, n, out) != n) { ok = 0; break; }
    }
    fclose(in);
    fclose(out);
    return ok;
}

static char *read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc((size_t)n + 1);
    if (!buf) { fclose(f); return NULL; }
    if (n > 0 && fread(buf, 1, (size_t)n, f) != (size_t)n) {
        fclose(f);
        free(buf);
        return NULL;
    }
    buf[n] = '\0';
    fclose(f);
    return buf;
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: consumer <repo_root> <work_dir>\n");
        return 2;
    }
    const char *root = argv[1];
    const char *work = argv[2];
    char fixture[2048], repo[2048], db[2048], snap_path[2048], src[4096], dst[4096];
    mkdir(work, 0777);
    snprintf(fixture, sizeof fixture, "%s/crates/codetopo-extract/tests/fixture-py", root);
    snprintf(repo, sizeof repo, "%s/quarry-pkg", work);
    mkdir(repo, 0777);
    /* The F2 package is the fixture's three Python files (see the
     * header comment): copy them into a scratch package dir. */
    static const char *py_files[] = {"__init__.py", "core.py", "shapes.py"};
    int copied = 1;
    for (int i = 0; i < 3; i++) {
        snprintf(src, sizeof src, "%s/%s", fixture, py_files[i]);
        snprintf(dst, sizeof dst, "%s/%s", repo, py_files[i]);
        copied = copy_file(src, dst) && copied;
    }
    check(copied, "consumer: quarry package files copied");
    snprintf(db, sizeof db, "%s/consumer-quarry.db", work);
    snprintf(snap_path, sizeof snap_path,
             "%s/crates/codetopo-core/tests/fixtures/oracle/compass.json", root);

    /* Versions: static strings — read them, never free them. */
    const char *version = codetopo_version();
    check(version && version[0] != '\0', "consumer: codetopo_version answers");
    const char *schema = codetopo_schema_version();
    check(schema && strcmp(schema, "1.2") == 0, "consumer: schema version is 1.2");
    printf("consumer: core=%s schema=%s\n", version, schema);

    /* Index the quarry fixture through the ABI. */
    char *err = NULL;
    char *report = codetopo_index(repo, db, "quarry", &err);
    check(report != NULL, "consumer: codetopo_index succeeds");
    check(err == NULL, "consumer: index leaves err_out NULL on success");
    if (report) {
        check(json_int_after(report, "\"nodes\"") == 26, "consumer: report nodes == 26");
        check(json_int_after(report, "\"edges\"") == 33, "consumer: report edges == 33");
        check(json_int_after(report, "\"diagnostics\"") == 2,
              "consumer: report diagnostics == 2");
        codetopo_free_string(report);
    }
    if (err) codetopo_free_string(err);

    /* Open the database and query it. */
    err = NULL;
    codetopo_graph *graph = codetopo_open_db(db, &err);
    check(graph != NULL, "consumer: codetopo_open_db succeeds");
    if (err) codetopo_free_string(err);
    char *snapshot = NULL;
    if (graph) {
        err = NULL;
        char *stats = codetopo_query(graph, "stats", "{}", &err);
        check(stats != NULL, "consumer: stats query succeeds");
        if (stats) {
            const char *nodes = strstr(stats, "\"nodes\"");
            check(nodes && json_int_after(nodes, "\"total\"") == 26,
                  "consumer: stats nodes == 26");
            codetopo_free_string(stats);
        }
        if (err) codetopo_free_string(err);

        err = NULL;
        snapshot = codetopo_query(graph, "snapshot", "{}", &err);
        check(snapshot != NULL, "consumer: snapshot query succeeds");
        if (err) codetopo_free_string(err);
    }

    /* The snapshot loads back as a handle of its own. */
    if (snapshot) {
        err = NULL;
        codetopo_graph *loaded = codetopo_load_snapshot(snapshot, &err);
        check(loaded != NULL, "consumer: snapshot loads back");
        if (err) codetopo_free_string(err);
        if (loaded) {
            err = NULL;
            char *verify = codetopo_query(loaded, "verify", "{}", &err);
            check(verify && strstr(verify, "\"chain_valid\":true") != NULL,
                  "consumer: verify confirms the audit chain");
            if (verify) codetopo_free_string(verify);
            if (err) codetopo_free_string(err);
            codetopo_graph_free(loaded);
        }
        codetopo_free_string(snapshot);
    }
    if (graph) codetopo_graph_free(graph);

    /* A snapshot fixture from the oracle set loads directly too. */
    char *compass_json = read_file(snap_path);
    check(compass_json != NULL, "consumer: compass fixture reads");
    if (compass_json) {
        err = NULL;
        codetopo_graph *compass = codetopo_load_snapshot(compass_json, &err);
        check(compass != NULL, "consumer: compass snapshot loads");
        if (err) codetopo_free_string(err);
        if (compass) codetopo_graph_free(compass);
        free(compass_json);
    }

    /* Errors: NULL return + a JSON error object to free. */
    err = NULL;
    codetopo_graph *missing = codetopo_open_db("/nonexistent/no-such-index.db", &err);
    check(missing == NULL, "consumer: missing db returns NULL");
    check(err && strstr(err, "\"code\":\"unknown_index\"") != NULL,
          "consumer: missing db error code is unknown_index");
    if (err) codetopo_free_string(err);

    /* NULL-tolerant frees, as the header promises. */
    codetopo_free_string(NULL);
    codetopo_graph_free(NULL);
    check(1, "consumer: NULL frees are no-ops");

    printf("consumer: %d checks, %d failures\n", checks, failures);
    if (failures == 0) printf("CONSUMER PASS\n");
    return failures == 0 ? 0 : 1;
}
