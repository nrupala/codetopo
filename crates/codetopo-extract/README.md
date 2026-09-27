// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

# codetopo-extract

Tree-sitter extractors for **Rust** (`.rs`) and **TypeScript/JavaScript**
(`.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`). One parse per file, one
recursive walk, emitting
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

### defines / refs

Best effort, per the marimo contract:

- **defines**: parameter names plus `let`/`const` binding names found in the
  body (patterns are destructured; only pattern-shaped tree nodes are
  descended into, so type annotations don't leak in).
- **refs**: other identifiers and type names referenced in the body, minus
  the defined names. Nested item declarations are skipped (their bindings
  belong to their own symbols); JSX subtrees are skipped (keeps tag names
  like `div` out of refs).

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

Crate pins `tree-sitter-typescript = "0.23"` because 0.24 does not exist on
crates.io. Its 0.23 API exposes `LANGUAGE_TYPESCRIPT` / `LANGUAGE_TSX` as
`tree_sitter_language::LanguageFn`, converted with `.into()` to
`tree_sitter::Language` (supported since tree-sitter 0.25).

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
5. **`impl Trait for Type`** methods are scoped under the self type only;
   the trait side is not recorded (no `implements` edge in this version of
   the emit contract).
6. **defines/refs are heuristic.** Destructuring edge cases, `typeof`
   annotations, and identifiers inside skipped subtrees (nested items,
   JSX) may be missed or, rarely, misattributed.
7. **`column` is never set** on emitted symbols (line only, per the
   schema's minimal write path).
