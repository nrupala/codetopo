# quarry — the known-answer fixture package (F2)

A small hand-written Python package whose extraction and graph
answers are known **by construction** and asserted by
`crates/codetopo-extract/tests/known_answer_py.rs`. If a change to
the extractor or the builder moves any number below, the change is
either a bug or a deliberate mapping change — in the second case
this README's arithmetic must be updated in the same PR, showing
the new derivation. (Per the build spec: counts cross-checked by
hand; the README shows the arithmetic.)

The package exercises: `__init__.py` + two submodules; relative,
absolute, and external imports; a `Protocol`; a base/derived pair;
a decorated function and class; module-level constants; and a
property with a setter — the duplicate-id case pinned below.

## Extraction answers (per file)

### `__init__.py` → module `quarry` (the root module: id == package)

Symbols (2): `quarry::__all__`, `quarry::VERSION` — both
module-level plain-name assignments, both variables. Nothing else
in the file declares a name.

Imports (2): `from .core import Engine` → `quarry::core`
(bindings `[Engine]`); `from .shapes import Shape` →
`quarry::shapes` (`[Shape]`). One dot from the root package module
stays in the package.

### `core.py` → module `quarry::core`

Symbols (8 rows, 7 unique ids):

| id | kind | why |
|---|---|---|
| `quarry::core::DEFAULT_SIZE` | variable | module-level assignment |
| `quarry::core::Drawable` | interface | class with a `Protocol` base (D2) |
| `quarry::core::Drawable::draw` | method | def in the class body; stub body, 0 decisions |
| `quarry::core::Engine` | class | plain class, no bases |
| `quarry::core::Engine::__init__` | method | |
| `quarry::core::Engine::size` | method | the `@property` getter, line 20 |
| `quarry::core::Engine::size` | method | the `@size.setter`, line 24 — **same id, deliberately** |
| `quarry::core::Engine::render` | method | |

Imports (3): `import os` → `external::os` (`[os]`);
`from typing import Protocol` → `external::typing` (`[Protocol]`);
`from quarry.shapes import Circle, Shape` → `quarry::shapes`
(`[Circle, Shape]` — absolute import: first segment is the package
name, so it resolves internally).

Calls (1): `render` returns `shape.draw()` →
`quarry::core::Engine::render` → `shape.draw`, line 28.

Heritage (1 row): `Drawable` → `Protocol` (Inherits). See the
graph note below for why no edge results.

### `shapes.py` → module `quarry::shapes`

Symbols (6): `Shape` (class), `Shape::area` (method), `Circle`
(class), `Circle::__init__` (method), `Circle::area` (method),
`Point` (class — the `@dataclass`; its annotated fields `x`/`y`
are class-level assignments and are **not** symbols).

Imports (2): `import math` → `external::math` (`[math]`);
`from dataclasses import dataclass` → `external::dataclasses`
(`[dataclass]`).

Heritage (1 row): `Circle` → `Shape` (Inherits).

### Package totals (extraction)

16 symbol rows (15 unique ids) · 7 imports · 1 call · 2 heritage
rows.

## Graph answers (builder output)

Nodes (26):

- Package 1 — `quarry` (the root module doubles as the package
  node, CODE_SCHEMA §3.1).
- Module 2 — `quarry::core`, `quarry::shapes`.
- File 3 — one per source file.
- Symbols 15 — the kept rows: Variable 3 (`__all__`, `VERSION`,
  `DEFAULT_SIZE`), Interface 1 (`Drawable`), Class 4 (`Engine`,
  `Shape`, `Circle`, `Point`), Method 7 (`draw`, `__init__`,
  `size` ×1 kept, `render`, `Shape::area`, `Circle::__init__`,
  `Circle::area`).
- External 5 — import stubs `external::os`, `external::typing`,
  `external::dataclasses`, `external::math`, plus the unresolved
  call stub `external::shape.draw`.

Edges (33):

- Contains 20 — package→{root file, core module, shapes module}
  3; each non-root module→its file 2; files→their symbols, flat:
  root file 2 + core file 7 (kept symbols only) + shapes file 6 =
  15. 3 + 2 + 15 = 20.
- Imports 7 — one per RawImport row, external ones landing on the
  stubs above.
- Calls 1 — `render` → `external::shape.draw`: the suffix rule
  needs a symbol id ending `::shape.draw`; `Drawable::draw` ends
  `::Drawable::draw`, so the call is unresolved (and diagnosed).
- Reads 2 — `Engine::__init__` reads `DEFAULT_SIZE` (its parameter
  default is a read of the constant); `Circle::area` reads
  `external::math` (the attribute root of `math.pi`, resolved via
  the import binding).
- Defines 1 — `Engine::__init__` defines `Engine::size`: its
  parameter is named `size`, and the name resolves to the property.
- Writes 1 — `Engine::__init__` writes `Engine::size`
  (`self.size = size` writes the attribute name `size`).
- Inherits 1 — `Circle` → `Shape`. Drawable's `Protocol` heritage
  produces **no edge**: the target name resolves to no symbol in
  the graph and unresolvable heritage is silent by design — the
  `interface` node kind is the carrier of the Protocol fact (D2),
  not an edge.

Diagnostics (2):

- `MultiDefinition` for `quarry::core::Engine::size`, locations
  core.py:20 and core.py:24.
- `UnresolvedCall` for `shape.draw` from `Engine::render`.

## Full-directory indexing (the F4/F5 surfaces' view)

The answers above are the Python files only — that is what
`known_answer_py.rs` extracts. Indexing the fixture **directory**
through the CLI, HTTP, or MCP also indexes this README (Markdown
is a supported surface), which adds: 1 file (4 files indexed in
total), +1 module node and +1 doc node and +1 file node
(26 → **29 nodes**), and +3 contains edges (package→README
module, module→its file, file→its doc; 33 → **36 edges**). The
parity and lifecycle suites (`codetopo-server/tests/api_py.rs`,
`codetopo-cli/tests/lifecycle_py.rs`,
`codetopo-mcp/tests/mcp_py.rs`) assert those totals against the
CLI's view of the same database.

## The property/setter duplicate, stated plainly

A `@property` and its `@x.setter` are two `def`s with one name in
one class body, so the extractor emits two symbol rows with the
**same id** — it does not invent a distinguishing name, because
any rename would be a fiction the schema never authorized. The
builder's duplicate rule (documented in `graph.rs`) keeps the
first occurrence — the getter — ignores the duplicate wholesale,
and reports `MultiDefinition` with both locations. Conditional
duplicate definitions (`if TYPE_CHECKING:` / ImportError fallback
pairs) surface the same way, by schema design.
