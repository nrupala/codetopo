# Known-answer fixture — Ruby

Hand-written Ruby in the shape real Ruby ships in: modules
wrapping classes, `require_relative` between files, instance
variables for state. Every expectation in
`../known_answer_rb.rs` is derived from THIS document, which
was written from the sources by hand — never from extractor
output.

Package `quarry`. Module ids are path-derived (the shared
rule): `lib/circle.rb` → `quarry::lib::circle`, `main.rb` →
`quarry::main`. Ruby modules and classes are both class-kind
symbols (a Ruby module is a named container of methods and
constants; the schema has no namespace kind), and their names
are symbol-path segments.

Ruby's import mechanism is the `require` / `require_relative`
call: a call by either name whose first argument is a string
literal is an **import** (recorded as an import, not a call).
`require_relative` resolves on module-id segments relative to
the requiring file, like a C include; `require` resolves the
same way when its first segment is the package name, and is
`external::{first segment}` otherwise. Bindings are empty.

## Symbols (13)

### lib/shape.rb (3)

| name | kind | line |
| --- | --- | --- |
| Quarry | class (module) | 1 |
| Shape | class | 2 |
| area | method | 3 |

### lib/circle.rb (4)

| name | kind | line |
| --- | --- | --- |
| Quarry | class (module) | 3 |
| Circle | class | 4 |
| initialize | method | 7 |
| area | method | 11 |

### core/engine.rb (4)

| name | kind | line |
| --- | --- | --- |
| Quarry | class (module) | 3 |
| Engine | class | 4 |
| initialize | method | 7 |
| render | method | 11 |

`MAX_RENDERS = 4` inside the class body is a class-level
assignment — a field by the schema's rule, not a symbol (the
Python precedent).

### main.rb (2)

| name | kind | line |
| --- | --- | --- |
| engine | variable | 3 |
| wheel | variable | 4 |

## Imports (3)

| from | to | line |
| --- | --- | --- |
| quarry::lib::circle | quarry::lib::shape | circle.rb:1 |
| quarry::core::engine | quarry::lib::circle | engine.rb:1 |
| quarry::main | quarry::core::engine | main.rb:1 |

## Calls (7)

`to_name` is the called method's name (the receiver is not
part of it — the Python attribute rule). Calls are recorded
in walk order: an outer call precedes the calls in its
arguments.

| from | to_name | line |
| --- | --- | --- |
| quarry::lib::shape::Quarry::Shape::area | raise | 4 |
| quarry::lib::circle::Quarry::Circle | attr_reader | 5 |
| quarry::core::engine::Quarry::Engine::render | area | 13 |
| quarry::main | new | 3 |
| quarry::main | new | 4 |
| quarry::main | puts | 5 |
| quarry::main | render | 5 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | inherits | 4 |

## Measured fields (spot-pinned)

- `Shape.area`: defines [], refs [NotImplementedError, raise]
  — a bare call's name is a ref (the Python rule), and a
  constant argument is a ref. decisions 0.
- `Circle.initialize`: defines [radius], writes [@radius],
  refs [] — the right-hand `radius` is the parameter, a
  define; an instance-variable target is a write of its full
  name, sigil included.
- `Circle.area`: refs [@radius].
- `Engine.initialize`: defines [], writes [@renders].
- `Engine.render`: defines [wheel], refs [@renders] — the
  augmented `@renders += 1` reads the variable; `wheel.area`
  reads only `wheel`, a define. writes [].
