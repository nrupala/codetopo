// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by [Nrupal Akolkar](https://nrupalakolkar.com) · Built with [Muse by Meta](https://muse.ai/join) — join Muse, then redeem code **101EEK** in Settings within 48 hours of joining and we both earn tokens.

# codetopo-extract

Tree-sitter extractors for **Rust** (`.rs`), **TypeScript/JavaScript**
(`.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`), and **Python**
(`.py`, `.pyi`). One parse per file, one recursive walk, emitting
[`codetopo_core::emit::FileSymbols`](../codetopo-core/src/emit.rs).

```rust
use std::path::Path;
use codetopo_extract::{extract_file, is_supported};

assert!(is_supported(Path::new("src/net.rs")));
let fs = extract_file("mycrate", Path::new("src/net.rs"), &source)?;
println!("{}", fs.module_id); // "mycrate::net"
```

## ID rules

### Module ids

**Rust** — rel path, strip leading `src/`, strip `.rs`, `/` → `::`;
`mod.rs`/`lib.rs` map to the parent; `main.rs` → `main`:

| rel path         | module_id          |
|------------------|--------------------|
| `src/net.rs`     | `{pkg}::net`       |
| `src/lib.rs`     | `{pkg}`            |
| `src/main.rs`    | `{pkg}::main`      |
| `src/a/mod.rs`   | `{pkg}::a`         |
| `src/a/b.rs`     | `{pkg}::a::b`      |
| `tests/it.rs`    | `{pkg}::tests::it` |

**TypeScript** — rel path, strip extension, `/` → `::`; trailing `/index`
collapses to the parent. Unlike Rust, a leading `src/` is **kept**:

| rel path        | module_id         |
|-----------------|-------------------|
| `src/util.ts`   | `{pkg}::src::util`|
| `src/index.ts`  | `{pkg}::src`      |
| `index.ts`      | `{pkg}`           |

**Python** — rel path, strip extension, `/` → `::`; `__init__.py`
collapses to its package (its id *is* the package). A leading `src/`
is **kept**, as in TypeScript:

| rel path            | module_id            |
|---------------------|----------------------|
| `core/graph.py`     | `{pkg}::core::graph` |
| `core/__init__.py`  | `{pkg}::core`        |
| `__init__.py`       | `{pkg}`              |
| `main.py`           | `{pkg}::main`        |
| `core/graph.pyi`    | `{pkg}::core::graph` |

### Symbol ids

`{module_id}::{item path}` with `::` nesting. Examples:

- top-level `fn foo` in `{pkg}::net` → `{pkg}::net::foo`
- `fn connect` in `impl Client` in `{pkg}::net` → `{pkg}::net::Client::connect`
- `fn f` in `mod inner` in `{pkg}::m` → `{pkg}::m::inner::f`

The `impl` block itself is **not** a symbol; its self type (`Client`,
`Client<T>` → `Client`) scopes the methods.

### Kind mapping

| Source construct | NodeKind |
|---|---|
| Rust `fn` (free) | `Function` |
| Rust `struct` / `enum` / `union` | `Class` |
| Rust `trait` | `Interface` |
| Rust `fn` in `impl` / `trait` (incl. `function_signature_item`) | `Method` |
| Rust `const` / `static` | `Variable` |
| TS `function` (incl. generators) | `Function` |
| TS `class` (incl. abstract) | `Class` |
| TS `interface` | `Interface` |
| TS `method_definition` / `method_signature` | `Method` |
| TS top-level `const` / `let` / `var` | `Variable` |
| Python `def` / `async def` (module level, or nested in a function) | `Function` |
| Python `def` directly in a class body (incl. `@property` / `@classmethod` / `@staticmethod`) | `Method` |
| Python `class` | `Class` |
| Python `class` with a `Protocol` base (plain, `typing.Protocol`, or subscripted) | `Interface` |
| Python module-level plain-name assignment | `Variable` |
| Python PEP 695 `type X = …` alias | `Class` |

Python non-symbols, by design: class-level assignments (fields are
not symbols in any language here), destructuring assignment targets
at module level, and lambdas (a lambda assigned to a name makes that
name a `Variable`; the lambda itself is not a function symbol).

### Calls

`RawCall { from_id, to_name, line }`: `from_id` is the enclosing symbol's id
(or the module id at top level); `to_name` is the callee **as written**
(`client.connect`, `TcpStream::new`, `foo`). TS `new Server()` is recorded as
a call with `to_name = "Server"` (constructor dependency for blast-radius).

### Imports

**Rust** `use` → `RawImport { from_module_id, to_module_id, bindings, line }`:

| use statement | to_module_id | bindings |
|---|---|---|
| `use crate::a::b;` | `{pkg}::a::b` | `["b"]` |
| `use crate::a::b as c;` | `{pkg}::a::b` | `["c"]` |
| `use self::x;` | `{pkg}::<modpath>::x` | `["x"]` |
| `use super::x;` | `{pkg}::<parent-modpath>::x` | `["x"]` |
| `use a::{b, c as d};` | `external::a` | `["b", "d"]` (one import) |
| `use a::*;` | `external::a` | `[]` |
| `use std::io::Read;` | `external::std` | `["Read"]` |
| `use mycrate::x;` (first segment == package) | `{pkg}::x` | `["x"]` |

`super::` past the crate root (and anything unresolvable) →
`external::unresolved`.

**TypeScript** `import … from '…'`:

| specifier | to_module_id |
|---|---|
| `'./util'` (from `src/app.ts`) | `{pkg}::src::util` |
| `'../shared/x'` (from `src/sub/m.ts`) | `{pkg}::src::shared::x` |
| `'./deep/index'` | `{pkg}::src::deep` (trailing `/index` → parent) |
| `'path'`, `'react'` (bare) | `external::path`, `external::react` |

`bindings` = the imported names (default import, `* as ns`, `{a, b as c}` →
alias wins). Resolution is purely syntactic — no filesystem access — so a
specifier that names a file with an unexpected extension still maps
lexically.

**Python** imports → `RawImport` at module granularity. The first
dotted segment decides: the package's own name is internal (the
segment *is* the package segment of the id), anything else is
`external::{first}` — the Rust bare-root rule.

| statement | to_module_id | bindings |
|---|---|---|
| `import a.b.c` | `{pkg}::a::b::c` if `a` is the package, else `external::a` | `["a"]` |
| `import a.b as d` | as above for `a.b` | `["d"]` (alias wins) |
| `from a.b import x, y as z` | module `a.b`, resolved as above | `["x", "z"]` |
| `from a.b import *` | module `a.b` | `[]` |
| `from . import x` | the importing module's package | `["x"]` |
| `from .mod import x` | package + `mod` | `["x"]` |
| `from ..pkg.mod import x` | ascend one segment, then `pkg::mod` | `["x"]` |
| `from __future__ import annotations` | `external::__future__` | `["annotations"]` |

Relative imports resolve on **module-id segments**, never file
paths: the importing module's package is its id minus the last
segment, except `__init__.py`, whose id already is its package.
Ascending past the root segment yields `external::unresolved`.

**Python calls** follow the shared rule: `to_name` is the callee
text as written (`foo`, `obj.m`, `pkg.mod.f`, `super().m`);
class instantiation is an ordinary call. Decorator calls belong to
the *enclosing* scope (they are evaluated there); a bare decorator
(`@cache`, `@x.setter`) becomes a ref on the decorated symbol.

**Python heritage** is `Inherits` only — Python has no separate
implements syntax; the Protocol distinction is carried by the
`Interface` node kind above, not by an edge. Attribute bases
contribute their attribute name (`pkg.Base` → `Base`), subscript
bases recurse to the value (`Generic[T]` → `Generic`); keyword
arguments (`metaclass=…`) and call bases are skipped.

### defines / refs

Best effort, per the marimo contract:

- **defines**: parameter names plus `let`/`const` binding names found in the
  body (patterns are destructured; only pattern-shaped tree nodes are
  descended into, so type annotations don't leak in).
- **refs**: other identifiers and type names referenced in the body, minus
  the defined names. Nested item declarations are skipped (their bindings
  belong to their own symbols); JSX subtrees are skipped (keeps tag names
  like `div` out of refs).

For Python the same contract is filled from Python's binding forms:
**defines** are parameter names (typed, defaulted, `*args`/`**kwargs`,
destructured) plus for/with-as/comprehension/walrus/except-as targets
and local assignment targets; **writes** are assignment targets (a
plain name both defines and writes; `self.x = …` writes `x`). Type
annotation names are refs. An augmented-assignment target (`x += 1`)
is recorded as a *ref*, not a write — a disclosed limit of the
schema (§9 reports it as such). `global`/`nonlocal` names are
excluded from defines; assignments to them still write.

## Error behavior

- `extract_file` returns `Err(ExtractError::UnsupportedExtension)` for
  unknown extensions — never panics on the extension.
- A file that parses **with errors** (the normal tree-sitter case for
  half-written code) is walked anyway; whatever parses cleanly is
  extracted. Only a total parse failure (no tree at all) returns
  `Err(ExtractError::ParseFailed)` so the caller can skip the file with a
  warning.
- UTF-8-hostile or structurally odd nodes are skipped, never unwrapped.

## Node kinds used (verified, not guessed)

All node-kind strings were read from the grammars' `node-types.json` in the
crates.io registry cache:

- tree-sitter-rust 0.24.2: `function_item`, `function_signature_item`,
  `struct_item`, `enum_item`, `union_item`, `trait_item`, `impl_item`,
  `mod_item`, `const_item`, `static_item`, `use_declaration`,
  `scoped_use_list`, `use_list`, `use_wildcard`, `use_as_clause`,
  `scoped_identifier`, `call_expression`, `let_declaration`, `parameters`,
  `identifier`, `type_identifier`.
- tree-sitter-typescript 0.23.2: `function_declaration`,
  `generator_function_declaration`, `class_declaration`,
  `abstract_class_declaration`, `interface_declaration`, `method_definition`,
  `method_signature`, `lexical_declaration`, `variable_declarator`,
  `import_statement`, `import_clause`, `named_imports`, `namespace_import`,
  `import_specifier`, `call_expression`, `new_expression`,
  `export_statement`, `module` (a `namespace` block), `formal_parameters`,
  `arrow_function`.
- tree-sitter-python 0.25.0: `function_definition`, `class_definition`,
  `decorated_definition`, `type_alias_statement`, `import_statement`,
  `import_from_statement`, `future_import_statement`, `call`,
  `assignment`, `parameters`, `aliased_import`, `attribute`, `block`,
  `except_clause`. (One delta against the grammar's research copy:
  `tuple_expression` exists there but not in 0.25.0; the mapping does
  not use it.)

Crate pins `tree-sitter-typescript = "0.23"` because 0.24 does not exist on
crates.io. Its 0.23 API exposes `LANGUAGE_TYPESCRIPT` / `LANGUAGE_TSX` as
`tree_sitter_language::LanguageFn`, converted with `.into()` to
`tree_sitter::Language` (supported since tree-sitter 0.25). The Python
grammar is pinned at `tree-sitter-python = "0.25"` (0.25.0, MIT) for
the same runtime pairing.

## Known limitations

1. **Rust `use foo::bar`** where `foo` is neither `std`, `crate`, `self`,
   `super`, nor the package name is classified `external::foo`. In Rust
   2018+ that spelling really is an external crate, so this is usually
   right; 2015-edition intra-crate paths are the exception.
2. **Relative TS imports resolve syntactically.** `./util` maps to
   `{pkg}::<dir>::util` without checking the file exists or which
   extension it has.
3. **Enum variants, struct fields, and TS interface members** are not
   symbols (only `method_signature` members become methods).
4. **Macros are opaque.** `macro_rules!` bodies and `macro_invocation`
   call sites are walked for plain `call_expression`s only; generated
   items are invisible.
5. **`impl Trait for Type`** methods are scoped under the self type, and
   the trait side IS recorded: the emit contract carries heritage, so the
   builder produces an `implements` edge (and `inherits` for trait
   supertraits and TS `extends`).
6. **defines/refs are heuristic.** Destructuring edge cases, `typeof`
   annotations, and identifiers inside skipped subtrees (nested items,
   JSX) may be missed or, rarely, misattributed.
7. **`column` is never set** on emitted symbols (line only, per the
   schema's minimal write path).
8. **Python duplicate definitions are diagnostics, not silence.**
   Both branches of a conditional definition are walked
   (`if TYPE_CHECKING: … else: …`, try/except ImportError fallbacks,
   platform branches, typing overload sets, and a `@property` with
   its `@x.setter`): where two definitions land on one symbol id,
   the builder keeps the first and reports a `MultiDefinition`
   diagnostic naming every location. Python code uses these
   patterns heavily, so Python indexes normally carry more of these
   diagnostics than Rust/TS ones — that is the schema working as
   designed (§5.1 of CODE_SCHEMA), not an extraction error.
9. **Python 2 syntax is not supported.** The grammar is Python 3;
   Py2 files parse with errors and the error-tolerant walk extracts
   what parses — the house behaviour for half-broken files — with
   nothing promised beyond that.
10. **Protocol detection is name-based.** A class is an `Interface`
    when a base's name is `Protocol` (plain, `typing.Protocol`, or
    subscripted). A project-local base coincidentally named
    `Protocol` would be classified the same way; conversely a
    Protocol subclassed through an alias (`P = Protocol`) is seen
    as a plain class.
11. **Dynamic Python is invisible, as in every static extractor.**
    `importlib.import_module("x")`, `__import__`, `getattr` dispatch,
    `eval`/`exec`, metaclass- or decorator-injected members, and
    monkey-patching (`C.m = f` — recorded as a write, never promoted
    to a method) do not appear; only written definitions are symbols.
    Namespace packages (PEP 420) spanning multiple roots are beyond
    the nearest-manifest package rule, for every language.
