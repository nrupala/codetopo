# Known-answer fixture — Java

Hand-written Java in the shape real Java ships in: one public
type per file, a `package` declaration per file, imports at the
top. Every expectation in `../known_answer_java.rs` is derived
from THIS document, which was written from the sources by hand —
never from extractor output.

Package `quarry`. **Module ids come from the `package`
declaration**, not the path — a Java type's identity is its
package plus its name, and source trees carry layout noise
(`src/main/java/…`) that is not part of that identity. The module
id is the package segments plus the file stem:
`shapes/Shape.java` (`package quarry.shapes`) →
`quarry::shapes::Shape`; `Main.java` (`package quarry`) →
`quarry::Main`. (A file with no package declaration falls back to
the path rule under the caller's package name.)

## Symbols (10)

### shapes/Shape.java (2)

| name | kind | line |
| --- | --- | --- |
| Shape | interface | 3 |
| area | method (no body) | 4 |

### shapes/Circle.java (3)

| name | kind | line |
| --- | --- | --- |
| Circle | class | 3 |
| Circle | method (constructor) | 6 |
| area | method | 10 |

The `radius` field is a field, not a symbol. `area`'s line is
10, not 11: the `@Override` annotation is a child of the
method_declaration node, so the node's first line — the line
rule every extractor uses — is the annotation's line.

### core/Engine.java (3)

| name | kind | line |
| --- | --- | --- |
| Engine | class | 7 |
| render | method | 10 |
| renderCount | method | 15 |

### Main.java (2)

| name | kind | line |
| --- | --- | --- |
| Main | class | 6 |
| main | method | 7 |

## Imports (4)

An import is internal when its first segment equals the file's
own package root (`quarry`); then its target is the imported
type's module id (package segments + type name). Anything else
is `external::{first segment}`.

| from file | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| core/Engine.java | `import java.util.List;` | external::java | [List] | 3 |
| core/Engine.java | `import quarry.shapes.Circle;` | quarry::shapes::Circle | [Circle] | 5 |
| Main.java | `import quarry.core.Engine;` | quarry::core::Engine | [Engine] | 3 |
| Main.java | `import quarry.shapes.Circle;` | quarry::shapes::Circle | [Circle] | 4 |

## Calls (5)

`to_name` is the invoked method's name (the Python attribute
rule: the last segment); object creation is a call on the type
name.

| from | to_name | line |
| --- | --- | --- |
| quarry::core::Engine::render | area | 12 |
| quarry::Main::main | Engine | 8 |
| quarry::Main::main | Circle | 9 |
| quarry::Main::main | render | 10 |
| quarry::Main::main | println | 11 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | implements | 3 |

## Measured fields (spot-pinned)

- `Shape.area`: no body — decisions none, defines [],
  refs [double] (the return type).
- `Circle` constructor: defines [radius], writes [radius]
  (`this.radius = radius` writes the field name), refs [double]
  (the parameter type; the right-hand `radius` is the parameter,
  a define, and drops out).
- `Circle.area`: defines [], refs [double, radius] — a bare
  field name in a body is an identifier read.
- `Engine.render`: defines [wheel],
  refs [Circle, area, double, renders] — the parameter type, the
  invoked method's name (a call target is a ref, the Python
  rule), the return type, and the augmented target `renders`
  (read, not written). writes [].
- `Main.main`: defines [args, area, engine, wheel],
  writes [area, engine, wheel] (initialized locals),
  refs [Circle, Engine, String, System, double, println, render, void] —
  constructor type names, the parameter type, the local
  declaration's `double`, the root of
  `System.out` (only the root of a field chain is a read), the
  invoked names, the return type. decisions 0.
