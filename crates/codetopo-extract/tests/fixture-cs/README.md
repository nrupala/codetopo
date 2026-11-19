# Known-answer fixture — C#

Hand-written C# in the shape real C# ships in: namespaces
wrapping types, `using` directives at the top, a top-level
entry file. Every expectation in `../known_answer_cs.rs` is
derived from THIS document, which was written from the sources
by hand — never from extractor output.

Package `quarry`. Module ids are path-derived (the shared
rule): `Shapes/Circle.cs` → `quarry::Shapes::Circle`,
`Program.cs` → `quarry::Program`. **Namespace names are
symbol-path segments** (the C++ rule): `namespace Quarry.Shapes
{ class Circle … }` in module `quarry::Shapes::Circle` gives
the class the id `quarry::Shapes::Circle::Quarry::Shapes::Circle`.

A plain `using` binds no name (it imports a namespace's
members), so bindings are empty except for an alias using,
which binds the alias. A using is internal when its first
segment is the file's own namespace root (`Quarry` here), or
matches the package name case-insensitively (the Program.cs
case — a top-level file declares no namespace) — the root
segment is read as the package name, so the target
is the package plus the remaining segments as a module id;
anything else is `external::{first segment}`.

## Symbols (10)

### Shapes/Shape.cs (2)

| name | kind | line |
| --- | --- | --- |
| Shape | interface | 3 |
| Area | method (no body) | 5 |

### Shapes/Circle.cs (3)

| name | kind | line |
| --- | --- | --- |
| Circle | class | 5 |
| Circle | method (constructor) | 9 |
| Area | method | 14 |

The `Radius` property is a field, not a symbol.

### Core/Engine.cs (3)

| name | kind | line |
| --- | --- | --- |
| Engine | class | 6 |
| Render | method | 10 |
| RenderCount | method | 16 |

### Program.cs (2)

| name | kind | line |
| --- | --- | --- |
| engine | variable | 5 |
| wheel | variable | 6 |

## Imports (6)

| from | statement | to | line |
| --- | --- | --- | --- |
| quarry::Shapes::Circle | `using System;` | external::System | 1 |
| quarry::Core::Engine | `using System.Collections.Generic;` | external::System | 1 |
| quarry::Core::Engine | `using Quarry.Shapes;` | quarry::Shapes | 2 |
| quarry::Program | `using System;` | external::System | 1 |
| quarry::Program | `using Quarry.Core;` | quarry::Core | 2 |
| quarry::Program | `using Quarry.Shapes;` | quarry::Shapes | 3 |

## Calls (5)

`to_name` is the callee text as written (the Kotlin rule);
object creation is a call on the type name. Calls are in
walk order: an outer call precedes the calls in its arguments.

| from | to_name | line |
| --- | --- | --- |
| quarry::Core::Engine::Quarry::Core::Engine::Render | wheel.Area | 13 |
| quarry::Program | Engine | 5 |
| quarry::Program | Circle | 6 |
| quarry::Program | Console.WriteLine | 7 |
| quarry::Program | engine.Render | 7 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | inherits | 5 |

(For a class, the first base-list type is Inherits and the
rest are Implements; for an interface all are Inherits. This
fixture exercises the class form with one base.)

## Measured fields (spot-pinned)

- `Shape.Area`: no body — decisions none, defines [],
  refs [double] (the return type).
- `Circle` constructor: defines [Radius, radius],
  writes [Radius] — a bare-name assignment target both
  defines and writes (the marimo rule, applied mechanically;
  here the assigned name is the property), refs [double]
  (the parameter type).
- `Circle.Area`: refs [Math, Radius, double] — the member
  access `Math.PI` reads `Math`; the property reads are
  identifier reads.
- `Engine.Render`: defines [wheel],
  refs [Area, Circle, _renders, double] — the parameter and
  return types, the augmented target (read, not written),
  and the invoked method's name (a ref, the Java rule).
  writes [].
- `Engine.RenderCount`: refs [_renders, int].
