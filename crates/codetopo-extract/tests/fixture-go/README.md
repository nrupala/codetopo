# Known-answer fixture — Go

Hand-written Go in the shape real Go ships in: one package per
directory, methods on receivers, grouped imports. Every
expectation in `../known_answer_go.rs` is derived from THIS
document, which was written from the sources by hand — never
from extractor output.

Package `quarry` (the module path a `go.mod` would declare).
**Module ids are the package plus the directory** — in Go all
files in a directory form one package, so the module id is the
caller's package name plus the directory segments, with NO file
stem: `shapes/*.go` → `quarry::shapes`, `core/engine.go` →
`quarry::core`, a root file → `quarry`. Methods nest under
their receiver type's name: `quarry::shapes::Circle::Area`.

## Symbols (10)

### shapes/shape.go (2)

| name | kind | line |
| --- | --- | --- |
| Shape | interface | 3 |
| Area | method (under Shape) | 4 |

### shapes/circle.go (3)

| name | kind | line |
| --- | --- | --- |
| Circle | class (struct) | 3 |
| NewCircle | function | 7 |
| Area | method (under Circle) | 11 |

The `Radius` field is a field, not a symbol.

### core/engine.go (4)

| name | kind | line |
| --- | --- | --- |
| MaxRenders | variable (const) | 5 |
| Count | variable | 7 |
| Engine | class (struct) | 9 |
| Render | method (under Engine) | 14 |

### main.go (1)

| name | kind | line |
| --- | --- | --- |
| main | function | 10 |

## Imports (4)

The import path IS the module path: `/` → `::`. A path whose
first segment is the package name is internal; anything else is
`external::{first segment}`. Bindings: the package name (last
segment) by default, the alias when one is written, none for a
dot import.

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::core | `import "quarry/shapes"` | quarry::shapes | [shapes] | engine.go:3 |
| quarry | `import "fmt"` | external::fmt | [fmt] | main.go:4 |
| quarry | `import "quarry/core"` | quarry::core | [core] | main.go:6 |
| quarry | `import "quarry/shapes"` | quarry::shapes | [shapes] | main.go:7 |

## Calls (4)

`to_name` is the called function's name — for a selector call,
the field (the Python attribute rule).

| from | to_name | line |
| --- | --- | --- |
| quarry::core::Engine::Render | Area | 17 |
| quarry::main | NewCircle | 11 |
| quarry::main | Render | 12 |
| quarry::main | Println | 13 |

## Heritage (0)

None in this fixture. By design, Go struct embedding is NOT
heritage (it is composition; method promotion is a
call-resolution concern, not a type-hierarchy edge). An
interface embedding another interface IS heritage (Inherits),
as in Kotlin.

## Measured fields (spot-pinned)

- `Shape.Area`: no body — decisions none, defines [],
  refs [float64] (the result type).
- `NewCircle`: defines [radius], refs [Circle, float64]
  (parameter type, result type, composite-literal type — the
  literal's `radius` value is the parameter, a define).
- `Circle.Area`: defines [c] (the receiver),
  refs [Circle, float64] (receiver type, result type);
  `c.Radius` reads only `c`, a define.
- `Engine.Render`: defines [e], refs [Engine, float64],
  writes [] — `e.renders++` writes nothing under the scan rules
  (an update's field target is not a write; its object is a
  read of a define), and `Count++` on a package-level variable
  is an update of an identifier, also not a write.
- `main`: defines [area, engine], writes [area, engine]
  (short-var declarations), refs [Engine, fmt, shapes] — the
  composite-literal type name and the two package roots of
  selector calls (a selector's field is not a ref).
