# Known-answer fixture — Swift

Hand-written Swift in the shape real Swift ships in: a
protocol, a conforming struct, a class, and a top-level entry
file. Every expectation in `../known_answer_swift.rs` is
derived from THIS document, which was written from the sources
by hand — never from extractor output.

Package `quarry`. Module ids are path-derived (the shared
rule): `Shapes/Shape.swift` → `quarry::Shapes::Shape`,
`Core/Engine.swift` → `quarry::Core::Engine`, `main.swift` →
`quarry::main`.

Swift imports name external modules — the language has no
in-package import syntax — so every import is
`external::{first segment}` with the last segment as binding.

## Symbols (9)

### Shapes/Shape.swift (2)

| name | kind | line |
| --- | --- | --- |
| Shape | interface (protocol) | 1 |
| area | method (protocol requirement, no body) | 2 |

### Shapes/Circle.swift (2)

| name | kind | line |
| --- | --- | --- |
| Circle | class (struct) | 1 |
| area | method | 4 |

The `radius` property is a field, not a symbol.

### Core/Engine.swift (2)

| name | kind | line |
| --- | --- | --- |
| Engine | class | 3 |
| render | method | 6 |

The `renders` property is a field, not a symbol.

### main.swift (3)

| name | kind | line |
| --- | --- | --- |
| engine | variable | 1 |
| wheel | variable | 2 |
| area | variable | 3 |

## Imports (1)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::Core::Engine | `import Foundation` | external::Foundation | [Foundation] | 1 |

## Calls (5)

`to_name` is the callee text as written (the Kotlin rule).

| from | to_name | line |
| --- | --- | --- |
| quarry::Core::Engine::render | wheel.area | 8 |
| quarry::main | Engine | 1 |
| quarry::main | Circle | 2 |
| quarry::main | engine.render | 3 |
| quarry::main | print | 4 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | inherits | 1 |

## Measured fields (spot-pinned)

- `Shape.area`: no body — decisions none, defines [],
  refs [Double] (the return type).
- `Circle.area`: defines [], refs [Double, radius] — the
  return type and the property read in the body expression.
- `Engine.render`: defines [wheel],
  refs [Circle, Double, renders] — the parameter type, the
  return type, and the augmented target `renders` (read, not
  written). writes [].
