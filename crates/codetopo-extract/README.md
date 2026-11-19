// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by [Nrupal Akolkar](https://nrupalakolkar.com) · Built with [Muse by Meta](https://muse.ai/join).

# codetopo-extract

Tree-sitter extractors for **Rust** (`.rs`), **TypeScript/JavaScript**
(`.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`), **Python**
(`.py`, `.pyi`), **Kotlin** (`.kt`, `.kts`), **C** (`.c`, `.h`),
**Java** (`.java`), **Go** (`.go`), **Shell** (`.sh`, `.bash`),
**C++** (`.cpp`, `.hpp`, `.cc`, `.cxx`, `.hh`, `.hxx`), and
**Swift** (`.swift`), **Ruby** (`.rb`), **R** (`.R`, `.r`),
and **Julia** (`.jl`), and **C#** (`.cs`). One parse per file, one recursive walk, emitting
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

**Kotlin** — rel path, strip extension, `/` → `::` (the Python
rule; no collapse — Kotlin has no `__init__` form):

| rel path            | module_id            |
|---------------------|----------------------|
| `Main.kt`           | `{pkg}::Main`        |
| `core/Engine.kt`    | `{pkg}::core::Engine` |
| `shapes/Circle.kt`  | `{pkg}::shapes::Circle` |

**C** — rel path, strip extension, `/` → `::` (the Python rule).
A header and its implementation share a stem, hence a module —
`shapes/circle.h` and `shapes/circle.c` are the same module, so a
prototype and its definition land on one symbol id (the builder
keeps the first file's record and reports the rest as multiple
definitions, as it does for any duplicate):

| rel path            | module_id            |
|---------------------|----------------------|
| `main.c`            | `{pkg}::main`        |
| `core/engine.c`     | `{pkg}::core::engine` |
| `core/engine.h`     | `{pkg}::core::engine` |

**Java** — the `package` declaration plus the file stem. A Java
type's identity is its package plus its name; source-tree layout
(`src/main/java/…`) is not part of that identity, so the path
beyond the file name does not enter the id. A file with no
package declaration falls back to the path rule above:

| rel path (package)              | module_id            |
|---------------------------------|----------------------|
| `shapes/Shape.java` (`quarry.shapes`) | `quarry::shapes::Shape` |
| `core/Engine.java` (`quarry.core`)    | `quarry::core::Engine`  |
| `Main.java` (`quarry`)                | `quarry::Main`          |

**Go** — the package name plus the directory segments. All
files in a directory form one Go package, so there is no file
stem in the id — and an import path, which IS a module path,
resolves to exactly this id:

| rel path            | module_id            |
|---------------------|----------------------|
| `main.go`           | `{pkg}`              |
| `shapes/circle.go`  | `{pkg}::shapes`      |
| `core/engine.go`    | `{pkg}::core`        |

**Shell** — rel path, strip extension, `/` → `::` (the Python
rule):

| rel path            | module_id            |
|---------------------|----------------------|
| `main.sh`           | `{pkg}::main`        |
| `lib/util.sh`       | `{pkg}::lib::util`   |
| `core/engine.sh`    | `{pkg}::core::engine` |

**C++** — rel path, strip extension, `/` → `::` (the C rule;
a header and its implementation share a module). Namespace
names are symbol-path segments, not module segments:

| rel path            | module_id            |
|---------------------|----------------------|
| `shapes/Circle.hpp` | `{pkg}::shapes::Circle` |
| `shapes/Circle.cpp` | `{pkg}::shapes::Circle` |
| `core/Engine.cpp`   | `{pkg}::core::Engine` |

**Swift** — rel path, strip extension, `/` → `::` (the Python
rule):

| rel path            | module_id            |
|---------------------|----------------------|
| `main.swift`        | `{pkg}::main`        |
| `Shapes/Circle.swift` | `{pkg}::Shapes::Circle` |
| `Core/Engine.swift`   | `{pkg}::Core::Engine`   |

**Ruby** — rel path, strip extension, `/` → `::` (the Python
rule):

| rel path            | module_id            |
|---------------------|----------------------|
| `main.rb`           | `{pkg}::main`        |
| `lib/circle.rb`     | `{pkg}::lib::circle` |
| `core/engine.rb`    | `{pkg}::core::engine` |

**R** — rel path, strip extension, `/` → `::` (the Python
rule):

| rel path            | module_id            |
|---------------------|----------------------|
| `main.R`            | `{pkg}::main`        |
| `lib/shapes.R`      | `{pkg}::lib::shapes` |
| `core/engine.R`     | `{pkg}::core::engine` |

**Julia** — rel path, strip extension, `/` → `::` (the
Python rule):

| rel path            | module_id            |
|---------------------|----------------------|
| `main.jl`           | `{pkg}::main`        |
| `Shapes.jl`         | `{pkg}::Shapes`      |
| `Core/Engine.jl`    | `{pkg}::Core::Engine` |

**C#** — rel path, strip extension, `/` → `::` (the Python
rule). Namespace names are symbol-path segments (a
file-scoped namespace prefixes the whole file):

| rel path            | module_id            |
|---------------------|----------------------|
| `Program.cs`        | `{pkg}::Program`     |
| `Shapes/Circle.cs`  | `{pkg}::Shapes::Circle` |
| `Core/Engine.cs`    | `{pkg}::Core::Engine` |

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
| Kotlin `fun` (top level, or nested in a function) | `Function` |
| Kotlin `fun` directly in a class / object body | `Method` |
| Kotlin `class` / `object` / companion object | `Class` |
| Kotlin `interface` | `Interface` |
| Kotlin top-level `val` / `var` property | `Variable` |
| C function definition and prototype declaration | `Function` |
| C `struct` / `union` / `enum` definition, `typedef` name | `Class` |
| C enumerator | `Variable` (scoped under its enum) |
| C file-scope variable, object-like macro with a value | `Variable` |
| C function-like macro | `Function` |

C non-symbols, by design: struct/union fields (fields are not
symbols in any language here), and value-less `#define`s (the
include-guard form — a flag, not a binding).

| Java `class` / `enum` / `record` | `Class` |
| Java `interface` / annotation type | `Interface` |
| Java method / constructor | `Method` |
| Java enum constant | `Variable` (scoped under its enum) |

Java non-symbols, by design: fields and record components
(fields are not symbols in any language here).

| Go function | `Function` |
| Go method | `Method` (scoped under its receiver type) |
| Go struct / other named type / type alias | `Class` |
| Go interface | `Interface` |
| Go interface method element | `Method` |
| Go package-level `var` / `const` | `Variable` |

Go non-symbols, by design: struct fields (fields are not
symbols in any language here).

| Shell function | `Function` |
| Shell top-level variable assignment | `Variable` |

Shell has no other binding forms — no types, no classes.

| C++ function (unqualified, namespace/file scope) | `Function` |
| C++ method (in-class, or qualified out-of-line) | `Method` |
| C++ `class` / `struct` / `union` / `enum` definition, `typedef` name, `using` alias | `Class` |
| C++ enumerator | `Variable` (scoped under its enum) |
| C++ file/namespace-scope variable, object-like macro with a value | `Variable` |
| C++ function-like macro | `Function` |

C++ non-symbols, by design: fields (fields are not symbols in
any language here), and value-less `#define`s. A qualified
name's last-but-one segment is treated as the type (a
namespace function defined out-of-line with full qualification
is therefore read as a method of a like-named type — the
qualified form is, in practice, the member form). Template
declarations are transparent: the declared name is the symbol,
parameters are not part of the id.

| Swift `class` / `struct` / `actor` / `enum` | `Class` |
| Swift `protocol` | `Interface` |
| Swift function outside a type / inside a type | `Function` / `Method` |
| Swift initializer / deinitializer / subscript | `Method` |
| Swift protocol requirement | `Method` |
| Swift enum entry | `Variable` (scoped under its enum) |
| Swift top-level `let` / `var` | `Variable` |
| Swift `typealias` | `Class` |

Swift non-symbols, by design: properties inside a type
(fields are not symbols in any language here). An `extension`
emits no symbol of its own: it opens a scope under the
extended type's name so its methods nest there, and its
conformances are heritage rows for that type.

| Ruby `module` / `class` | `Class` |
| Ruby method inside a type / outside | `Method` / `Function` |
| Ruby singleton method (`def self.x`, `def Foo.x`) | `Method` |
| Ruby top-level assignment | `Variable` |

Ruby non-symbols, by design: class-level assignments (fields
are not symbols in any language here — a class constant is a
field by this rule).

| R function assignment (`f <- function(…)`) | `Function` |
| R top-level assignment | `Variable` |

R has no class-definition syntax in this grammar (S3/S4/R6
types are built by calls), so there are no Class symbols and
no heritage — both empty by construction.

| Julia `module` block | `Class` |
| Julia `struct` / primitive type | `Class` |
| Julia abstract type | `Interface` |
| Julia function (long or short form) | `Function` |
| Julia function on a qualified name (`Base.show(…)`) | `Method` |
| Julia top-level assignment | `Variable` |

Julia non-symbols, by design: struct fields (fields are not
symbols in any language here).

| C# `class` / `struct` / `record` | `Class` |
| C# `interface` | `Interface` |
| C# method / constructor | `Method` |
| C# enum member | `Variable` (scoped under its enum) |
| C# top-level statement variable | `Variable` |

C# non-symbols, by design: properties and fields (fields are
not symbols in any language here).

Kotlin non-symbols, by design: class-level properties and
primary-constructor `val`/`var` parameters (fields are not symbols
in any language here), secondary constructors (their bodies walk
under the class scope), and `typealias` declarations (no alias
mapping exists in the code schema for them).

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

**Kotlin** imports → `RawImport` at module granularity, resolved
by the same dotted rule (first segment is the package name →
internal, else `external::{first}`):

| statement | to_module_id | bindings |
|---|---|---|
| `import a.b.C` | `{pkg}::a::b::C` if `a` is the package, else `external::a` | `["C"]` (last segment — what Kotlin binds) |
| `import a.b.C as D` | as above for `a.b.C` | `["D"]` (alias wins) |
| `import a.b.*` | module `a.b`, resolved as above | `[]` |

**Kotlin calls** follow the shared rule: `to_name` is the callee
text as written (`foo`, `shape.area`, `Engine` for a constructor
call). **Kotlin heritage** is `Inherits` only, from delegation
specifiers (`class C : Shape, Closeable()` contributes `Shape` and
`Closeable` — the last segment of each specifier's type).

**C** includes → `RawImport`, resolved on module-id segments
relative to the including file's directory:

| statement | to_module_id | bindings |
|---|---|---|
| `#include "circle.h"` (from `shapes/circle.c`) | `{pkg}::shapes::circle` | `[]` |
| `#include "../shapes/circle.h"` (from `core/engine.h`) | `{pkg}::shapes::circle` | `[]` |
| `#include <math.h>` | `external::math` | `[]` |
| `#include <sys/types.h>` | `external::sys` | `[]` |

Bindings are always empty — a C include binds no names. Ascending
past the root with `..` yields `external::unresolved`. **C calls**
follow the shared rule: `to_name` is the callee text as written
(`foo`, `obj.m`, `(*fp)`). **C heritage** is empty — C has no
inheritance syntax; the section is empty by construction.

**Java** imports → `RawImport`. An import is internal when its
first segment equals the importing file's own package root;
then the target is the imported type's module id (package
segments + type name). Anything else is `external::{first}`:

| statement (from a `quarry.core` file) | to_module_id | bindings |
|---|---|---|
| `import quarry.shapes.Circle;` | `quarry::shapes::Circle` | `["Circle"]` |
| `import java.util.List;` | `external::java` | `["List"]` |
| `import static quarry.core.Engine.MAX;` | `quarry::core::Engine` | `["MAX"]` |
| `import quarry.shapes.*;` | `quarry::shapes` | `[]` |

**Java calls** follow the shared rule with the Python
attribute form: `to_name` is the invoked method's name
(`wheel.area()` → `area`); object creation is a call on the
type name (`new Circle(…)` → `Circle`). **Java heritage**:
superclass and interface-extends are `Inherits`; implemented
interfaces are `Implements`.

**Go** imports → `RawImport`: the import path IS the module
path (`/` → `::`), internal when its first segment is the
package name, else `external::{first segment}`:

| statement | to_module_id | bindings |
|---|---|---|
| `import "quarry/shapes"` | `quarry::shapes` | `["shapes"]` |
| `import "fmt"` | `external::fmt` | `["fmt"]` |
| `import alias "quarry/shapes"` | `quarry::shapes` | `["alias"]` |
| `import . "quarry/util"` | `quarry::util` | `[]` |
| `import _ "quarry/blank"` | `quarry::blank` | `[]` |

**Go calls** follow the shared rule: `to_name` is the called
function's name — for a selector call, the field (`e.Wheel.Area()`
→ `Area`). **Go heritage**: an interface embedding another
interface is `Inherits`. Struct embedding is NOT heritage — it
is composition; method promotion is a call-resolution concern,
not a type-hierarchy edge.

**Shell** imports are the `source` / `.` command — shell's
only import mechanism. A command by that name whose first
argument is a literal path is recorded as an import (not a
call), resolved on module-id segments relative to the sourcing
file, like a C include:

| statement (from `core/engine.sh`) | to_module_id | bindings |
|---|---|---|
| `. "../lib/util.sh"` | `{pkg}::lib::util` | `[]` |
| `source lib/more.sh` | `{pkg}::core::lib::more` | `[]` |

**Shell calls**: every other command is a call, `to_name` the
command name as written. **Shell heritage** is empty — shell
has no type system.

**C++** includes follow the C rule exactly. `using`
declarations are also imports: the qualifier path is the
module, the last segment the binding (a path whose first
segment is the package name is already a module path; any
other path resolves under the package):

| statement | to_module_id | bindings |
|---|---|---|
| `using quarry::shapes::Circle;` | `quarry::shapes` | `["Circle"]` |
| `using namespace quarry::core;` | `quarry::core` | `[]` |

**C++ calls** follow the C rule: `to_name` is the callee text
as written. **C++ heritage**: base-class clauses are
`Inherits` — C++ has one inheritance relation.

**Swift** imports name external modules — the language has
no in-package import syntax — so every import is
`external::{first segment}` with the last segment as binding:

| statement | to_module_id | bindings |
|---|---|---|
| `import Foundation` | `external::Foundation` | `["Foundation"]` |

**Swift calls** follow the shared rule: `to_name` is the
callee text as written (`Engine()`, `wheel.area()`).
**Swift heritage**: inheritance specifiers are `Inherits`,
for conformances and superclasses alike — Swift has one
inheritance list.

**Ruby** imports are the `require` / `require_relative`
calls — a call by either name whose first argument is a
string literal is recorded as an import, not a call.
`require_relative` resolves relative to the requiring file
like a C include; `require` resolves the same way when its
first segment is the package name, else
`external::{first segment}`:

| statement (from `core/engine.rb`) | to_module_id | bindings |
|---|---|---|
| `require_relative "../lib/circle"` | `{pkg}::lib::circle` | `[]` |
| `require "json"` | `external::json` | `[]` |

**Ruby calls**: `to_name` is the called method's name (the
receiver is not part of it — the Python attribute rule).
**Ruby heritage**: a class's superclass is `Inherits`.

**R** imports are calls: `library(x)` / `require(x)` load a
package (`external::{name}`, the name as binding), and
`source("p")` includes a file, resolved relative to the
sourcing file like a C include. A call by any of those names
with a literal first argument is an import, not a call:

| statement (from `core/engine.R`) | to_module_id | bindings |
|---|---|---|
| `library(stats)` | `external::stats` | `["stats"]` |
| `source("core/engine.R")` (from `main.R`) | `{pkg}::core::engine` | `[]` |

**R calls**: `to_name` is the called name — for `pkg::fn`
the function name, for `obj$fn` the member name. **R
heritage** is empty (no inheritance syntax).

**Julia** imports are `using` / `import` statements. A
dotted path resolves on module-id segments: a leading `.`
starts from the file's directory module and each extra dot
ascends one segment; an absolute path whose first segment
is the package name is internal, anything else is
`external::{first segment}`. A selected import binds the
selected names:

| statement (from `Core/Engine.jl`) | to_module_id | bindings |
|---|---|---|
| `using ..Shapes: Circle` | `{pkg}::Shapes` | `["Circle"]` |
| `using LinearAlgebra` | `external::LinearAlgebra` | `[]` |

**Julia calls** follow the shared rule: `to_name` is the
callee text as written. **Julia heritage**: a type head's
`<:` supertype is `Inherits`.

**C#** imports are `using` directives. A using is internal
when its first segment is the file's own namespace root, or
matches the package name case-insensitively — the root
segment is read as the package name, so the target is the
package plus the remaining segments. A plain using binds
nothing; an alias using binds the alias; a static using
binds the type name:

| statement | to_module_id | bindings |
|---|---|---|
| `using System;` | `external::System` | `[]` |
| `using Quarry.Shapes;` | `{pkg}::Shapes` | `[]` |
| `using Foo = Quarry.Core.Engine;` | `{pkg}::Core::Engine` | `["Foo"]` |
| `using static System.Math;` | `external::System` | `["Math"]` |

**C# calls** follow the shared rule: `to_name` is the
callee text as written; object creation is a call on the
type name. **C# heritage**: in a base list, for a class,
struct, or record the first type is `Inherits` and the
rest are `Implements`; for an interface all are `Inherits`.

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

For Kotlin the same contract is filled from Kotlin's binding
forms: **defines** are parameter names (function, lambda, `for`,
and `catch` parameters) plus local property names and plain
assignment targets; **writes** are assignment targets and local
property names (a plain name both defines and writes; `obj.x = …`
writes `x`). Type names (parameter, property, and return types)
are refs. An augmented-assignment target (`x += 1`) is recorded
as a *ref*, not a write — the same disclosed limit as Python.
Only the root of a navigation chain is a read (`shape.area()`
reads `shape`, never `area`).

For C the same contract is filled from C's binding forms:
**defines** are parameter names plus local declaration names and
plain assignment targets; **writes** are assignment targets and
initialized declaration names (a plain name both defines and
writes; `obj.field = …` writes `field`). Type names (return type,
parameter types, declaration types) are refs. An
augmented-assignment target (`x += 1`) is recorded as a *ref*,
not a write — the same disclosed limit as Python. Only the
argument of a field expression is a read (`obj.field` reads
`obj`, never `field`).

For Java the same contract is filled from Java's binding forms:
**defines** are parameter names plus local variable names and
plain assignment targets; **writes** are assignment targets and
initialized local names (a plain name both defines and writes;
`obj.field = …` writes `field`). Type names (return type,
parameter and local declaration types) are refs, and an invoked
method's name is a ref (the Python call rule). An
augmented-assignment target (`x += 1`) is recorded as a *ref*,
not a write — the same disclosed limit as Python. Only the
object of a field access is a read (`obj.field` reads `obj`,
never `field`). A symbol's line is its declaration node's first
line — for an annotated method, the annotation's line, since
the annotation is part of the node.

For Go the same contract is filled from Go's binding forms:
**defines** are receiver, parameter, and named-result names
plus short-var and local `var`/`const` names and plain
assignment targets; **writes** are assignment targets and
initialized declaration names (a plain name both defines and
writes; `obj.field = …` writes `field`). Type names are refs.
An augmented-assignment target (`x += 1`) and an `x++` target
are recorded as *refs*, not writes — the same disclosed limit
as Python. Only the operand of a selector is a read
(`e.Wheel.Area()` reads `e`); composite-literal keys are field
names, not reads. The blank identifier `_` is never a define.

For Shell the same contract is filled from shell's binding
forms: **defines** are assignment targets, `local`/`declare`
names, and `for` variables; **writes** are assignment targets
(a plain name both defines and writes; an array-element target
`arr[i] = …` writes `arr`). A command's name is a ref (the
called-name rule, as in Python). Expansions of named variables
(`$x`, `${x}`) are refs; positional and special parameters
(`$1`, `$@`, `$?`, …) are not.

For C++ the scan is the C scan plus member initializers: a
field initializer (`radius_(radius)`) writes its field, and
its arguments are reads scanned with the body.

For Swift the same contract is filled from Swift's binding
forms: **defines** are parameter names, local `let`/`var`
names, `for` item names, and plain assignment targets;
**writes** are assignment targets and initialized local names
(a plain name both defines and writes; `obj.field = …` writes
`field`). Type names are refs. An augmented-assignment target
(`x += 1`) is recorded as a *ref*, not a write — the same
disclosed limit as Python. Only the target of a navigation is
a read (`wheel.area()` reads `wheel`, never `area`). In this
grammar (tree-sitter-swift 0.7) a function's return type rides
a second `name` field, and a pattern's bound names are its
simple identifiers (`bound_identifier` is a field name, not a
node kind) — the walker reads both accordingly.

For Ruby the same contract is filled from Ruby's binding
forms: **defines** are parameter names (positional, keyword,
splat, block) plus block parameters and plain assignment
targets; **writes** are assignment targets (a plain name both
defines and writes; `obj.attr = …` writes `attr`;
instance/class/global variables write under their full
written names, sigils included). Constants and variables are
refs; an augmented-assignment target (`x += 1`, `@x += 1`) is
recorded as a *ref*, not a write — the same disclosed limit
as Python. Only a call's receiver is a read; a bare call's
name is a ref, as a plain-name call is in Python.

For R the same contract is filled from R's binding forms:
**defines** are parameter names plus assignment targets
(`<-`, `<<-`, `=`, and the reversed `->` forms); **writes**
are assignment targets (a plain name both defines and
writes). Only the left side of `$`/`@` and of `::` is a read,
and an argument's name (`list(wheel = 1)`) is not a read. A
nested function definition is its own symbol, pruned from
the enclosing scan.

For Julia the same contract is filled from Julia's binding
forms: **defines** are parameter names plus assignment
targets; **writes** are assignment targets (a plain name
both defines and writes). Only the value of a field
expression is a read (`c.radius` reads `c`); a compound
assignment's target (`x += 1`) is read, not written — the
same disclosed limit as Python.

For C# the scan is the Java scan's form: **defines** are
parameter names, local variable names, and plain assignment
targets; **writes** are assignment targets and initialized
local names (a plain name both defines and writes;
`obj.Field = …` writes `Field`). Type names and invoked
method names are refs. An augmented-assignment target is
recorded as a *ref*, not a write — the same disclosed limit
as Python. Only the expression of a member access is a read.

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
- tree-sitter-kotlin-ng 1.1.0: `source_file`, `import`,
  `qualified_identifier`, `class_declaration`, `object_declaration`,
  `companion_object`, `delegation_specifiers`, `function_declaration`,
  `function_value_parameters`, `function_body`,
  `property_declaration`, `variable_declaration`,
  `call_expression`, `navigation_expression`, `assignment`,
  `for_statement`, `catch_block`, `lambda_literal`,
  `when_expression`, `binary_expression`.
- tree-sitter-c 0.24.2: `translation_unit`, `function_definition`,
  `declaration`, `function_declarator`, `init_declarator`,
  `struct_specifier`, `union_specifier`, `enum_specifier`,
  `enumerator`, `type_definition`, `preproc_include`,
  `preproc_def`, `preproc_function_def`, `call_expression`,
  `assignment_expression`, `field_expression`,
  `parameter_declaration`.
- tree-sitter-java 0.23.5: `program`, `package_declaration`,
  `import_declaration`, `class_declaration`,
  `interface_declaration`, `enum_declaration`, `enum_constant`,
  `record_declaration`, `annotation_type_declaration`,
  `method_declaration`, `constructor_declaration`,
  `formal_parameter`, `method_invocation`,
  `object_creation_expression`, `assignment_expression`,
  `field_access`, `local_variable_declaration`,
  `variable_declarator`.
- tree-sitter-go 0.25.0: `source_file`, `package_clause`,
  `import_declaration`, `import_spec`, `function_declaration`,
  `method_declaration`, `type_declaration`, `type_spec`,
  `struct_type`, `interface_type`, `method_elem`, `type_elem`,
  `field_declaration`, `var_declaration`, `const_declaration`,
  `call_expression`, `selector_expression`,
  `assignment_statement`, `short_var_declaration`,
  `inc_statement`, `composite_literal`, `qualified_type`.
- tree-sitter-bash 0.25.1: `program`, `function_definition`,
  `command`, `command_name`, `variable_assignment`,
  `declaration_command`, `simple_expansion`, `expansion`,
  `variable_name`, `if_statement`, `for_statement`,
  `while_statement`, `case_statement`, `test_command`,
  `command_substitution`, `list`.
- tree-sitter-cpp 0.23.4: `translation_unit`,
  `namespace_definition`, `class_specifier`, `struct_specifier`,
  `enum_specifier`, `function_definition`, `field_declaration`,
  `declaration`, `type_definition`, `alias_declaration`,
  `using_declaration`, `qualified_identifier`,
  `base_class_clause`, `preproc_include`, `call_expression`,
  `assignment_expression`, `field_expression`,
  `field_initializer`, `parameter_declaration`.
- tree-sitter-swift 0.7.4: `source_file`, `import_declaration`,
  `class_declaration`, `protocol_declaration`,
  `protocol_function_declaration`, `function_declaration`,
  `init_declaration`, `property_declaration`, `enum_entry`,
  `typealias_declaration`, `call_expression`,
  `navigation_expression`, `assignment`, `pattern`,
  `inheritance_specifier`, `user_type`.
- tree-sitter-ruby 0.23.1: `program`, `module`, `class`,
  `method`, `singleton_method`, `singleton_class`,
  `assignment`, `operator_assignment`, `call`,
  `scope_resolution`, `constant`, `identifier`,
  `instance_variable`, `if`, `elsif`, `case`, `when`, `while`,
  `until`, `for`, `binary`, `method_parameters`,
  `block_parameters`.
- tree-sitter-r 1.3.0: `program`, `function_definition`,
  `parameter`, `binary_operator`, `call`, `argument`,
  `extract_operator`, `namespace_operator`, `if_statement`,
  `for_statement`, `while_statement`, `repeat_statement`,
  `identifier`, `string`.
- tree-sitter-julia 0.23.1: `source_file`,
  `module_definition`, `struct_definition`,
  `abstract_definition`, `primitive_definition`, `type_head`,
  `function_definition`, `signature`, `assignment`,
  `call_expression`, `field_expression`,
  `compound_assignment_expression`, `using_statement`,
  `import_statement`, `import_path`, `selected_import`,
  `if_statement`, `for_statement`, `while_statement`,
  `binary_expression`, `identifier`.
- tree-sitter-c-sharp 0.24.0: `compilation_unit`,
  `using_directive`, `namespace_declaration`,
  `file_scoped_namespace_declaration`, `class_declaration`,
  `interface_declaration`, `struct_declaration`,
  `record_declaration`, `enum_declaration`,
  `method_declaration`, `constructor_declaration`,
  `property_declaration`, `field_declaration`,
  `enum_member_declaration`, `invocation_expression`,
  `member_access_expression`, `object_creation_expression`,
  `assignment_expression`, `variable_declaration`,
  `variable_declarator`, `parameter`, `qualified_name`.

Crate pins `tree-sitter-typescript = "0.23"` because 0.24 does not exist on
crates.io. Its 0.23 API exposes `LANGUAGE_TYPESCRIPT` / `LANGUAGE_TSX` as
`tree_sitter_language::LanguageFn`, converted with `.into()` to
`tree_sitter::Language` (supported since tree-sitter 0.25). The Python
grammar is pinned at `tree-sitter-python = "0.25"` (0.25.0, MIT) for
the same runtime pairing. The Kotlin grammar is pinned at
`tree-sitter-kotlin-ng = "1.1"` (1.1.0, MIT): the upstream
`tree-sitter-kotlin` crate pins tree-sitter <0.23, whose `links`
value cannot coexist with tree-sitter 0.25 in one dependency graph;
the `-ng` fork of the same grammar carries the modern
`tree-sitter-language` binding. The C grammar is pinned at
`tree-sitter-c = "0.24"` (0.24.2, MIT) for the same runtime
pairing. The Java grammar is pinned at `tree-sitter-java =
"0.23"` (0.23.5, MIT) for the same runtime pairing. The Go
grammar is pinned at `tree-sitter-go = "0.25"` (0.25.0, MIT)
for the same runtime pairing. The Shell grammar is pinned at
`tree-sitter-bash = "0.25"` (0.25.1, MIT) for the same runtime
pairing. The C++ grammar is pinned at `tree-sitter-cpp =
"0.23"` (0.23.4, MIT) for the same runtime pairing. `.h` files
are extracted as C — one extension, one extractor; C++ headers
use the C++ extensions above. The Swift grammar is pinned at
`tree-sitter-swift = "0.7"` (0.7.4, MIT) for the same runtime
pairing. The Ruby grammar is pinned at `tree-sitter-ruby =
"0.23"` (0.23.1, MIT) for the same runtime pairing. The R
grammar is pinned at `tree-sitter-r = "1"` (1.3.0, MIT) for
the same runtime pairing. The Julia grammar is pinned at
`tree-sitter-julia = "0.23"` (0.23.1, MIT) for the same
runtime pairing. The C# grammar is pinned at
`tree-sitter-c-sharp = "0.24"` (0.24.0, MIT) for the same
runtime pairing.

## The RA extractor (domain-profile build, Stage 5)

Alongside the code extractors, this crate carries the Research
Analyst tenant extractor (`src/ra.rs`, with `src/xlsx.rs`,
`src/xmlmini.rs`, `src/eval.rs`, `src/siblings.rs`): it reads a
financial-model workbook (XLSX) plus a declaration and emits a graph
under `profiles/ra.profile.json`. RA is **analysis-only** — the
extractor measures a model's structure and arithmetic; nothing in
it recommends, scores, or advises on any security.

**Declaration (`codetopo.ra-decl/1`, JSON).** The workbook alone
does not say what its cells MEAN, so kind, label, period, and unit
are declaration facts, never guessed: the declaration names every
sheet (in workbook order), every non-caption cell (kind, label,
optional period/unit/`presented_as` tag, optional source block),
the articulation identities to execute, the scenarios (driver
bindings), the estimates edges (method/window/error), and the
references edges (optional claimant). Extraction refuses a workbook
the declaration does not account for exactly — an undeclared
non-caption cell, or a declared cell the workbook lacks, is an
error, not a guess.

**Evaluator bounds (`src/eval.rs`).** Values are recomputed from
the workbook's own formulas by a bounded evaluator — the cached
display values stored in the file NEVER enter evaluation. Grammar:
numbers, cell refs (same-sheet and sheet-qualified, `$` anchors
stripped), ranges as function arguments only, `+ − * / % ^`, unary
signs, parentheses, comparisons inside `IF` conditions only.
Functions: `SUM`, `AVERAGE`, `MIN`, `MAX`, `ABS`, `ROUND`, `SQRT`,
`IF` (lazy branches), `NPV` (first value discounted one period).
Anything outside the set — `XNPV` and `IRR` deliberately included —
is an unknown function and the cell's value is **absent**:
`not_measured`, never zero, never estimated. A formula reading a
cell the workbook does not contain records an `UnresolvedCall`
finding and invents no edge; a circular reference yields no values
for its cycle and the core reports the SCC as a `Cycle` diagnostic.

**Absence rules.** A measured field that could not be measured is
absent from the node — absence is the answer, never a fabricated
number. The same law governs provenance: a `sourced_from` edge
missing `source` or `as_of` is REFUSED (`MissingProvenance`, the
cell keeps its value, the edge does not enter); an `estimates` edge
missing method, window, or error is refused the same way; market
data enters only through a declared source under the profile's data
doctrine (Stooq is the production price source of truth; there is
no silent fallback to another vendor — a gap in the series is an
absent price, dated as itself).

**Identities execute (format gap F-2, closed).** The three
articulation identities — balance-sheet equation, net-income
rollforward, cash articulation — run over the emitted graph's
measured values. A failure is a `Diagnostic::ArticulationIdentity`
carrying BOTH sides and the tolerance; nothing is repaired,
averaged, or silently passed. A side with no measured value is
neither passed nor failed. Identity right-hand sides are signed
sums of cell refs (`bs!B11+is!C10-cf!C7`): the rollforward
subtracts by definition.

**Builder decisions recorded at Stage 5.** F-3: the builder gained
public `add_node` (register one tenant node verbatim),
`add_boundary_stub` (the boundary-stub entry point over the
previously private `ensure_external`), `add_contains`, and
`record_diagnostic` (the tenant-diagnostic seam) — all additive,
CODE-profile-neutral. F-1: `references` splits per edge (claimant →
`declared_claim`; none → found reference, `extracted_truth`),
adjudicated by `edge_provenance_class`; admission honours the
split because the core's per-edge requirement
(`provenance_required_attrs_for_edge`) treats a kind-dictionary
attr declared OPTIONAL as a distinguisher, not a mandate — while
required attrs (`source`/`as_of`, `method`/`window`/`error`) keep
their full refusal force.

**Statistics siblings (`src/siblings.rs`).** `sensitivity`
re-evaluates the model's own arithmetic under a one-sided step
(0.01 for rates, 1% of value otherwise) and reports the finite
difference, the elasticity, and the provenance class of any
non-truth edge on the pair's path; `scenario_spread` reports each
output's values across the declared scenarios with range and
signed deviation from base — never a confidence interval, never a
target. Both are proven against hand-derived oracles by the RA
Regression Contract (`tests/ra_contract.rs`, clauses C-RA1…C-RA8;
fixtures and their hand arithmetic in `tests/fixtures/ra/`).

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
