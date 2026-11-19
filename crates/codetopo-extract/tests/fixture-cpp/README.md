# Known-answer fixture — C++

Hand-written C++ in the shape real C++ ships in: headers with
in-namespace class declarations, implementation files with
out-of-line member definitions. Every expectation in
`../known_answer_cpp.rs` is derived from THIS document, which
was written from the sources by hand — never from extractor
output.

Package `quarry`. Module ids are path-derived (the shared
rule); a header and its implementation share a stem, hence a
module (the C rule): `shapes/Circle.hpp` and `shapes/Circle.cpp`
are both `quarry::shapes::Circle`. **Namespace names are symbol
path segments**: `namespace quarry { class Circle … }` in
module `quarry::shapes::Circle` gives the class the id
`quarry::shapes::Circle::quarry::Circle`. An out-of-line
definition `Circle::area()` inside the namespace composes the
same id the in-class declaration has:
`…::quarry::Circle::area`.

## Symbols (12)

### shapes/Shape.hpp (2)

| name | kind | line |
| --- | --- | --- |
| Shape | class | 5 |
| area | method (pure virtual, no body) | 7 |

### shapes/Circle.hpp (3)

| name | kind | line |
| --- | --- | --- |
| Circle | class | 7 |
| Circle | method (constructor declaration) | 9 |
| area | method (declaration) | 10 |

The `radius_` field is a field, not a symbol.

### shapes/Circle.cpp (2)

| name | kind | line |
| --- | --- | --- |
| Circle | method (constructor definition) | 5 |
| area | method (definition) | 7 |

### core/Engine.hpp (3)

| name | kind | line |
| --- | --- | --- |
| Engine | class | 7 |
| render | method (declaration) | 9 |
| renderCount | method (declaration) | 10 |

### core/Engine.cpp (2)

| name | kind | line |
| --- | --- | --- |
| render | method (definition) | 7 |
| renderCount | method (definition) | 12 |

## Imports (5)

The C include rule, unchanged: quoted paths resolve relative
to the including file, system includes are
`external::{first segment}`, bindings always empty.

| from | to | line |
| --- | --- | --- |
| quarry::shapes::Circle | quarry::shapes::Shape | Circle.hpp:3 |
| quarry::shapes::Circle | quarry::shapes::Circle | Circle.cpp:1 |
| quarry::core::Engine | quarry::shapes::Circle | Engine.hpp:3 |
| quarry::core::Engine | quarry::core::Engine | Engine.cpp:1 |
| quarry::core::Engine | external::cstddef | Engine.cpp:3 |

## Calls (1)

| from | to_name | line |
| --- | --- | --- |
| quarry::core::Engine::quarry::Engine::render | wheel.area | 9 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | inherits | 7 |

## Measured fields (spot-pinned)

- `Shape::area` (hpp): no body — decisions none, defines [],
  refs [double] (the return type).
- `Circle` constructor (cpp): defines [radius],
  writes [radius_] — the member initializer `radius_(radius)`
  writes the field; refs [double] (the parameter type; the
  initializer argument is the parameter, a define).
- `Circle::area` (cpp): defines [], refs [double, radius_],
  decisions 0.
- `Engine::render` (hpp declaration): defines [wheel],
  refs [Circle, double] (parameter and return types).
- `Engine::render` (cpp definition): defines [wheel],
  refs [Circle, double, renders_] — `renders_ += 1` reads the
  field; `wheel.area()` reads only `wheel`, a define.
  writes [].
- `Engine::renderCount` (cpp): refs [int, renders_].
