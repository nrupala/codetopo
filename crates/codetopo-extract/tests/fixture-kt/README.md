# quarry — the known-answer fixture package (Kotlin)

A small hand-written Kotlin package whose extraction answers are
known **by construction** and asserted by
`crates/codetopo-extract/tests/known_answer_kt.rs`. If a change to
the extractor moves any number below, the change is either a bug
or a deliberate mapping change — in the second case this README's
arithmetic must be updated in the same PR, showing the new
derivation. (Counts cross-checked by hand from the sources; the
README shows the arithmetic.)

The package exercises: an interface and an implementor; a class
with a primary-constructor property; module-level `const val` /
`var` properties; a top-level builder function; cross-file imports
(internal, aliased by path, and external).

Module ids are path-derived (the shared rule): the package name,
then the directories, then the file stem. A file named after its
class therefore yields a doubled final segment (`shapes/Circle.kt`
→ module `quarry::shapes::Circle`, class `Circle` →
`quarry::shapes::Circle::Circle`) — the rule is the same one that
gives Python `quarry::core::Engine` for class `Engine` in
`core.py`; here the stem happens to equal the class name.

## Extraction answers (per file)

### `shapes/Shape.kt` → module `quarry::shapes::Shape`

Symbols (2): `Shape` (interface — the declaration's keyword is
`interface`, line 3), `Shape::area` (method, line 4; no body).
Imports 0 · calls 0 · heritage 0.

### `shapes/Circle.kt` → module `quarry::shapes::Circle`

Symbols (2): `Circle` (class, line 5), `Circle::area` (method,
line 6). The primary-constructor property `val radius` is not a
symbol (the class-level-field rule).

Imports (1): `import kotlin.math.PI` → `external::kotlin`
(bindings `[PI]` — the last segment; the first segment `kotlin`
is not the package name, so the dotted rule lands external).

Heritage (1): `Circle` → `Shape` (Inherits, line 5 — the
delegation specifier `: Shape`).

Calls 0. `area`'s scan: defines `[]`, refs `[Double, PI, radius]`
(the return type by the annotation rule; `PI` and the constructor
property read in the body), writes `[]`.

### `core/Engine.kt` → module `quarry::core::Engine`

Symbols (5): `DEFAULT_SIZE` (variable — module-level `const val`,
line 6), `counter` (variable, line 8), `Engine` (class, line 10),
`Engine::render` (method, line 13), `buildEngine` (function,
line 19). The class-level `var size` is not a symbol.

Imports (2): `import quarry.shapes.Circle` → `quarry::shapes::Circle`
(`[Circle]`, line 3); `import quarry.shapes.Shape` →
`quarry::shapes::Shape` (`[Shape]`, line 4). First segment is the
package name, so both resolve internally, onto the modules the
path rule derives for those files.

Calls (3): `render` → `shape.area` (line 15, the navigation callee
as written); `buildEngine` → `Engine` (line 20), `buildEngine` →
`Circle` (line 20) — constructor calls are calls, outer first.

Heritage 0 (the constructor parameter `val shape: Shape` is not a
supertype).

Scans: `render` — defines `[]`, refs `[Double, counter, shape]`
(`counter += 1` reads `counter` by the augmented rule;
`shape.area()` reads the root `shape`; `Double` by the annotation
rule), writes `[]`. `buildEngine` — defines `[]`, refs
`[Circle, Engine]` (the two callee names; the return type
`Engine` is already in the set), writes `[]`. The `Engine` class
symbol — refs `[DEFAULT_SIZE, Int]` (the class-level property's
initializer and its type; method bodies are pruned from the class
scan), defines/writes `[]`.

### `Main.kt` → module `quarry::Main`

Symbols (1): `main` (function, line 6).

Imports (2): `import quarry.core.Engine` → `quarry::core::Engine`
(`[Engine]`, line 3); `import quarry.shapes.Circle` →
`quarry::shapes::Circle` (`[Circle]`, line 4).

Calls (4), all from `main`: `Engine` (line 7), `Circle` (line 7),
`println` (line 8), `engine.render` (line 8).

Scan of `main`: defines `[engine]`, writes `[engine]` (the local
`val` binds and writes, the assignment precedent), refs
`[Circle, Engine, println]` (`engine` is read in `engine.render()`
but is a define, so it is filtered from refs; `render` is a
navigation member, never a ref).

### Package totals (extraction)

10 symbols · 5 imports · 7 calls · 1 heritage row.

## Full-directory indexing

The answers above are the Kotlin files only — that is what
`known_answer_kt.rs` extracts. Indexing the fixture directory
through the CLI also indexes this README (Markdown is a supported
surface), which adds a doc node and its file/module nodes.
