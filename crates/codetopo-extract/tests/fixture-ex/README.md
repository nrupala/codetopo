# Known-answer fixture — Elixir

Hand-written Elixir in the shape real Elixir ships in: one
`defmodule` per file named for its module, a behaviour with
a `@callback`, a protocol with an implementation, `alias` /
`import` / `require` / `use` between files, a module
attribute for shared state. Every expectation in
`../known_answer_ex.rs` is derived from THIS document, which
was written from the sources by hand — never from extractor
output.

Package `quarry`. Module ids are path-derived (the shared
rule): `Shapes/Circle.ex` → `quarry::Shapes::Circle`,
`Core/Engine.ex` → `quarry::Core::Engine`, `main.exs` →
`quarry::main`. A `defmodule` name is a symbol-path segment
list (the PHP namespace rule): `defmodule
Quarry.Shapes.Circle` in module `quarry::Shapes::Circle`
gives the module symbol the id
`quarry::Shapes::Circle::Quarry::Shapes::Circle`, and its
functions nest under it. A `defprotocol` name is the same;
a `defimpl P, for: T` symbol's segments are P's segments
followed by T's segments — the name of the implementation
module Elixir itself synthesises (`P.T`).

Symbols (§3.2): a `defmodule` is a class-kind symbol (a
named container of functions; the schema has no namespace
kind — the Ruby/Julia module rule); a `defprotocol` is an
interface; a `defimpl` is a class; `def` / `defp` inside a
module are methods, outside are functions. `defstruct`
defines the enclosing module's struct fields — fields are
not symbols (the struct takes the module's own name; the
Ruby class-assignment / Julia field precedent), so a
`defstruct` contributes no symbol and its field names
(`radius`, `renders`) appear nowhere below. A `@callback`
is a behaviour contract declaration, not a function: no
symbol, no call.

Imports are the `alias` / `import` / `require` / `use`
calls. A dotted module name resolves on segments: a first
segment matching the package name case-insensitively is
internal (the segment is read as the package name, so the
target is the package plus the remaining segments);
anything else is `external::{first segment}`. Bindings:
`alias` binds its last segment (or its `as:` name);
`require` binds its last segment (it makes the module's
macros available under that name); `import` binds the
`only:` function names, and nothing without an `only:`;
`use` binds nothing (it injects code, it names nothing).

Calls (§3.4 — the called function's name; for a remote
call `Module.fun` the receiver is not part of it, the
Python attribute rule). The declaration calls
(`defmodule`, `def`, `defp`, `defprotocol`, `defimpl`,
`defstruct`, `alias`, `import`, `require`, `use`) and the
control-form calls (`if`, `unless`, `case`, `cond`, `for`,
`with`, `try`, `receive`) are not calls. Calls are
recorded in walk order: an outer call precedes the calls
in its arguments.

Heritage: a `@behaviour M` inside a module is that module
implementing M (Implements); a `defimpl P, for: T` is T
implementing P (Implements). Elixir has no inheritance.

The defines/refs/writes scan (§3.6's marimo discipline,
Elixir form: nested declarations pruned, reads that are
also defines dropped, a match `=` target is a define and a
write, only the receiver of a remote call is a read — a
bare local call's name is a ref, as a plain-name call is in
Python; a module attribute read `@name` is a ref under its
full written name, `@` included; `__MODULE__` is never a
ref, the `$this` rule). Decisions (§3.2's enumeration,
Elixir form: `if` / `unless` +1, `case` / `cond`
contribute clauses − 1, each `&&` / `||` / `and` / `or`
+1).

## Symbols (14)

### Shapes/Shape.ex (1)

| name | kind | line |
| --- | --- | --- |
| Shape | class (defmodule) | 1 |

The `@callback` on line 2 declares the behaviour's
contract: no symbol, no call.

### Shapes/Circle.ex (4)

| name | kind | line |
| --- | --- | --- |
| Circle | class (defmodule) | 1 |
| new | method | 8 |
| area | method | 12 |
| scale | method (defp) | 16 |

The `defstruct` on line 4 and the `@pi` attribute set on
line 6 are fields / module state, not symbols.

### Shapes/Drawable.ex (4)

| name | kind | line |
| --- | --- | --- |
| Drawable | interface (defprotocol) | 1 |
| draw | method (protocol, no body) | 2 |
| Circle | class (defimpl Drawable for Circle) | 5 |
| draw | method (in the defimpl) | 6 |

### Core/Engine.ex (3)

| name | kind | line |
| --- | --- | --- |
| Engine | class (defmodule) | 1 |
| new | method | 11 |
| render | method | 15 |

The `defstruct` on line 9 is a field declaration, not a
symbol.

### main.exs (2)

| name | kind | line |
| --- | --- | --- |
| engine | variable | 4 |
| wheel | variable | 5 |

## Imports (7)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::Core::Engine | `use GenServer` | external::GenServer | [] | 2 |
| quarry::Core::Engine | `alias Quarry.Shapes.Circle` | quarry::Shapes::Circle | [Circle] | 4 |
| quarry::Core::Engine | `alias Quarry.Shapes.Drawable` | quarry::Shapes::Drawable | [Drawable] | 5 |
| quarry::Core::Engine | `import Quarry.Shapes.Circle, only: [area: 1]` | quarry::Shapes::Circle | [area] | 6 |
| quarry::Core::Engine | `require Logger` | external::Logger | [Logger] | 7 |
| quarry::main | `alias Quarry.Core.Engine` | quarry::Core::Engine | [Engine] | 1 |
| quarry::main | `alias Quarry.Shapes.Circle` | quarry::Shapes::Circle | [Circle] | 2 |

## Calls (8)

| from | to_name | line |
| --- | --- | --- |
| quarry::Shapes::Drawable::Quarry::Shapes::Drawable::Quarry::Shapes::Circle::draw | inspect | 7 |
| quarry::Core::Engine::Quarry::Core::Engine::render | info | 17 |
| quarry::Core::Engine::Quarry::Core::Engine::render | area | 20 |
| quarry::Core::Engine::Quarry::Core::Engine::render | draw | 20 |
| quarry::main | new | 4 |
| quarry::main | new | 5 |
| quarry::main | puts | 6 |
| quarry::main | render | 6 |

The `if` on Engine.ex line 15 is a control form, not a
call. `area(wheel)` on line 20 is the imported local call;
`Drawable.draw(wheel)` on the same line is a remote call
whose receiver is not part of `to_name`.

## Heritage (2)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | implements | 2 |
| Circle | Drawable | implements | 5 |

## Measured fields (spot-pinned)

- `Circle.new`: defines [radius], refs [], writes [] —
  the `%__MODULE__{}` construction reads only its field
  values; `__MODULE__` is never a ref. decisions 0.
- `Circle.area`: defines [radius] (bound by the struct
  pattern in the head), refs [@pi] — the module attribute
  read, under its full written name. decisions 0.
- `Circle.scale`: defines [factor, radius], refs [].
- `Drawable.draw` (protocol): defines [shape], refs [],
  decisions none — a protocol function has no body.
- `Drawable` impl `draw`: defines [circle], refs [IO] —
  only the receiver of `IO.inspect` is a read. decisions 0.
- `Engine.new`: defines [], refs [], decisions 0.
- `Engine.render`: defines [engine, wheel],
  refs [Drawable, Logger, area] — the two remote calls'
  receivers and the bare local call's name; `engine` in
  the `if` condition is a define. writes [].
  decisions 2 — the `if` (+1) and its `&&` (+1).
- `main` variables: `engine` refs [Engine], `wheel` refs
  [Circle] — the receivers of their initialiser calls.
