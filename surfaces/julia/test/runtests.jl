# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
#
# Conformance suite for the Julia surface: the shared diaspora battery
# (F1–F5) exercised through `ccall` against the real `libcodetopo_c`,
# plus the Julia layer's own strictness discipline.
#
# JSON HANDLING — FALLBACK IN EFFECT (see src/Codetopo.jl and the
# package README): JSON.jl is not a dependency of this surface in this
# build because the General registry was unreachable from the build
# sandbox, so every check below extracts values from the RAW JSON
# strings the wrappers return — the same substring-extraction technique
# the C ABI lane's tests/conformance.c uses, field-for-field.
#
# Every expected number is a known answer: the oracle goldens committed
# in crates/codetopo-core/tests/fixtures/oracle/ (produced by networkx /
# python-igraph as validation oracles), the quarry README's hand-derived
# counts, and the sibling fixture's ground truth as pinned by
# crates/codetopo-cli/tests/py_sibling_fixture.rs.
#
# Run:  CODETOPO_C_LIB=/path/to/libcodetopo_c.so \
#         julia --project=surfaces/julia surfaces/julia/test/runtests.jl

using Test
using Codetopo

const REPO_ROOT = normpath(joinpath(@__DIR__, "..", "..", ".."))
const ORACLE_DIR = joinpath(REPO_ROOT, "crates", "codetopo-core", "tests",
                            "fixtures", "oracle")
const QUARRY_DIR = joinpath(REPO_ROOT, "crates", "codetopo-extract", "tests",
                            "fixture-py")

# ---- raw-string JSON extraction helpers (mirror tests/conformance.c) ----

"""Index just past the `:` of `\"key\"` searched from `start`, or 0."""
function _value_pos(json::AbstractString, key::AbstractString,
                    start::Integer=1)
    i = findnext("\"$key\"", json, start)
    i === nothing && return 0
    j = findnext(':', json, last(i))
    j === nothing && return 0
    return j + 1
end

function _skip_ws(json::AbstractString, pos::Integer)
    while pos <= lastindex(json) && json[pos] in (' ', '\t', '\n', '\r')
        pos = nextind(json, pos)
    end
    return pos
end

"""Integer following `\"key\":`, optionally searching after `marker` first."""
function json_int_after(json::AbstractString, marker, key::AbstractString)
    start = 1
    if marker !== nothing
        i = findfirst(marker, json)
        i === nothing && return -1
        start = first(i)
    end
    pos = _value_pos(json, key, start)
    pos == 0 && return -1
    m = match(r"-?\d+", json, _skip_ws(json, pos))
    return m === nothing ? -1 : parse(Int, m.match)
end

"""Float value of the rank entry `\"id\":\"<id>\"` (its `\"value\":` field)."""
function rank_value(json::AbstractString, id::AbstractString)
    i = findfirst("\"id\":\"$id\"", json)
    i === nothing && return NaN
    pos = _value_pos(json, "value", first(i))
    pos == 0 && return NaN
    m = match(r"-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?", json, _skip_ws(json, pos))
    return m === nothing ? NaN : parse(Float64, m.match)
end

"""The segment of `a`/`b` between markers `from` and `to` is identical."""
function segment_equal(a::AbstractString, b::AbstractString,
                       from::AbstractString, to::AbstractString)
    function segment(s)
        i = findfirst(from, s); i === nothing && return nothing
        j = findnext(to, s, last(i)); j === nothing && return nothing
        return s[first(i):prevind(s, first(j))]
    end
    sa, sb = segment(a), segment(b)
    return sa !== nothing && sa == sb
end

function check_rank(g, metric, ids, want, tol; align_sign=false)
    ans = Codetopo.rank(g, metric; limit=1000)
    got = [rank_value(ans, id) for id in ids]
    if align_sign && sum(got .* want) < 0
        got = -got   # eigenvector is defined up to sign; align, then compare
    end
    for (id, gv, wv) in zip(ids, got, want)
        @test abs(gv - wv) <= tol
    end
    return ans
end

# ---------------------------------------------------------------------------

@testset "versions" begin
    @test startswith(Codetopo.codetopo_version(), "0.9")
    @test Codetopo.schema_version() == "2.0"
end

@testset "error shape" begin
    err = try
        Codetopo.open_db(joinpath(tempdir(), "no-such-index-xyz.db"))
        nothing
    catch e
        e
    end
    @test err isa CodetopoError
    @test err.code == "unknown_index"

    err2 = try
        Codetopo.load_snapshot("{\"schema_version\": \"9.0\", \"nodes\": [], \"edges\": []}")
        nothing
    catch e
        e
    end
    @test err2 isa CodetopoError
    @test err2.code == "unsupported_schema_version"
end

# ---- F2 first: build the quarry handle most other testsets use -----------

workdir = mktempdir()
quarry_db = joinpath(workdir, "quarry.db")
quarry = nothing

@testset "F2 quarry (index -> 26 nodes / 33 edges / 2 diagnostics)" begin
    # The known answer (26/33/2, the fixture README's hand-derived
    # arithmetic) is over the package's three .py files ONLY, with
    # package name "quarry" (load-bearing: any other name drops the
    # absolute import onto an external stub). Indexing the fixture
    # directory as-is also indexes its README.md as Markdown, which adds
    # exactly 3 nodes / 3 edges (29/36, diagnostics still 2) — the same
    # distinction the JS surface's conformance suite pins. Both are
    # asserted here, so neither number can drift silently.
    pkg_copy = joinpath(workdir, "quarry-pkg")
    mkpath(pkg_copy)
    for name in ("__init__.py", "core.py", "shapes.py")
        cp(joinpath(QUARRY_DIR, name), joinpath(pkg_copy, name))
    end
    report = Codetopo.index_repo(pkg_copy, quarry_db; package="quarry")
    @test json_int_after(report, nothing, "nodes") == 26
    @test json_int_after(report, nothing, "edges") == 33
    @test json_int_after(report, nothing, "diagnostics") == 2

    full_db = joinpath(workdir, "quarry-full.db")
    full_report = Codetopo.index_repo(QUARRY_DIR, full_db; package="quarry")
    @test json_int_after(full_report, nothing, "nodes") == 29
    @test json_int_after(full_report, nothing, "edges") == 36
    @test json_int_after(full_report, nothing, "diagnostics") == 2

    global quarry = Codetopo.open_db(quarry_db)
    st = Codetopo.stats(quarry)
    @test json_int_after(st, "\"nodes\"", "total") == 26
    @test json_int_after(st, "\"edges\"", "total") == 33
    dg = Codetopo.diagnostics(quarry)
    @test json_int_after(dg, nothing, "total") == 2
end

@testset "surface smoke — the twenty operations answer" begin
    g = quarry
    @test occursin("package", Codetopo.describe_schema(g))
    @test occursin("Engine", Codetopo.search_symbols(g, "Engine"))
    @test occursin("quarry::core::Engine",
                   Codetopo.get_symbol(g, "quarry::core::Engine"))
    @test !isempty(Codetopo.descendants(g, "quarry"))
    @test !isempty(Codetopo.ancestors(g, "quarry::core::Engine"))
    @test !isempty(Codetopo.blast_radius(g, "quarry::core::Engine"))
    @test !isempty(Codetopo.neighborhood(g, "quarry::core::Engine"; depth=1))
    @test !isempty(Codetopo.metrics(g))
    @test !isempty(Codetopo.communities(g))
    @test !isempty(Codetopo.topo_sort(g))
    @test startswith(Codetopo.render(g; layout="layered"), "{")
    # No working tree supplied: history is absent, honestly.
    @test occursin("\"measured\":false", Codetopo.history(g))
    @test occursin("\"chain_valid\":true", Codetopo.verify(g))
    snap = Codetopo.snapshot(g)
    @test occursin("\"schema_version\":\"2.0\"", snap)
    d = Codetopo.diff(g; other_snapshot_json=snap)
    @test startswith(d, "{")
    # Unknown op errors through the raw hatch, naming the op.
    err = try
        Codetopo.query(g, "no_such_op", "{}")
        nothing
    catch e
        e
    end
    @test err isa CodetopoError
    @test err.code == "invalid_param"
    @test occursin("no_such_op", err.message)
    # Unknown symbol errors with the shared vocabulary.
    err2 = try
        Codetopo.get_symbol(g, "quarry::nope::Missing")
        nothing
    catch e
        e
    end
    @test err2 isa CodetopoError
    @test err2.code == "unknown_symbol"
end

@testset "strictness — unknown kwargs throw ArgumentError naming the key" begin
    g = quarry
    err = try
        Codetopo.rank(g; metric="pagerank")
        nothing
    catch e
        e
    end
    @test err isa ArgumentError
    @test occursin("metric", err.msg)

    err2 = try
        Codetopo.stats(g; bogus=1)
        nothing
    catch e
        e
    end
    @test err2 isa ArgumentError
    @test occursin("bogus", err2.msg)

    # The C layer's own rank discipline, through the raw hatch: a
    # non-`by` metric key errors naming the key (the F3 pin, here too).
    err3 = try
        Codetopo.query(g, "rank", "{\"metric\": \"pagerank\"}")
        nothing
    catch e
        e
    end
    @test err3 isa CodetopoError
    @test err3.code == "invalid_param"
    @test occursin("metric", err3.message)
end

# ---- F1: oracle goldens ---------------------------------------------------

@testset "F1 oracle goldens (compass)" begin
    @test isfile(joinpath(ORACLE_DIR, "compass.golden.json"))
    g = Codetopo.load_snapshot(read(joinpath(ORACLE_DIR, "compass.json"), String))
    ids = ["a", "b", "c", "d", "e", "f", "g", "h"]
    check_rank(g, "pagerank", ids,
        [0.050366533824645295, 0.0717724799737871, 0.0717724799737871,
         0.14187699226038397, 0.17096200118402893, 0.1956833061815353,
         0.21669618663194073, 0.08087001996989152], 1e-6)
    check_rank(g, "betweenness", ids,
        [0.0, 0.07142857142857142, 0.047619047619047616,
         0.21428571428571427, 0.19047619047619047, 0.11904761904761904,
         0.0, 0.0], 1e-9)
    check_rank(g, "closeness", ids,
        [0.0, 0.14285714285714285, 0.14285714285714285,
         0.3214285714285714, 0.2857142857142857, 0.27472527472527475,
         0.2706766917293233, 0.19047619047619047], 1e-9)
    check_rank(g, "katz", ids,
        [0.3182844374972241, 0.35011288124694656, 0.35011288124694656,
         0.3883070137466134, 0.35711513887188545, 0.3539959513844127,
         0.35368403263566545, 0.3532957256219188], 1e-6)
    # Eigenvector on a DAG is ABSENT (nilpotent adjacency): measured:false
    # with a reason, never zeros.
    eig = Codetopo.rank(g; by="eigenvector")
    @test occursin("\"measured\":false", eig)
    close(g)
end

@testset "F1 oracle goldens (ring)" begin
    @test isfile(joinpath(ORACLE_DIR, "ring.golden.json"))
    g = Codetopo.load_snapshot(read(joinpath(ORACLE_DIR, "ring.json"), String))
    ids = ["w", "x", "y", "z"]
    check_rank(g, "pagerank", ids,
        [0.2813632148631319, 0.276658031180747, 0.15508018934150383,
         0.28689856461461727], 1e-6)
    check_rank(g, "betweenness", ids, [0.5, 0.5, 0.0, 0.5], 1e-9)
    check_rank(g, "closeness", ids, [0.6, 0.5, 0.5, 0.75], 1e-9)
    check_rank(g, "eigenvector", ids,
        [0.5262015076917937, 0.43104970112724544, 0.3531051686825981,
         0.6423588314257299], 1e-6; align_sign=true)
    check_rank(g, "katz", ids,
        [0.4909351618601384, 0.48655572748762066, 0.486117784050369,
         0.5347295493315389], 1e-6)
    close(g)
end

@testset "F1 oracle goldens (cycle-trap topo-sort, exact)" begin
    g = Codetopo.load_snapshot(
        read(joinpath(ORACLE_DIR, "cycle-trap.json"), String))
    topo = Codetopo.topo_sort(g)
    @test json_int_after(topo, nothing, "total_groups") == 4
    @test json_int_after(topo, nothing, "total_nodes") == 6
    @test json_int_after(topo, nothing, "cyclic_groups") == 1
    @test occursin("\"cyclic\":true", topo)
    ilib = findfirst("\"lib\"", topo)
    ix = findfirst("\"x\"", topo)
    imid = findfirst("\"mid\"", topo)
    iapp = findfirst("\"app\"", topo)
    @test ilib !== nothing && ix !== nothing && imid !== nothing &&
          iapp !== nothing
    @test first(ilib) < first(ix) < first(imid) < first(iapp)
    close(g)
end

# ---- F3: Aarona sibling tripwire ------------------------------------------

@testset "F3 Aarona sibling tripwire" begin
    repo = joinpath(workdir, "sibling", "repo")
    mkpath(repo)
    files = [
        "a.py" => "import b\nimport c\n\n\ndef run():\n    b.work()\n    c.work()\n",
        "b.py" => "import c\n\n\ndef work():\n    c.work()\n",
        "c.py" => "import d\n\n\ndef work():\n    d.work()\n",
        "d.py" => "import e\n\n\ndef work():\n    e.work()\n",
        "e.py" => "import a\nimport c\n\n\ndef work():\n    a.run()\n    c.work()\n",
    ]
    for (name, body) in files
        write(joinpath(repo, name), body)
    end
    db = joinpath(workdir, "sibling", "sibling.db")
    Codetopo.index_repo(repo, db; package="repo")
    g = Codetopo.open_db(db)

    st = Codetopo.stats(g)
    @test json_int_after(st, "\"nodes\"", "total") == 16
    @test json_int_after(st, "\"edges\"", "imports") == 7
    @test json_int_after(st, "\"edges\"", "calls") == 7
    @test !occursin("\"external\"", st)

    topo = Codetopo.topo_sort(g)
    @test json_int_after(topo, nothing, "cyclic_groups") == 2

    # A directed path inside the cycle exists and is found.
    p = Codetopo.path(g, "repo::a::run", "repo::d::work")
    @test occursin("repo::d::work", p)

    # rank with a `metric` key throws at the Julia layer, naming the key.
    err = try
        Codetopo.rank(g; metric="pagerank")
        nothing
    catch e
        e
    end
    @test err isa ArgumentError
    @test occursin("metric", err.msg)
    close(g)
end

# ---- F4 lifecycle + F5 determinism ----------------------------------------

@testset "F4 lifecycle (graph content, verify chain)" begin
    snap1 = Codetopo.snapshot(quarry)
    loaded = Codetopo.load_snapshot(snap1)
    v = Codetopo.verify(loaded)
    @test occursin("\"chain_valid\":true", v)
    snap2 = Codetopo.snapshot(loaded)
    # Snapshots embed wall-clock audit timestamps: byte-identity is the
    # known lifecycle flake, so compare GRAPH CONTENT (nodes + edges
    # payloads) and let `verify` check the chain (mirrors conformance.c).
    # Segment markers follow this boundary's serialization: the C layer
    # emits serde_json::Value, whose keys are alphabetically sorted
    # (audit_head, audit_log, edges, nodes, schema_version).
    @test segment_equal(snap1, snap2, "\"edges\":", "\"nodes\":")
    @test segment_equal(snap1, snap2, "\"nodes\":", "\"schema_version\"")
    close(loaded)
end

@testset "F5 determinism (repeated queries identical)" begin
    g = Codetopo.load_snapshot(read(joinpath(ORACLE_DIR, "compass.json"), String))
    r1 = Codetopo.rank(g; by="pagerank", limit=1000)
    r2 = Codetopo.rank(g; by="pagerank", limit=1000)
    @test r1 == r2
    close(g)
    s1 = Codetopo.stats(quarry)
    s2 = Codetopo.stats(quarry)
    @test s1 == s2
end

close(quarry)
