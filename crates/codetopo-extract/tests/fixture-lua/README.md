# Known-answer fixture — Lua

Hand-written Lua in the shape real Lua ships in: table-based
"classes" with methods defined by `function T:name`, modules
loaded by `require`, a shared module-table counter and a
shared global. Every expectation in `../known_answer_lua.rs`
is derived from THIS document, which was written from the
sources by hand — never from extractor output.

Package `quarry`. Module ids are path-derived (the shared
rule): `shapes.lua` → `quarry::shapes`, `core/engine.lua` →
`quarry::core::engine`, `main.lua` → `quarry::main`.

Lua has no class syntax: a table is the only type
construction. A top-level name bound directly to a table
constructor (`local Shape = {}`) is therefore a class-kind
symbol — the table other definitions hang methods on.
Table fields are not symbols, and neither are top-level
field assignments (`Shape.__index = Shape`,
`Engine.renders = 0` — module state, not declarations).
A definition on a dotted or method name
(`function Shape.new`, `function Shape:area`) is a method
nested under its table: its symbol path is the name chain.
A plain `function name` / `local function name` is a
function. Any other top-level plain-name binding is a
variable. A top-level name bound to a `setmetatable(...)`
call is a variable, not a class — only a direct table
literal is a class (the syntactic rule, stated plainly).

`require("a.b")` with a string-literal argument is an
import, not a call (the R rule). Lua resolves `require`
against `package.path` from the package root, never
relative to the requiring file, so the dotted path is
package-root-relative: segments split on `.` (or `/`) join
under the package name — `require("core.engine")` in any
file targets `quarry::core::engine`; a path already
starting with the package name is used as written. The
import binds the name the call is assigned to
(`local engine = require(...)` binds `engine`); a bare
`require` binds nothing. A `require` whose argument is not
a string literal is an ordinary call.

Heritage: Lua has no inheritance syntax. Exactly one
metatable form is counted, because it is the subclassing
idiom with an unambiguous reading: a `setmetatable(T, {
__index = U })` call whose first argument is a plain name
`T` and whose metatable's `__index` field is a plain name
`U` records `T` inherits `U`. `T.__index = T` is NOT
heritage (it is how a table becomes its own method table),
and `setmetatable({}, T)` instance construction is NOT
heritage (its first argument is a fresh instance, not a
type). The counted call is still also a call.

## Symbols (16)

### shapes.lua (5)

| name | kind | line |
| --- | --- | --- |
| Shape | class (table literal) | 1 |
| Shape.new | method (`function Shape.new`) | 4 |
| Shape.area | method (`function Shape:area`) | 10 |
| Circle | class (table literal) | 14 |
| Circle.area | method (`function Circle:area`) | 17 |

### core/engine.lua (5)

| name | kind | line |
| --- | --- | --- |
| shapes | variable | 1 |
| Shape | variable | 2 |
| Engine | class (table literal) | 4 |
| Engine.new | method (`function Engine.new`) | 8 |
| Engine.render | method (`function Engine:render`) | 13 |

### main.lua (6)

| name | kind | line |
| --- | --- | --- |
| engine | variable | 1 |
| shapes | variable | 2 |
| sharedCount | variable (global) | 4 |
| wheel | variable | 6 |
| state | variable | 7 |
| bump | function (local) | 11 |

## Imports (3)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::core::engine | `local shapes = require("shapes")` | quarry::shapes | [shapes] | 1 |
| quarry::main | `local engine = require("core.engine")` | quarry::core::engine | [engine] | 1 |
| quarry::main | `local shapes = require("shapes")` | quarry::shapes | [shapes] | 2 |

## Calls (11)

`to_name` is the callee text as written (the Julia rule):
a method call keeps its colon form (`wheel:area`), a
dotted call keeps its dots (`Shape.area`). Calls are
recorded in walk order: an outer call precedes the calls
in its arguments.

| from | to_name | line |
| --- | --- | --- |
| quarry::shapes::Shape::new | setmetatable | 5 |
| quarry::shapes | setmetatable | 15 |
| quarry::shapes::Circle::area | Shape.area | 19 |
| quarry::core::engine::Engine::new | setmetatable | 9 |
| quarry::core::engine::Engine::render | wheel:area | 15 |
| quarry::main | engine.new | 7 |
| quarry::main | print | 8 |
| quarry::main | state:render | 8 |
| quarry::main | print | 9 |
| quarry::main | print | 16 |
| quarry::main | bump | 16 |

The two `require` calls are imports, not calls (above).

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | inherits | 15 |

## Measured fields (spot-pinned)

- `Shape.new`: defines [radius, self], refs [Shape,
  setmetatable] — the parameter, the local instance, the
  metatable call and its table argument; `self.radius =
  radius` reads only defines. writes [radius, self] — the
  field name of `self.radius = …` and the local itself.
  decisions 0.
- `Shape.area`: defines [], refs [self] — a colon method's
  `self` is implicit (no parameter node exists for it), so
  its reads stay refs. writes []. decisions 0.
- `Circle.area`: defines [], refs [Shape, self] — the
  `self.radius` read's root and the delegated call's root;
  the invoked name of a dotted callee is not a ref (the
  Julia field rule). writes []. decisions 1 (the `if`).
- `Engine.new`: defines [self], refs [Engine,
  setmetatable], writes [self]. decisions 0.
- `Engine.render`: defines [wheel], refs [Engine] — the
  counter read and write both go through the `Engine`
  root; the method call's receiver `wheel` is a define.
  writes [renders] — `Engine.renders = …` writes the
  field name. decisions 0.
- Variables: `shapes` (engine.lua) and `engine`/`shapes`
  (main.lua) have refs [require] — the initializer's
  callee, per the scan (the call itself is an import, not
  a call). `Shape` (engine.lua) refs [shapes]; `wheel`
  refs [shapes]; `state` refs [engine]; `sharedCount`
  refs [].
- `bump`: defines [sharedCount], refs [], writes
  [sharedCount] — a plain assignment inside a function
  defines and writes the name in the scan (the marimo
  discipline); its reads are reads of a define and drop.
  decisions 0.
