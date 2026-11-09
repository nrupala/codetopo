// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by 2351485 ALBERTA INC. (https://nrupalakolkar.com)
//
// codetopo — C++ face.
//
// A header-only C++17 wrapper over the codetopo C ABI
// (crates/codetopo-c/include/codetopo.h, cdylib `libcodetopo_c`).
// The C ABI is the shared foundation every language face stands on;
// this face consumes that ABI directly. It does NOT use the `cxx`
// crate: `cxx` is an alternative bridge technology that would create
// a second, separately maintained boundary, while the house
// foundation — the substrate the R and Julia faces also stand on —
// is the C ABI. One foundation, one conformance story.
//
// BOUNDARY SHAPE
//   Exactly the C ABI's shape: CODE_SCHEMA JSON strings in and out
//   (CODE_SCHEMA §7 — JSON is the universal pathway). Every answer
//   this wrapper returns is the same JSON the CLI, HTTP, MCP, and C
//   surfaces produce for the same question, as a std::string. There
//   are no C++ structs mirroring the graph schema; the schema
//   travels as JSON and evolves under CODE_SCHEMA versioning.
//
// JSON POLICY — NO THIRD-PARTY JSON DEPENDENCY
//   This face depends on nothing beyond the C++ standard library
//   and the shipped C header. That is a deliberate choice, not an
//   omission: the wrapper never needs to *interpret* an answer —
//   answers pass through as strings for the caller to parse with
//   whatever JSON library their own project already carries. The
//   one object the wrapper itself must read is the small error
//   object ({"code","message"}), and it reads that with the
//   targeted field extraction in namespace detail below — a few
//   lines, fully visible here, instead of a dependency.
//
// OWNERSHIP
//   - codetopo::Graph is the RAII owner of a `codetopo_graph*`:
//     move-only, its destructor calls codetopo_graph_free().
//   - codetopo::CString is the RAII owner of a `char*` returned by
//     the C ABI: its destructor calls codetopo_free_string().
//   - codetopo_version() / codetopo_schema_version() return static
//     strings owned by the library; version() / schema_version()
//     copy them into std::string. Never free the originals.
//
// ERRORS
//   Every fallible C call that returns NULL becomes a thrown
//   codetopo::Error carrying the error object's parsed `code`
//   (the existing surfaces' vocabulary: unknown_symbol,
//   unknown_index, unsupported_schema_version, invalid_param,
//   no_path, io_error, store_error, internal) and `message`.
//   Querying through an empty (default-constructed or moved-from)
//   Graph throws Error with code "invalid_param" without calling
//   into the library at all.
//
// OPERATIONS
//   Indexing and opening are the free functions index(), open_db(),
//   load_snapshot(). Everything else is Graph::query(op, args_json)
//   with the C ABI's operation names: describe_schema, stats,
//   search_symbols, get_symbol, descendants, ancestors,
//   blast_radius, path, neighborhood, diagnostics, rank, metrics,
//   communities, topo_sort, render, history, diff, snapshot, verify.
//   An unknown op is an invalid_param error from the library, never
//   a silent default; rank's metric key is `by` only — any other
//   key errors, naming the key (the 0.9.1 discipline).

#ifndef CODETOPO_HPP
#define CODETOPO_HPP

#include "codetopo.h"

#include <stdexcept>
#include <string>
#include <utility>

namespace codetopo {

/// A codetopo failure, carried across the C++ boundary as an
/// exception. `code()` is the stable machine-readable code from
/// the C ABI's error object; `message()` (and `what()`) is its
/// human-readable detail.
class Error : public std::runtime_error {
public:
    Error(std::string code, std::string message)
        : std::runtime_error(message.empty() ? code : message),
          code_(std::move(code)),
          message_(std::move(message)) {}

    const std::string& code() const noexcept { return code_; }
    const std::string& message() const noexcept { return message_; }

private:
    std::string code_;
    std::string message_;
};

/// RAII owner of a `char*` returned by the C ABI (a query/index
/// answer or an error string). Frees via codetopo_free_string() —
/// the only correct release for those pointers. Move-only.
class CString {
public:
    CString() noexcept = default;
    explicit CString(char* ptr) noexcept : ptr_(ptr) {}
    ~CString() { codetopo_free_string(ptr_); }

    CString(const CString&) = delete;
    CString& operator=(const CString&) = delete;

    CString(CString&& other) noexcept : ptr_(other.ptr_) { other.ptr_ = nullptr; }
    CString& operator=(CString&& other) noexcept {
        if (this != &other) {
            codetopo_free_string(ptr_);
            ptr_ = other.ptr_;
            other.ptr_ = nullptr;
        }
        return *this;
    }

    /// The wrapped C string, or "" when empty. Borrowed — valid
    /// only while this guard lives.
    const char* c_str() const noexcept { return ptr_ ? ptr_ : ""; }
    /// A copy of the wrapped string as a std::string.
    std::string str() const { return ptr_ ? std::string(ptr_) : std::string(); }
    bool empty() const noexcept { return ptr_ == nullptr; }
    /// Hand the pointer to the caller (who must then release it
    /// with codetopo_free_string()); this guard becomes empty.
    char* release() noexcept {
        char* ptr = ptr_;
        ptr_ = nullptr;
        return ptr;
    }

private:
    char* ptr_ = nullptr;
};

namespace detail {

/// Extract the string value of `key` from a small flat JSON object
/// by targeted search — the only JSON reading this wrapper does
/// (see the JSON policy in the file header). Handles the standard
/// backslash escapes serde_json emits. Returns "" when absent.
inline std::string json_string_field(const std::string& json, const char* key) {
    const std::string pattern = std::string("\"") + key + "\"";
    std::size_t pos = json.find(pattern);
    if (pos == std::string::npos) return std::string();
    pos = json.find(':', pos + pattern.size());
    if (pos == std::string::npos) return std::string();
    ++pos;
    while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
    if (pos >= json.size() || json[pos] != '"') return std::string();
    ++pos;
    std::string out;
    while (pos < json.size()) {
        const char c = json[pos++];
        if (c == '"') break;
        if (c == '\\' && pos < json.size()) {
            const char esc = json[pos++];
            switch (esc) {
                case 'n': out.push_back('\n'); break;
                case 't': out.push_back('\t'); break;
                case 'r': out.push_back('\r'); break;
                case '"': out.push_back('"'); break;
                case '\\': out.push_back('\\'); break;
                case '/': out.push_back('/'); break;
                default: out.push_back(esc); break;
            }
        } else {
            out.push_back(c);
        }
    }
    return out;
}

/// Build an Error from a C ABI error string, taking ownership of
/// it (freed here via the CString guard). A missing error string
/// still yields a well-formed Error — failure is never silent.
inline Error error_from(char* err_raw) {
    if (!err_raw) {
        return Error("internal", "operation failed without an error message");
    }
    const CString guard(err_raw);
    const std::string text = guard.str();
    std::string code = json_string_field(text, "code");
    std::string message = json_string_field(text, "message");
    if (code.empty()) code = "internal";
    if (message.empty()) message = text;
    return Error(code, message);
}

/// The success/failure fork shared by every string-returning C
/// call: NULL answer -> throw the parsed error; otherwise return
/// the answer as a std::string (freed via the guard). A stray
/// err_out set on success is freed, never leaked.
inline std::string take_answer(char* raw, char* err_raw) {
    if (!raw) throw error_from(err_raw);
    if (err_raw) codetopo_free_string(err_raw);
    const CString guard(raw);
    return guard.str();
}

/// The same fork for handle-returning calls.
inline codetopo_graph* take_handle(codetopo_graph* raw, char* err_raw) {
    if (!raw) throw error_from(err_raw);
    if (err_raw) codetopo_free_string(err_raw);
    return raw;
}

}  // namespace detail

/// RAII owner of a graph handle. Move-only; the destructor
/// releases the handle with codetopo_graph_free(). Handles are
/// not shared between threads for concurrent queries — use one
/// handle per thread (the C ABI's guarantee).
class Graph {
public:
    Graph() noexcept = default;
    ~Graph() { reset(); }

    Graph(const Graph&) = delete;
    Graph& operator=(const Graph&) = delete;

    Graph(Graph&& other) noexcept : handle_(other.handle_) { other.handle_ = nullptr; }
    Graph& operator=(Graph&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = other.handle_;
            other.handle_ = nullptr;
        }
        return *this;
    }

    /// Open the index database at `db_path`. Throws Error
    /// (code "unknown_index") when no database exists there.
    static Graph open_db(const std::string& db_path);
    /// Load a graph from a Snapshot JSON string. Throws Error
    /// (code "unsupported_schema_version") on a CODE_SCHEMA major
    /// mismatch.
    static Graph load_snapshot(const std::string& snapshot_json);

    /// Run one query operation (see the operation list in the
    /// file header). `args_json` is a JSON object string; the
    /// default "{}" serves the no-argument operations. Returns
    /// the answer JSON. Throws Error on any failure.
    std::string query(const std::string& op, const std::string& args_json = "{}") const {
        if (!handle_) {
            throw Error("invalid_param", "query on an empty Graph handle");
        }
        char* err = nullptr;
        char* raw = codetopo_query(handle_, op.c_str(), args_json.c_str(), &err);
        return detail::take_answer(raw, err);
    }

    bool valid() const noexcept { return handle_ != nullptr; }
    explicit operator bool() const noexcept { return valid(); }
    /// Borrow the raw handle (e.g. to interop with the C ABI
    /// directly). Ownership stays with this Graph.
    codetopo_graph* native_handle() const noexcept { return handle_; }
    /// Hand the raw handle to the caller, who must then release
    /// it with codetopo_graph_free(); this Graph becomes empty.
    codetopo_graph* release() noexcept {
        codetopo_graph* handle = handle_;
        handle_ = nullptr;
        return handle;
    }
    /// Release the handle now; this Graph becomes empty.
    void reset() noexcept {
        if (handle_) {
            codetopo_graph_free(handle_);
            handle_ = nullptr;
        }
    }

private:
    explicit Graph(codetopo_graph* handle) noexcept : handle_(handle) {}
    codetopo_graph* handle_ = nullptr;
};

inline Graph Graph::open_db(const std::string& db_path) {
    char* err = nullptr;
    codetopo_graph* raw = codetopo_open_db(db_path.c_str(), &err);
    return Graph(detail::take_handle(raw, err));
}

inline Graph Graph::load_snapshot(const std::string& snapshot_json) {
    char* err = nullptr;
    codetopo_graph* raw = codetopo_load_snapshot(snapshot_json.c_str(), &err);
    return Graph(detail::take_handle(raw, err));
}

/// Version of the codetopo core this library was built against
/// (e.g. "0.9.0"). A copy — the C ABI's static string is never
/// freed and never exposed.
inline std::string version() {
    const char* value = codetopo_version();
    return value ? std::string(value) : std::string();
}

/// CODE_SCHEMA version this build speaks (e.g. "1.2").
inline std::string schema_version() {
    const char* value = codetopo_schema_version();
    return value ? std::string(value) : std::string();
}

/// Index the repository at `repo_path` into the database at
/// `db_path` (created/replaced, in the store format every surface
/// opens). `package` may be nullptr, in which case the repository
/// directory name is used, as on the other surfaces. Returns the
/// index report JSON ({"package","files_indexed","files_failed",
/// "nodes","edges","diagnostics"}). Throws Error on failure.
inline std::string index(const std::string& repo_path, const std::string& db_path,
                         const char* package = nullptr) {
    char* err = nullptr;
    char* raw = codetopo_index(repo_path.c_str(), db_path.c_str(), package, &err);
    return detail::take_answer(raw, err);
}

/// index() with the package name as a std::string.
inline std::string index(const std::string& repo_path, const std::string& db_path,
                         const std::string& package) {
    return index(repo_path, db_path, package.c_str());
}

/// Free-function spelling of Graph::open_db.
inline Graph open_db(const std::string& db_path) { return Graph::open_db(db_path); }

/// Free-function spelling of Graph::load_snapshot.
inline Graph load_snapshot(const std::string& snapshot_json) {
    return Graph::load_snapshot(snapshot_json);
}

}  // namespace codetopo

#endif  // CODETOPO_HPP
