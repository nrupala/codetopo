# Known-answer fixture — Julia

Hand-written Julia in the shape real Julia ships in: one
module per file, abstract and concrete types, a short-form
function definition, `using` imports with relative paths.
Every expectation in `../known_answer_jl.rs` is derived from
THIS document, which was written from the sources by hand —
never from extractor output.

Package `quarry`. Module ids are path-derived (the shared
rule): `Shapes.jl` → `quarry::Shapes`, `Core/Engine.jl` →
`quarry::Core::Engine`, `main.jl` → `quarry::main`. A Julia
`module` block is a class-kind symbol (a named container of
bindings; the schema has no namespace kind), and its name is
a symbol-path segment for everything inside it.

Imports are `using` / `import` statements. A dotted path
resolves on module-id segments: a leading `.` starts from the
file's directory module, each extra leading dot ascends one
segment; an absolute path whose first segment is the package
name is internal, anything else is `external::{first
segment}`. A selected import (`using P: a, b`) targets P's
module with the selected names as bindings.

## Symbols (9)

### Shapes.jl (4)

| name | kind | line |
| --- | --- | --- |
| Shapes | class (module) | 1 |
| Shape | interface (abstract type) | 3 |
| Circle | class (struct) | 5 |
| area | function (short form) | 9 |

The `radius` field is a field, not a symbol.

### Core/Engine.jl (3)

| name | kind | line |
| --- | --- | --- |
| Engine | class (module) | 1 |
| EngineState | class (mutable struct) | 5 |
| render | function | 9 |

### main.jl (2)

| name | kind | line |
| --- | --- | --- |
| state | variable | 4 |
| wheel | variable | 5 |

## Imports (3)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::Core::Engine | `using ..Shapes: Circle` | quarry::Shapes | [Circle] | 3 |
| quarry::main | `using .Shapes: Circle` | quarry::Shapes | [Circle] | 1 |
| quarry::main | `using .Core: Engine` | quarry::Core | [Engine] | 2 |

## Calls (5)

`to_name` is the callee text as written (the Kotlin rule).
Calls are recorded in walk order: an outer call precedes the
calls in its arguments.

| from | to_name | line |
| --- | --- | --- |
| quarry::Core::Engine::Engine::render | area | 11 |
| quarry::main | Engine.EngineState | 4 |
| quarry::main | Circle | 5 |
| quarry::main | println | 6 |
| quarry::main | Engine.render | 6 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | inherits | 5 |

## Measured fields (spot-pinned)

- `area` (short form): defines [c], refs [Circle] — the
  parameter and its type; `c.radius` reads only `c`, a
  define. decisions 0.
- `render`: defines [state, wheel],
  refs [Circle, EngineState, area] — parameter types and the
  called name; `state.renders += 1` reads only `state` (a
  compound assignment to a field writes nothing under the
  scan rules). writes [].
