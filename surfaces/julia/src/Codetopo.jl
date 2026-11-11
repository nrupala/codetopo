# Copyright (C) 2026 2351485 ALBERTA INC.
# SPDX-License-Identifier: AGPL-3.0-or-later
# Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
#
# Codetopo.jl — the Julia surface of codetopo.
#
# MECHANISM (Aaron's buildability proof, diaspora scoreboard §2): the Rust
# core ships as a cdylib (`libcodetopo_c`, crate `codetopo-c`) exposing a
# stable C ABI, and this module calls it directly with `ccall` from Base
# Julia (`Libdl`). There is NO other bridge: jlrs is convenience only and
# is never load-bearing, so it is not used here at all.
#
# BOUNDARY SHAPE: CODE_SCHEMA JSON strings (CODE_SCHEMA §7 — JSON is the
# universal pathway). Every answer is the same JSON the CLI, HTTP, MCP and
# C surfaces produce for the same question, because the C library serves
# queries from the same codetopo-cli library builders.
#
# JSON HANDLING — THE FALLBACK IN EFFECT: this surface was built in a
# sandbox where the Julia General registry is unreachable (Pkg cannot
# install JSON.jl; see the package README for the probe record). Per the
# build brief's fallback clause, every query/index function therefore
# returns the RAW JSON string, and callers who have JSON.jl available can
# `JSON.parse` it themselves. The only JSON this module parses is the C
# layer's small error object, via the internal extractor below.
#
# MEMORY DISCIPLINE (no leaks by construction): every `char*` the C ABI
# returns — answers and error strings alike — is copied into a Julia
# `String` with `unsafe_string` and then released with
# `codetopo_free_string`, on success and error paths alike. The two
# exceptions are the static strings from `codetopo_version()` /
# `codetopo_schema_version()`: those are copied and NEVER freed, exactly
# as the C header prescribes. Graph handles are freed exactly once:
# `close` nulls the pointer after freeing, and the finalizer calls
# `close`.

module Codetopo

using Libdl

export Graph, CodetopoError
export codetopo_version, schema_version, library_path
export index_repo, open_db, load_snapshot, load_snapshot_file, close_graph
# `index` is the brief's name for the indexing entry point; it is provided
# as an alias of `index_repo` (Julia-idiomatic, and unambiguous in Main).
export describe_schema, stats, search_symbols, get_symbol, descendants,
    ancestors, blast_radius, path, neighborhood, diagnostics, rank,
    metrics, communities, topo_sort, render, history, diff, snapshot,
    verify, query

# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------

"""
    CodetopoError(code, message)

The C layer's error, carried across the boundary. `code` is the stable
vocabulary of the existing surfaces (`unknown_symbol`, `unknown_index`,
`unsupported_schema_version`, `invalid_param`, `no_path`, `io_error`,
`store_error`, `internal`); `message` is the human-readable detail.
"""
struct CodetopoError <: Exception
    code::String
    message::String
end

Base.showerror(io::IO, e::CodetopoError) =
    print(io, "CodetopoError(", e.code, "): ", e.message)

# ---------------------------------------------------------------------------
# Library discovery + loading
# ---------------------------------------------------------------------------

const _LIB_HANDLE = Ref{Ptr{Cvoid}}(C_NULL)
const _LIB_PATH = Ref{String}("")

_lib_filename() =
    Sys.iswindows() ? "codetopo_c.dll" :
    Sys.isapple() ? "libcodetopo_c.dylib" : "libcodetopo_c.so"

"""
    library_path() -> String

Resolve the `libcodetopo_c` shared library, in order:

1. the `CODETOPO_C_LIB` environment variable (a full path) — the override
   every test run and deployment should prefer;
2. the sibling build tree: `<repo>/target/release/…` then
   `<repo>/target/debug/…` relative to this package (the workspace target
   directory of the checkout this package lives in);
3. `<package>/lib/…` (a vendored copy beside the package);
4. `Libdl.find_library` on the system search path.

Throws `ErrorException` naming every location tried when none exists.
"""
function library_path()
    env = get(ENV, "CODETOPO_C_LIB", "")
    if !isempty(env)
        isfile(env) || throw(ErrorException(
            "CODETOPO_C_LIB is set to \"$env\" but no file exists there"))
        return String(env)
    end
    name = _lib_filename()
    pkgdir = dirname(@__DIR__)            # surfaces/julia
    repodir = normpath(joinpath(pkgdir, "..", ".."))
    candidates = [
        joinpath(repodir, "target", "release", name),
        joinpath(repodir, "target", "debug", name),
        joinpath(pkgdir, "lib", name),
    ]
    for c in candidates
        isfile(c) && return c
    end
    found = Libdl.find_library(["libcodetopo_c", "codetopo_c"])
    isempty(found) || return found
    throw(ErrorException(
        "libcodetopo_c not found; set CODETOPO_C_LIB to the built " *
        "libcodetopo_c path (build it with " *
        "`cargo build -p codetopo-c --release`). Tried: " *
        join(candidates, ", ")))
end

function _ensure_lib()
    if _LIB_HANDLE[] == C_NULL
        p = library_path()
        # RTLD_LAZY per the brief; the handle is kept in a module-level
        # Ref for the life of the process so the library stays loaded.
        _LIB_HANDLE[] = Libdl.dlopen(p, Libdl.RTLD_LAZY)
        _LIB_PATH[] = p
    end
    return nothing
end

# NOTE on ccall shape: Julia forbids a ccall library expression that
# references a LOCAL variable, and Julia 1.11 wants a path string (or a
# LazyLibrary) as the library, so every call below names the library as
# the module-global `_LIB_PATH[]` (set by `_ensure_lib()`, which also
# dlopens the library with RTLD_LAZY and keeps the handle resident),
# never a local holding the path.

# ---------------------------------------------------------------------------
# C string plumbing — copy, then free with codetopo_free_string
# ---------------------------------------------------------------------------

function _free_string(ptr::Ptr{Cchar})
    ptr == C_NULL && return nothing
    _ensure_lib(); ccall((:codetopo_free_string, _LIB_PATH[]), Cvoid, (Ptr{Cchar},), ptr)
    return nothing
end

"""Copy a heap string returned by the C ABI, then free the original."""
function _take_string(ptr::Ptr{Cchar})
    ptr == C_NULL && return nothing
    try
        return unsafe_string(ptr)
    finally
        _free_string(ptr)
    end
end

"""Copy a STATIC string (version/schema version). Never freed."""
_static_string(ptr::Ptr{Cchar}) = ptr == C_NULL ? "" : unsafe_string(ptr)

# Extract one string field from the C layer's flat error object
# {"code": "...", "message": "..."} — the only JSON this module parses
# (see the fallback note at the top of this file).
function _error_field(json::AbstractString, key::AbstractString)
    m = match(Regex("\"" * key * "\"\\s*:\\s*\"((?:\\\\.|[^\"\\\\])*)\""), json)
    m === nothing && return ""
    return replace(m.captures[1], r"\\(.)" => s"\1")
end

function _throw_from_err(err_ptr::Ptr{Cchar})
    err_ptr == C_NULL &&
        throw(CodetopoError("internal", "C call failed without an error string"))
    text = _take_string(err_ptr)
    code = _error_field(text, "code")
    message = _error_field(text, "message")
    throw(CodetopoError(isempty(code) ? "internal" : code,
                        isempty(message) ? text : message))
end

# Run a fallible C call expressed as a closure `err_ref -> ptr`: on a NULL
# result, throw the C error; on success, also free a stray error string if
# the C layer set one (it contractually sets NULL, but freeing is free).
function _call_string(f::Function)
    err = Ref{Ptr{Cchar}}(C_NULL)
    ptr = f(err)::Ptr{Cchar}
    if ptr == C_NULL
        _throw_from_err(err[])
    end
    err[] != C_NULL && _free_string(err[])
    return _take_string(ptr)
end

# ---------------------------------------------------------------------------
# Versions (static strings — copied, never freed)
# ---------------------------------------------------------------------------

"""The codetopo core version this library was built against (e.g. \"0.9.1\")."""
codetopo_version() = (_ensure_lib(); _static_string(
    ccall((:codetopo_version, _LIB_PATH[]), Ptr{Cchar}, ())))

"""The CODE_SCHEMA version this build speaks (e.g. \"2.0\")."""
schema_version() = (_ensure_lib(); _static_string(
    ccall((:codetopo_schema_version, _LIB_PATH[]), Ptr{Cchar}, ())))

# ---------------------------------------------------------------------------
# Graph handle
# ---------------------------------------------------------------------------

"""
    Graph

An owned handle to a frozen codetopo graph (the C ABI's opaque
`codetopo_graph*`). Freed by the finalizer, or explicitly and idempotently
with `close` / `close_graph`. One handle per thread for concurrent use,
per the C header.
"""
mutable struct Graph
    ptr::Ptr{Cvoid}
    function Graph(ptr::Ptr{Cvoid})
        ptr == C_NULL && throw(ArgumentError("Graph handle must not be NULL"))
        g = new(ptr)
        finalizer(close_graph, g)
        return g
    end
end

"""Free a graph handle now. Idempotent; the finalizer is then a no-op."""
function close_graph(g::Graph)
    if g.ptr != C_NULL
        ptr = g.ptr
        g.ptr = C_NULL
        _ensure_lib(); ccall((:codetopo_graph_free, _LIB_PATH[]), Cvoid, (Ptr{Cvoid},), ptr)
    end
    return nothing
end

Base.close(g::Graph) = close_graph(g)
Base.isopen(g::Graph) = g.ptr != C_NULL

function _handle(g::Graph)
    g.ptr == C_NULL && throw(CodetopoError("invalid_param",
        "graph handle is closed"))
    return g.ptr
end

function _call_graph(f::Function)
    err = Ref{Ptr{Cchar}}(C_NULL)
    ptr = f(err)::Ptr{Cvoid}
    if ptr == C_NULL
        _throw_from_err(err[])
    end
    err[] != C_NULL && _free_string(err[])
    return Graph(ptr)
end

# ---------------------------------------------------------------------------
# Keyword-argument discipline (the 0.9.1 lesson, at the Julia layer too)
# ---------------------------------------------------------------------------

function _check_kwargs(kwargs, allowed, fname)
    for k in keys(kwargs)
        if !(k in allowed)
            throw(ArgumentError(
                "unknown keyword argument '$k' for $fname; " *
                "allowed keywords: " * join(sort!(string.(collect(allowed))), ", ")))
        end
    end
    return nothing
end

_kw(kwargs, key, default) = haskey(kwargs, key) ? kwargs[key] : default

# ---------------------------------------------------------------------------
# Minimal JSON writing (arguments cross the boundary as JSON objects;
# only string escaping + scalars are needed — no JSON.jl, per the fallback)
# ---------------------------------------------------------------------------

function _json_escape(s::AbstractString)
    io = IOBuffer()
    for c in s
        if c == '"'
            write(io, "\\\"")
        elseif c == '\\'
            write(io, "\\\\")
        elseif c == '\n'
            write(io, "\\n")
        elseif c == '\r'
            write(io, "\\r")
        elseif c == '\t'
            write(io, "\\t")
        elseif c < ' '
            write(io, "\\u", lpad(string(UInt32(c), base=16), 4, '0'))
        else
            write(io, c)
        end
    end
    return String(take!(io))
end

_json_value(v::AbstractString) = "\"" * _json_escape(v) * "\""
_json_value(v::Bool) = v ? "true" : "false"
_json_value(v::Integer) = string(v)
_json_value(v::AbstractFloat) = string(v)
_json_value(::Nothing) = "null"

"""Build a JSON object string from ordered key => value pairs."""
function _args_json(pairs...)
    io = IOBuffer()
    write(io, '{')
    for (i, (k, v)) in enumerate(pairs)
        i > 1 && write(io, ',')
        write(io, '"', k, "\":", _json_value(v))
    end
    write(io, '}')
    return String(take!(io))
end

# ---------------------------------------------------------------------------
# The twenty operations
# ---------------------------------------------------------------------------

"""
    index_repo(repo_path, db_path; package=nothing) -> String

Index the repository at `repo_path` into the database at `db_path`
(created/replaced, in the store format every surface opens). Returns the
index report as a raw JSON string
(`{"package","files_indexed","files_failed","nodes","edges","diagnostics"}`).
`package` defaults to the repository directory name, as on the other
surfaces. Also available under the brief's name `index`.
"""
function index_repo(repo_path::AbstractString, db_path::AbstractString; kwargs...)
    _check_kwargs(kwargs, (:package,), "index")
    package = _kw(kwargs, :package, nothing)
    _ensure_lib()
    # An empty package string is the C layer's "use the default" signal
    # (it falls back to the repository directory name), so no NULL
    # Cstring ever has to cross the boundary here.
    return _call_string() do err
        ccall((:codetopo_index, _LIB_PATH[]), Ptr{Cchar},
              (Cstring, Cstring, Cstring, Ref{Ptr{Cchar}}),
              repo_path, db_path,
              package === nothing ? "" : String(package), err)
    end
end

const index = index_repo

"""
    open_db(db_path) -> Graph

Open the index database at `db_path` and hold its graph. A missing
database throws `CodetopoError` with code `unknown_index`.
"""
function open_db(db_path::AbstractString)
    _ensure_lib()
    return _call_graph() do err
        ccall((:codetopo_open_db, _LIB_PATH[]), Ptr{Cvoid},
              (Cstring, Ref{Ptr{Cchar}}), db_path, err)
    end
end

"""
    load_snapshot(snapshot_json) -> Graph

Load a graph from a Snapshot JSON string (the portable form the
`snapshot` operation and every other surface produce). A CODE_SCHEMA
major mismatch throws `CodetopoError` with code
`unsupported_schema_version`.
"""
function load_snapshot(snapshot_json::AbstractString)
    _ensure_lib()
    return _call_graph() do err
        ccall((:codetopo_load_snapshot, _LIB_PATH[]), Ptr{Cvoid},
              (Cstring, Ref{Ptr{Cchar}}), snapshot_json, err)
    end
end

"""Load a graph from a Snapshot JSON file on disk. See `load_snapshot`."""
load_snapshot_file(path::AbstractString) = load_snapshot(read(path, String))

"""
    query(graph, op, args_json="{}") -> String

The raw C ABI query entry point: run operation `op` with the JSON-object
string `args_json`, returning the raw answer JSON. An unknown `op` throws
`CodetopoError` (`invalid_param`) naming the operation — never a silent
default. The typed wrappers below are this function with validated,
Julia-shaped arguments.
"""
function query(g::Graph, op::AbstractString, args_json::AbstractString="{}")
    _ensure_lib()
    handle = _handle(g)
    return _call_string() do err
        ccall((:codetopo_query, _LIB_PATH[]), Ptr{Cchar},
              (Ptr{Cvoid}, Cstring, Cstring, Ref{Ptr{Cchar}}),
              handle, op, args_json, err)
    end
end

_q(g, op) = query(g, op, "{}")
_q(g, op, pairs...) = query(g, op, _args_json(pairs...))

"""The CODE_SCHEMA document (node/edge kinds, tiers, metric methods). Needs a handle, per the C ABI."""
describe_schema(g::Graph; kwargs...) =
    (_check_kwargs(kwargs, (), "describe_schema"); _q(g, "describe_schema"))

"""Aggregate counts: nodes/edges totals by kind."""
stats(g::Graph; kwargs...) =
    (_check_kwargs(kwargs, (), "stats"); _q(g, "stats"))

"""Symbols whose id or label contains `query` (case-insensitive), capped at `limit`."""
function search_symbols(g::Graph, query_text::AbstractString; kwargs...)
    _check_kwargs(kwargs, (:limit,), "search_symbols")
    return _q(g, "search_symbols",
              "query" => String(query_text),
              "limit" => Int(_kw(kwargs, :limit, 20)))
end

"""The symbol dossier for node id `node`: the node, its incident edges, its measurements where measured."""
get_symbol(g::Graph, node::AbstractString; kwargs...) =
    (_check_kwargs(kwargs, (), "get_symbol");
     _q(g, "get_symbol", "node" => String(node)))

function _closure(fname, op, g, node, kwargs)
    _check_kwargs(kwargs, (:limit, :expand), fname)
    return _q(g, op,
              "node" => String(node),
              "limit" => Int(_kw(kwargs, :limit, 20)),
              "expand" => Bool(_kw(kwargs, :expand, false)))
end

"""Transitive closure over `calls` + `imports` from `node` (CODE_SCHEMA §6)."""
descendants(g::Graph, node::AbstractString; kwargs...) =
    _closure("descendants", "descendants", g, node, kwargs)

"""The reverse closure: everything that reaches `node` over `calls` + `imports`."""
ancestors(g::Graph, node::AbstractString; kwargs...) =
    _closure("ancestors", "ancestors", g, node, kwargs)

"""Dependents of `node` over the breakage-propagating edge kinds (§6)."""
blast_radius(g::Graph, node::AbstractString; kwargs...) =
    _closure("blast_radius", "blast_radius", g, node, kwargs)

"""Shortest directed typed path from `from_id` to `to_id`; `no_path` error when none exists (never a reversed-edge synthesis)."""
function path(g::Graph, from_id::AbstractString, to_id::AbstractString; kwargs...)
    _check_kwargs(kwargs, (), "path")
    return _q(g, "path",
              "from" => String(from_id), "to" => String(to_id))
end

"""The k-hop subgraph around `node` (`depth` hops, both directions)."""
function neighborhood(g::Graph, node::AbstractString; kwargs...)
    _check_kwargs(kwargs, (:depth,), "neighborhood")
    return _q(g, "neighborhood",
              "node" => String(node),
              "depth" => Int(_kw(kwargs, :depth, 1)))
end

"""Well-formedness diagnostics as data (§5). A snapshot-backed handle answers the empty list — snapshots carry no diagnostics."""
diagnostics(g::Graph; kwargs...) =
    (_check_kwargs(kwargs, (), "diagnostics"); _q(g, "diagnostics"))

"""
    rank(graph; by="blast_radius", limit=20) -> String
    rank(graph, by; limit=20) -> String

Rank nodes by one of the nine core metrics (`fan_in`, `fan_out`,
`blast_radius`, `complexity`, `pagerank`, `betweenness`, `closeness`,
`eigenvector`, `katz`). `by` is the ONLY metric keyword — any other
keyword (e.g. `metric=`) throws `ArgumentError` naming the key, the
0.9.1 discipline. A metric that cannot be measured on this graph answers
with `"measured":false` and a reason — absence, never zeros.
"""
function rank(g::Graph; kwargs...)
    _check_kwargs(kwargs, (:by, :limit), "rank")
    return _rank(g, String(_kw(kwargs, :by, "blast_radius")),
                 Int(_kw(kwargs, :limit, 20)))
end

function rank(g::Graph, by::AbstractString; kwargs...)
    _check_kwargs(kwargs, (:limit,), "rank")
    return _rank(g, String(by), Int(_kw(kwargs, :limit, 20)))
end

_rank(g, by, limit) =
    _q(g, "rank", "by" => by, "limit" => limit)

"""§9 type + package measurements, methods stated beside the numbers."""
metrics(g::Graph; kwargs...) =
    (_check_kwargs(kwargs, (), "metrics"); _q(g, "metrics"))

"""Leiden communities at resolution 1.0 with the published fixed seed, plus modularity Q (absent at m = 0)."""
function communities(g::Graph; kwargs...)
    _check_kwargs(kwargs, (:limit,), "communities")
    return _q(g, "communities",
              "limit" => Int(_kw(kwargs, :limit, 20)))
end

"""Topological groups over the SCC condensation; multi-node groups are cycles, flagged `cyclic: true`."""
topo_sort(g::Graph; kwargs...) =
    (_check_kwargs(kwargs, (), "topo_sort"); _q(g, "topo_sort"))

"""Render the graph: a layout spec (`layout="layered"` or `"force"`) as JSON."""
function render(g::Graph; kwargs...)
    _check_kwargs(kwargs, (:layout,), "render")
    return _q(g, "render",
              "layout" => String(_kw(kwargs, :layout, "layered")))
end

"""
Git-history measurements (hotspots, change coupling) from a local working
tree. Without `repo_path`, the answer is `measured:false` with its reason —
history is a local-surfaces-only input (§9).
"""
function history(g::Graph; kwargs...)
    _check_kwargs(kwargs, (:repo_path, :since, :min_support), "history")
    pairs = Pair{String,Any}[
        "min_support" => Int(_kw(kwargs, :min_support, 2))]
    repo = _kw(kwargs, :repo_path, nothing)
    repo === nothing || push!(pairs, "repo_path" => String(repo))
    since = _kw(kwargs, :since, nothing)
    since === nothing || push!(pairs, "since" => String(since))
    return _q(g, "history", pairs...)
end

"""
Diff this graph (the "before") against exactly one other source:
`other_db_path=`, `other_snapshot_json=`, or `other_snapshot_path=`.
Anything else — none, or more than one — throws `ArgumentError`.
"""
function diff(g::Graph; kwargs...)
    _check_kwargs(kwargs,
        (:other_db_path, :other_snapshot_json, :other_snapshot_path), "diff")
    given = [k for k in
        (:other_db_path, :other_snapshot_json, :other_snapshot_path)
        if haskey(kwargs, k)]
    length(given) == 1 || throw(ArgumentError(
        "diff requires exactly one other source: 'other_db_path', " *
        "'other_snapshot_json', or 'other_snapshot_path' (got $(length(given)))"))
    k = given[1]
    return _q(g, "diff", String(k) => String(kwargs[k]))
end

"""The portable Snapshot JSON of this graph, bound to its audit head."""
snapshot(g::Graph; kwargs...) =
    (_check_kwargs(kwargs, (), "snapshot"); _q(g, "snapshot"))

"""Verify the audit chain behind this graph (entries, `chain_valid`, head)."""
verify(g::Graph; kwargs...) =
    (_check_kwargs(kwargs, (), "verify"); _q(g, "verify"))

end # module Codetopo
