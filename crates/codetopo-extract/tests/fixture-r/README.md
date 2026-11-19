# Known-answer fixture — R

Hand-written R in the shape real R ships in: functions and
constants bound by `<-` at the top level, `library()` for
packages, `source()` for files. Every expectation in
`../known_answer_r.rs` is derived from THIS document, which
was written from the sources by hand — never from extractor
output.

Package `quarry`. Module ids are path-derived (the shared
rule): `lib/shapes.R` → `quarry::lib::shapes`,
`core/engine.R` → `quarry::core::engine`, `main.R` →
`quarry::main`.

In R, a function definition is an assignment whose right side
is a `function(…)` expression — the grammar's
`function_definition` node under a `binary_operator`. An
assignment binding a function is a Function symbol (at any
nesting level, as Python's nested `def` is); an assignment
binding anything else is a Variable at the top level and a
scan define inside a function. R has no class-definition
syntax in this grammar (S3/S4/R6 types are built by calls), so
there are no Class symbols and no heritage — both sections
are empty by construction.

Imports are calls: `library(x)` / `require(x)` load a package
— `external::{name}` with the name as binding — and
`source("path")` includes a file, resolved relative to the
sourcing file like a C include. A call by any of those names
with a literal first argument is recorded as an import, not
a call.

## Symbols (7)

### lib/shapes.R (2)

| name | kind | line |
| --- | --- | --- |
| PI | variable | 1 |
| circle_area | function | 3 |

### core/engine.R (3)

| name | kind | line |
| --- | --- | --- |
| render_count | variable | 3 |
| clamp_size | function | 5 |
| render | function | 13 |

### main.R (2)

| name | kind | line |
| --- | --- | --- |
| engine | variable | 3 |
| area | variable | 4 |

## Imports (2)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::core::engine | `library(stats)` | external::stats | [stats] | 1 |
| quarry::main | `source("core/engine.R")` | quarry::core::engine | [] | 1 |

## Calls (5)

Calls in a top-level variable's initializer belong to the
module scope (the established rule for initializers).

| from | to_name | line |
| --- | --- | --- |
| quarry::core::engine::render | circle_area | 15 |
| quarry::main | list | 3 |
| quarry::main | list | 3 |
| quarry::main | render | 4 |
| quarry::main | print | 5 |

## Heritage (0)

None — see the note above.

## Measured fields (spot-pinned)

- `circle_area`: defines [circle], refs [PI] — `circle$radius`
  reads only `circle` (the extract rule), a define.
  decisions 0.
- `clamp_size`: defines [size], refs [], decisions 1 (the
  `if`).
- `render`: defines [engine, render_count],
  writes [render_count] — `<<-` binds the name in this
  function's cell (the marimo rule), so the right-hand read
  of `render_count` is a define and drops out of refs;
  refs [circle_area] (the called name).
