# Known-answer fixture — Haskell

Hand-written Haskell in the shape real Haskell ships in: one
module per file with an explicit `module … where` header,
a typeclass with a default method, `data` / `newtype` /
`type` declarations, an instance, explicit import lists, a
qualified import, and a `Main` entry module. Every
expectation in `../known_answer_hs.rs` is derived from THIS
document, which was written from the sources by hand —
never from extractor output.

Package `quarry`. Module ids are path-derived (the shared
rule): `Shapes.hs` → `quarry::Shapes`, `Core/Engine.hs` →
`quarry::Core::Engine`, `Main.hs` → `quarry::Main`. The
module header is not a symbol: a Haskell file declares
exactly one module, the builder materialises the module
node from the module id, and the header's job in the
extractor is to anchor import resolution (the Java package
rule). Declared module names carry the package root
(`Quarry.Shapes`, `Quarry.Core.Engine`): an import is
internal when its first segment is the file's own module
root or matches the package name case-insensitively — the
root segment is read as the package name, so the target is
the package plus the remaining segments; anything else is
`external::{first segment}`.

A type signature and the binding(s) below it are ONE
declaration: the symbol's line is the signature's line,
its `signature` is the signature text, and the signature's
type names are part of the symbol's refs. A binding with
patterns is a function; a pattern-less binding is a
variable. Several equations of one function (pattern
alternatives, guards) are one symbol. Class members are
methods nested under the class; instance members are
methods nested under the instance's type (nesting them
under the class would collide with the class's own member
of the same name). Record fields (`radius`, `renders`)
are fields, not symbols; data constructors are not
symbols either — a constructor *application* is a call,
exactly as object creation is in the C# fixture.

Bindings in an import list bind as written (`Circle (..)`
binds `Circle`); a qualified import binds its alias, or
the module's last segment when it has no alias; a `hiding`
import binds nothing (it imports everything *except* the
listed names, so no name is bound by the list).

## Reads and writes in an immutable language

Haskell bindings are immutable: nothing is ever assigned
after it is bound, so every `writes` list in this fixture
is empty — by the language's semantics, not by omission.
The shared top-level values `scaleFactor` and `unitCircle`
(defined in `Shapes.hs`) are *written* exactly once, at
their definitions, which the schema models as the
`Variable` symbol itself; every later use is a read, and
the reads show up as `refs` on the reading functions:
`render` reads both `scaleFactor` (line 11) and
`unitCircle` (line 14), `fallbackArea` reads `unitCircle`
(line 25), and `main` reads `unitCircle` (lines 8 and 12).
`let` / `where` / `do`-let bindings and pattern variables
are defines of the enclosing function; reads that are also
defines are dropped (the marimo rule, as everywhere).

## Symbols (17)

### Shapes.hs (11)

| name | kind | line |
| --- | --- | --- |
| Shape | interface (class) | 5 |
| Shape.area | method (signature only) | 6 |
| Shape.describe | method (default body) | 7 |
| Circle | class (data) | 10 |
| Radius | class (newtype) | 13 |
| RadiusAlias | class (type synonym) | 15 |
| Circle.area | method (instance) | 18 |
| Circle.describe | method (instance) | 19 |
| unitCircle | variable | 21 |
| scaleFactor | variable | 24 |
| defaultRadius | variable | 27 |

### Core/Engine.hs (5)

| name | kind | line |
| --- | --- | --- |
| EngineState | class (data) | 7 |
| render | function | 9 |
| renderCount | function | 16 |
| describeAll | function | 19 |
| fallbackArea | variable | 24 |

### Main.hs (1)

| name | kind | line |
| --- | --- | --- |
| main | variable (an `IO` action value) | 6 |

## Imports (6)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::Shapes | `import Data.Maybe (fromMaybe)` | external::Data | [fromMaybe] | 3 |
| quarry::Core::Engine | `import Data.Maybe (fromMaybe)` | external::Data | [fromMaybe] | 3 |
| quarry::Core::Engine | `import Quarry.Shapes (…)` | quarry::Shapes | [Circle, Shape, area, describe, scaleFactor, unitCircle] | 4 |
| quarry::Core::Engine | `import qualified Quarry.Shapes as Shapes` | quarry::Shapes | [Shapes] | 5 |
| quarry::Main | `import Quarry.Core.Engine (…)` | quarry::Core::Engine | [EngineState, describeAll, render, renderCount] | 3 |
| quarry::Main | `import Quarry.Shapes (…)` | quarry::Shapes | [Circle, unitCircle] | 4 |

## Calls (17)

`to_name` is the callee text as written (the Kotlin rule):
a plain name for an unqualified application, the full
qualified text for a qualified one (`Shapes.area`), the
constructor's name for a constructor application. An
application chain `f a b` is ONE call (to `f`); a call in
an argument is its own call, and an outer call precedes
the calls in its arguments (walk order). Symbolic
operators (`*`, `>`, `:`) are not calls. Calls in a
variable's initializer belong to the module, not to the
variable (the Julia variable rule — the fixture's
`main.jl` and `Program.cs` calls are their modules'):
`unitCircle`'s constructor call is `quarry::Shapes`'s,
`main`'s `do` block calls are `quarry::Main`'s.

| from | to_name | line |
| --- | --- | --- |
| quarry::Shapes | Circle | 22 |
| quarry::Shapes | Radius | 28 |
| quarry::Core::Engine::render | area | 11 |
| quarry::Core::Engine::render | renders | 12 |
| quarry::Core::Engine::render | area | 14 |
| quarry::Core::Engine::renderCount | fromMaybe | 17 |
| quarry::Core::Engine::renderCount | renders | 17 |
| quarry::Core::Engine::describeAll | describe | 22 |
| quarry::Core::Engine::describeAll | describeAll | 22 |
| quarry::Core::Engine | Shapes.area | 25 |
| quarry::Main | EngineState | 9 |
| quarry::Main | print | 10 |
| quarry::Main | render | 10 |
| quarry::Main | print | 11 |
| quarry::Main | renderCount | 11 |
| quarry::Main | print | 12 |
| quarry::Main | describeAll | 12 |

## Heritage (3)

An `instance C T` is heritage: `T` implements `C`. A
`deriving (…)` clause is a compiler-generated instance,
so it is heritage of the same kind.

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Show | implements | 11 |
| Circle | Eq | implements | 11 |
| Circle | Shape | implements | 17 |

## Measured fields (spot-pinned)

- `Shape.area`: no body — decisions none, defines [],
  refs [Double] (the signature's result type; `a` is a
  type variable, not a ref).
- `Shape.describe`: defines [s], refs [String] (the
  signature's result type; the body is a literal).
  decisions 0.
- `Circle.area` (instance): defines [r] (the pattern
  variable), refs [Circle] (the pattern's constructor).
  decisions 0.
- `Circle.describe` (instance): defines [c], refs [].
  decisions 0.
- `unitCircle`: refs [Circle] (the signature's type; the
  body's constructor application reads the same name).
- `scaleFactor`: refs [Double]. `defaultRadius`:
  refs [Radius].
- `render`: defines [scaled, state, wheel] (the
  parameters and the `let` binding), refs [Circle,
  Double, EngineState, area, renders, scaleFactor,
  unitCircle] — the signature's types, the called names
  (a field selector `renders` is a function in Haskell),
  and the shared values read. writes []. decisions 2
  (the `if` +1, the `&&` +1).
- `renderCount`: defines [override, state],
  refs [EngineState, Int, Maybe, fromMaybe, renders].
  decisions 0.
- `describeAll`: defines [c, rest, shapes] (the
  parameter and the case alternative's pattern variables),
  refs [Circle, String, describe, describeAll] — the
  signature's types, the called name, and the recursive
  call. decisions 1 (a `case` with two alternatives
  contributes alternatives − 1).
- `fallbackArea`: refs [Double, area, unitCircle] — the
  signature's type, the qualified call's name part, and
  the shared value read.
- `main`: defines [state, wheel] (the `do`-let
  bindings), refs [EngineState, IO, Nothing,
  describeAll, print, render, renderCount, unitCircle]
  — the signature's type, the constructed state, the
  `Nothing` constructor, the called names, and the
  shared value read. decisions none (a variable).
