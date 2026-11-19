# Known-answer fixture — C

Hand-written C (a header/implementation pair per module, the
shape C code actually ships in). Every expectation in
`../known_answer_c.rs` is derived from THIS document, which was
written from the sources by hand — never from extractor output.

Package `quarry`. Modules are path-derived; a header and its
implementation share a stem, hence a module (§3.1):
`shapes/shape.h` → `quarry::shapes::shape`;
`shapes/circle.h` and `shapes/circle.c` → `quarry::shapes::circle`;
`core/engine.h` and `core/engine.c` → `quarry::core::engine`.

## Symbols (15)

### shapes/shape.h (2)

| name | kind | line |
| --- | --- | --- |
| Shape | class | 4 |
| shape_area | function (prototype) | 9 |

The function-pointer field `area` is a field, not a symbol.

### shapes/circle.h (2)

| name | kind | line |
| --- | --- | --- |
| Circle | class | 6 |
| circle_area | function (prototype) | 10 |

### shapes/circle.c (2)

| name | kind | line |
| --- | --- | --- |
| PI | variable | 5 |
| circle_area | function | 7 |

### core/engine.h (4)

| name | kind | line |
| --- | --- | --- |
| Engine | class | 6 |
| engine_count | variable (`extern`) | 11 |
| engine_render | function (prototype) | 13 |
| engine_reset | function (prototype) | 14 |

### core/engine.c (5)

| name | kind | line |
| --- | --- | --- |
| MAX_RENDER_DEPTH | variable (object-like macro with a value) | 5 |
| engine_count | variable | 7 |
| clamp_size | function | 9 |
| engine_render | function | 16 |
| engine_reset | function | 22 |

The `#include` guards (`#define` with no value) emit nothing.

## Imports (6)

| from | to | line | rule |
| --- | --- | --- | --- |
| quarry::shapes::circle | quarry::shapes::shape | circle.h:4 | quoted, same directory |
| quarry::shapes::circle | quarry::shapes::circle | circle.c:1 | quoted, same directory |
| quarry::shapes::circle | external::math | circle.c:3 | system include |
| quarry::core::engine | quarry::shapes::circle | engine.h:4 | quoted, `..` ascends |
| quarry::core::engine | quarry::core::engine | engine.c:1 | quoted, same directory |
| quarry::core::engine | external::stddef | engine.c:3 | system include |

Bindings are always empty — a C include binds no names.

## Calls (2)

| from | to_name | line |
| --- | --- | --- |
| quarry::core::engine::engine_render | clamp_size | 18 |
| quarry::core::engine::engine_render | circle_area | 19 |

## Heritage (0)

C has no inheritance syntax; the section is empty by
construction.

## Measured fields (spot-pinned)

- `circle_area` (circle.c): signature
  `double circle_area(const struct Circle *circle)`, decisions 0,
  defines [circle], refs [Circle, PI, double], writes [] — the
  return type and the parameter's struct type are refs (the D5
  rule); `circle->radius` reads only `circle`, which is a define
  and drops out.
- `clamp_size`: decisions 1 (the `if`), defines [size],
  refs [int] (the return type), writes [].
- `engine_render` (engine.c): defines [engine],
  refs [Engine, circle_area, clamp_size, double, engine_count],
  writes [size] (`engine->size = …` writes the field name;
  `engine_count += 1` reads `engine_count`).
- `engine_reset` (engine.c): defines [engine_count],
  writes [engine_count], refs [void] — a plain assignment target
  is a define and a write (the marimo rule); the `void` return
  type is a ref.
- `shape_area` (prototype): defines [shape], refs [Shape, double].
- `PI`: signature `static const double PI` (the shared
  signature rule strips a trailing `=`). Likewise the definition
  of `engine_count` in engine.c: signature `int engine_count`.
