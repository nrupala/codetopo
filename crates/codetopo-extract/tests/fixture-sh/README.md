# Known-answer fixture — Shell

Hand-written POSIX-flavoured shell in the shape real scripts
ship in: library files sourced by the files that use them,
functions as the only abstraction. Every expectation in
`../known_answer_sh.rs` is derived from THIS document, which
was written from the sources by hand — never from extractor
output.

Package `quarry`. Module ids are path-derived (the shared
rule): `lib/util.sh` → `quarry::lib::util`, `core/engine.sh` →
`quarry::core::engine`, `main.sh` → `quarry::main`.

Shell's import mechanism is the `source` / `.` command: a
command named `source` or `.` whose first argument is a
literal path is an **import** (recorded as an import, not a
call), resolved on module-id segments relative to the sourcing
file, like a C include. Bindings are always empty.

## Symbols (8)

### lib/util.sh (2)

| name | kind | line |
| --- | --- | --- |
| UTIL_VERSION | variable | 4 |
| log_info | function | 6 |

### lib/shapes.sh (1)

| name | kind | line |
| --- | --- | --- |
| shape_area | function | 4 |

### core/engine.sh (4)

| name | kind | line |
| --- | --- | --- |
| MAX_RENDERS | variable | 7 |
| render_count | variable | 8 |
| clamp_size | function | 10 |
| render | function | 18 |

### main.sh (1)

| name | kind | line |
| --- | --- | --- |
| main | function | 6 |

## Imports (3)

| from | to | line |
| --- | --- | --- |
| quarry::core::engine | quarry::lib::util | engine.sh:4 |
| quarry::core::engine | quarry::lib::shapes | engine.sh:5 |
| quarry::main | quarry::core::engine | main.sh:4 |

## Calls (10)

Every command is a call; `to_name` is the command name as
written. Source commands are imports, not calls (above).

| from | to_name | line |
| --- | --- | --- |
| quarry::lib::util::log_info | echo | 7 |
| quarry::lib::shapes::shape_area | echo | 6 |
| quarry::lib::shapes::shape_area | bc | 6 |
| quarry::core::engine::clamp_size | echo | 12 |
| quarry::core::engine::clamp_size | echo | 14 |
| quarry::core::engine::render | log_info | 20 |
| quarry::core::engine::render | shape_area | 21 |
| quarry::main::main | clamp_size | 8 |
| quarry::main::main | render | 9 |
| quarry::main | main | 12 |

## Heritage (0)

Shell has no type system; the section is empty by construction.

## Measured fields (spot-pinned)

- `log_info`: defines [], refs [echo], writes [] — the `$1`
  in its string is a positional parameter, not a named ref.
- `shape_area`: defines [radius], writes [radius] (the
  `local radius="$1"` declaration), refs [bc, echo] — the
  `$radius` expansion in the echo string is a read of a define
  and drops out.
- `clamp_size`: decisions 1 (the `if`; the `[ … ]` condition
  is a test command, not a call), defines [], refs [echo].
- `render`: defines [render_count], writes [render_count]
  (the arithmetic assignment), refs [log_info, shape_area] —
  `$((render_count + 1))` reads `render_count`, a define.
- `main`: defines [size], writes [size] — `local size`
  declares (a define, no write); `size=$(clamp_size …)`
  assigns. refs [clamp_size, render].
