# Known-answer fixture — Scala

Hand-written Scala in the shape real Scala ships in: a
package per file, a trait and its implementor, an object
entry point. Every expectation in `../known_answer_scala.rs`
is derived from THIS document, which was written from the
sources by hand — never from extractor output.

Package `quarry`. Module ids are path-derived (the shared
rule): `Shapes.scala` → `quarry::Shapes`, `Engine.scala` →
`quarry::Engine`, `Main.scala` → `quarry::Main`. (The
`package` clause does not enter the module id; it anchors
the import rule below.)

An import is internal when its first segment equals the
file's package root (the `package` clause's first segment,
or the package name): then its target is the imported
type's module-shaped path — the package plus the remaining
segments. Anything else is `external::{first segment}`. The
imported name binds (a wildcard import binds nothing).

## Symbols (9)

### Shapes.scala (4)

| name | kind | line |
| --- | --- | --- |
| Shape | interface (trait) | 3 |
| area | method (abstract, no body) | 4 |
| Circle | class | 7 |
| area | method | 8 |

The constructor parameter `radius` is a field, not a symbol.

### Engine.scala (3)

| name | kind | line |
| --- | --- | --- |
| Engine | class | 5 |
| render | method | 8 |
| renderCount | method | 13 |

The `renders` var is a field, not a symbol.

### Main.scala (2)

| name | kind | line |
| --- | --- | --- |
| Main | class (object) | 6 |
| main | method | 7 |

## Imports (3)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::Engine | `import quarry.shapes.Circle` | quarry::shapes::Circle | [Circle] | 3 |
| quarry::Main | `import quarry.core.Engine` | quarry::core::Engine | [Engine] | 3 |
| quarry::Main | `import quarry.shapes.Circle` | quarry::shapes::Circle | [Circle] | 4 |

## Calls (5)

`to_name` is the callee text as written (the Kotlin rule);
`new X(…)` is a call on the type name. A parameterless
method access without parentheses (`wheel.area`) is a field
access — a ref in the scan, not a call; the fixture calls
with parentheses. Calls are in walk order: an outer call
precedes the calls in its arguments.

| from | to_name | line |
| --- | --- | --- |
| quarry::Engine::Engine::render | wheel.area | 10 |
| quarry::Main::Main::main | Engine | 8 |
| quarry::Main::Main::main | Circle | 9 |
| quarry::Main::Main::main | println | 10 |
| quarry::Main::Main::main | engine.render | 10 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | inherits | 7 |

## Measured fields (spot-pinned)

- `Shape.area`: no body — decisions none, defines [],
  refs [Double] (the return type).
- `Circle.area`: defines [], refs [Double, radius] — the
  return type and the constructor-field read in the body.
- `Engine.render`: defines [wheel],
  refs [Circle, Double, renders] — the parameter and return
  types and the augmented target `renders` (an infix `+=`,
  read not written). writes [].
- `Engine.renderCount`: refs [Int, renders].
- `Main.main`: defines [args, engine, wheel],
  writes [engine, wheel] (initialized `val`s),
  refs [Array, String, Unit, println] — the parameter type's
  identifiers, the return type, and the called name.
