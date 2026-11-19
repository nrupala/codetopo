# Known-answer fixture — PHP

Hand-written PHP in the shape real PHP ships in: bracketless
namespaces, `use` imports, classes with typed properties.
Every expectation in `../known_answer_php.rs` is derived from
THIS document, which was written from the sources by hand —
never from extractor output.

Package `quarry`. Module ids are path-derived (the shared
rule): `Shapes/Circle.php` → `quarry::Shapes::Circle`,
`main.php` → `quarry::main`. **Namespace names are
symbol-path segments** (the C++ rule; a bracketless namespace
prefixes the whole file): `namespace Quarry\Shapes; class
Circle …` in module `quarry::Shapes::Circle` gives the class
the id `quarry::Shapes::Circle::Quarry::Shapes::Circle`.

Variable names keep their `$` (the written form, as Ruby
keeps its sigils): a parameter is `$wheel`, a top-level
variable is `$engine`. A property write through
`$this->radius` writes `radius` (the member name as written
at the access). `$this` itself is never a ref.

A `use` import is internal when its first segment is the
file's own namespace root (or matches the package name
case-insensitively): the root segment is read as the package
name, so the target is the package plus the remaining
segments; the imported name binds. `require` / `include`
(and their `_once` forms) with a string-literal argument are
file imports, resolved relative to the including file like
a C include, with no bindings.

## Symbols (10)

### Shapes/Shape.php (2)

| name | kind | line |
| --- | --- | --- |
| Shape | interface | 5 |
| area | method (no body) | 7 |

### Shapes/Circle.php (3)

| name | kind | line |
| --- | --- | --- |
| Circle | class | 5 |
| __construct | method | 9 |
| area | method | 14 |

The `$radius` property is a field, not a symbol.

### Core/Engine.php (3)

| name | kind | line |
| --- | --- | --- |
| Engine | class | 7 |
| render | method | 11 |
| renderCount | method | 17 |

### main.php (2)

| name | kind | line |
| --- | --- | --- |
| $engine | variable | 8 |
| $wheel | variable | 9 |

## Imports (4)

| from | statement | to | bindings | line |
| --- | --- | --- | --- | --- |
| quarry::Core::Engine | `use Quarry\Shapes\Circle;` | quarry::Shapes::Circle | [Circle] | 5 |
| quarry::main | `use Quarry\Core\Engine;` | quarry::Core::Engine | [Engine] | 3 |
| quarry::main | `use Quarry\Shapes\Circle;` | quarry::Shapes::Circle | [Circle] | 4 |
| quarry::main | `require_once 'Core/Engine.php';` | quarry::Core::Engine | [] | 6 |

## Calls (4)

`to_name` is the called method's name for member and scoped
calls (the receiver is not part of it — the Python attribute
rule); object creation is a call on the type name.

| from | to_name | line |
| --- | --- | --- |
| quarry::Core::Engine::Quarry::Core::Engine::render | area | 14 |
| quarry::main | Engine | 8 |
| quarry::main | Circle | 9 |
| quarry::main | render | 10 |

## Heritage (1)

| type | target | kind | line |
| --- | --- | --- | --- |
| Circle | Shape | implements | 5 |

## Measured fields (spot-pinned)

- `Shape.area`: no body — decisions none, defines [],
  refs [float] (the return type).
- `Circle.__construct`: defines [$radius], writes [radius]
  (`$this->radius = …` writes the member name), refs [float]
  (the parameter type; the right-hand `$radius` is the
  parameter, a define).
- `Circle.area`: refs [float] — `$this->radius` reads only
  `$this`, which is never a ref.
- `Engine.render`: defines [$wheel],
  refs [Circle, area, float] — the parameter type, the
  invoked method's name (a ref, the Java rule), the return
  type. `$this->renders += 1` reads only `$this`. writes [].
- `Engine.renderCount`: refs [int].
