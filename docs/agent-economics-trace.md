# Agent-economics trace — raw shell exploration vs codetopo, on pallets/click

Date: 2026-11-18. Engine: codetopo 0.9.1 (the released build, installed
from crates.io-era release binaries; the same code as tag `v0.9.1`).
Target: **pallets/click** at commit
`08b803cf32ceb2ae6a772c62d5da42757a897d44` (main, 2026-11-16) — a public,
widely known Python codebase, chosen because its answers can be checked
against the source by any reader.

Two arms answered the same five topology questions, fixed in writing
before either arm ran. Arm A used only shell exploration (grep / find /
awk / file reads). Arm B used only codetopo (one `index`, then CLI
queries). Every tool call in both arms was logged with its exact returned
bytes; the full transcripts are committed beside this record as
`agent-economics-trace-arm-a.jsonl` and `agent-economics-trace-arm-b.jsonl`.

## The counts

| Measure | Arm A — raw shell | Arm B — codetopo 0.9.1 |
| --- | ---: | ---: |
| Tool calls | 21 | 15 |
| Bytes sent (commands) | 4,437 | 1,220 |
| Bytes returned | 24,308 | 74,977 |
| Tokens sent | 1,636 | 367 |
| Tokens returned | 6,610 | 19,675 |
| Tokens, questions | 202 | 202 |
| **Tokens, total** | **8,448** | **20,244** |

Counting rules: a tool call is one command invocation (a shell pipeline
is one call; the codetopo `index` is one call; each query is one call,
including searches used to form queries and calls that failed on argument
shape). Returned text is everything the command wrote to stdout + stderr,
byte-exact as logged. Tokens are counted with **tiktoken, encoding
`cl100k_base`**, over the exact sent and returned texts plus the five
question texts. The exploring agent's own reasoning is not tool traffic
and is not counted.

Index facts (Arm B, call 5): 131 files indexed, 0 failed; 2,184 symbols;
3,738 nodes; 14,418 edges; 1,226 diagnostics recorded.

Read the table with the correctness results below — cost alone is not
the comparison. Two structural facts drive the token row:

- Arm B's returns are structured JSON dossiers. The single largest
  return was the Q1 `symbol` dossier for `Context` (29,324 bytes,
  including every incident edge) and the Q3 blast-radius enumeration
  (24,277 bytes for 420 node ids). At codetopo's default limits
  (counts-first: totals plus the first 20 entries), the same answers
  cost ~1.1 KB per call — Arm B's first `ancestors` call returned
  1,122 bytes. The five questions asked for full sets where sets were
  the answer, so this run paid for enumeration.
- Arm A's returns are terse grep lines, but the arm needed 21 calls —
  including six rounds of frontier expansion to walk a transitive
  closure by hand — and two of its five answers are proxies, not
  measurements (Q3, Q5).

## The five questions

1. **Definition.** Where is the `Context` class defined? (file + line)
2. **Callers.** Which symbols call `echo` (the function defined in
   click's utils module), directly or transitively — the full ancestor
   set over calls/imports edges? (count + the set)
3. **Blast radius.** What breaks if `Context` changes: how many symbols
   are in its blast radius (dependents, overriders, implementers), and
   which are they? (count + the set)
4. **Path.** What is the shortest typed path from `Command.main` to
   `Context.invoke` (edge kinds named in sequence), or does no path
   exist?
5. **Centrality.** Which five symbols have the most dependents (highest
   fan-in: incoming dependency edges, containment excluded)? (ordered
   top 5 + their counts)

## Answers, and the referee's verdicts

Each arm's answers were checked against the source (path + line reads)
after both arms finished. A wrong answer is reported as wrong.

### Q1 — definition of `Context`

- Arm A: `src/click/core.py:236` (one grep). **Correct.**
- Arm B: `click::src::click::core::Context`, loc `src/click/core.py:236`
  (search, confirmed by the symbol dossier). **Correct.**

### Q2 — ancestors of `echo`

- Arm A: **289 symbols.** Method: direct callers mapped by grep + an
  enclosing-definition pass — 18 symbols in `src/click` (26 call sites)
  and 243 distinct enclosing definitions in tests/examples (410 call
  sites, 35 files); then a hand-walked transitive closure over
  `src/click` in six frontier rounds, reaching a 46-symbol fixpoint
  (the 18 direct callers plus 28 more: the `ProgressBar` iteration
  methods, `Context.invoke`, `Context.forward`, `Group.invoke`,
  `Command.__call__`, `Command.parse_args`, `Command.make_context`,
  `Group.parse_args`, `Group.resolve_command`'s chain,
  `Parameter.process_value`, `Option.process_value`,
  `Option.consume_value`, `Option.prompt_for_value`,
  `Command._main_shell_completion`, `BashComplete.source`,
  `ShellComplete.get_completions`, `ShellComplete.complete`,
  `_resolve_context`, the `pass_context` / `make_pass_decorator` /
  `pass_obj` wrappers, `termui.prompt`, `termui.progressbar`).
  Test/example callers were treated as leaves (nothing in the tree
  calls them statically). Docstring examples were excluded by reading.
- Arm B: **20 symbols** (`ancestors` total, confirmed at `--limit 1000`,
  not a cap). All 20 are inside Arm A's 46.
- Referee: Arm B's 20 is the truth *about the graph*, and the graph is
  incomplete here for one named reason: **dotted calls resolve to
  external stubs**. In tests, `click.echo(...)` resolves to the stub
  node `external::click.echo` — whose ancestor count, measured
  separately by the referee, is **332**. Every one of the 26 src
  symbols Arm A found and Arm B missed is reached through a dotted or
  dynamic call (`self.invoke(ctx)`, `ctx.invoke(...)`,
  `param.handle_parse_result(...)`, `cmd.make_context(...)`,
  `self.callback(...)`). Against source truth, Arm A's set is the
  closer answer; neither arm's number is the exact runtime set (Arm A
  cannot see dynamic dispatch either — it inferred two dynamic edges
  on name evidence, and said so).

### Q3 — blast radius of `Context`

- Arm A: a proxy, not a measurement: 149 distinct (file, class,
  definition) triples in `src/click` mention the name `Context`
  (including the class's own methods and annotation-only mentions).
  The arm stated it could not separate behavioural dependents from
  mentions by grep, and did not enumerate a set for the whole tree.
- Arm B: **420 nodes**, enumerated in full: 230 in `src/`, 187 in
  `tests/`, 3 in examples/docs — every symbol holding a calls /
  inherits / implements / reads / writes edge to `Context`.
- Referee: Arm B's answer is a measurement with a stated edge set;
  Arm A's is a lower-bound proxy over a narrower scope. On this
  question the graph is simply the better instrument.

### Q4 — path from `Command.main` to `Context.invoke`

- Arm A: **a path exists**: `Command.main` —calls→ `Command.invoke`
  —calls→ `Context.invoke`, evidenced at `src/click/core.py:1583`
  (`rv = self.invoke(ctx)` inside `main`) and `src/click/core.py:1437`
  (`ctx.invoke(self.callback, **ctx.params)` inside `Command.invoke`).
  **Correct.**
- Arm B: `{"rendered": "no path"}`. **Wrong against source truth** —
  both edges on the true path are dotted calls (`self.invoke`,
  `ctx.invoke`), which the extractor records as external stubs, so the
  graph contains no such path. A "no path" answer from the graph means
  *no path over resolved edges*; on method-heavy Python code that is a
  weaker statement than it reads.

### Q5 — top 5 by fan-in

- Arm A: not reliably answerable by grep. Best proxy (distinct symbols
  referencing the name, mixed methods, stated as such): `echo` 261
  direct callers by name across the tree; `Context` 149 mentions in
  `src/click`; `Parameter` 43, `Command` 39, `Option` 37 (src-only,
  undercounted by signature collapsing).
- Arm B: a measurement (method stated in the output: incoming
  dependency edges over imports, calls, inherits, implements, defines,
  reads, writes, references; containment excluded; stubs not ranked),
  over 588 ranked nodes: **click (package) 642 · tests' conftest
  `runner` fixture 458 · `Context` 130 · `CliRunner` 123 · `Command`
  99.**
- Referee: Arm B's ranking is exact for the graph it measured, with
  one structural caveat — `echo`'s true fan-in is split between the
  resolved node and the `external::click.echo` stub (332 ancestors),
  so a genuine heavyweight is absent from the list. Arm A's proxy
  caught that heavyweight and could not rank the rest.

## What this run establishes

- **Fewer calls, exact structure, where edges resolve.** Codetopo
  answered Q1, Q3, and Q5 (as measured) in 15 calls with no frontier
  walking; the blast-radius enumeration (420 named symbols) is not
  obtainable by grep at any reasonable cost.
- **Token cost is a presentation property, not a law.** At full-set
  enumeration, codetopo's JSON cost more returned tokens than terse
  grep output (19,675 vs 6,610). At its default counts-first limits,
  the same answers return totals in ~1 KB per call. Which regime an
  agent should use depends on whether it needs the set or the count.
- **One extraction limit dominates the misses.** Dotted calls
  (`obj.method(...)`) resolve to `external::` stubs in Python
  extraction (the behaviour recorded as F-2 in the 0.8.0 gate record;
  definitions are complete, dotted call sites are not resolved). It
  splits caller sets across a stub (Q2: 20 resolved + 332 on the
  stub), erases method-call paths (Q4 false "no path"), and splits
  fan-in (Q5). Any consumer of these answers on Python code should
  read stub ancestors alongside resolved ones.
- **Raw exploration's cost is the closure.** Arm A's 21 calls were
  mostly transitive-closure bookkeeping an agent must redo per
  question; its two proxy answers (Q3, Q5) mark the questions grep
  cannot truly answer.

## Reproduction

```
# Arm B (codetopo 0.9.1), from the click tree at the pinned commit:
codetopo index --db click-ct.db click
codetopo search --db click-ct.db Context --json
codetopo ancestors --db click-ct.db click::src::click::utils::echo --limit 1000 --json
codetopo blast-radius --db click-ct.db click::src::click::core::Context --limit 1000 --json
codetopo path --db click-ct.db click::src::click::core::Command::main click::src::click::core::Context::invoke --json
codetopo rank --db click-ct.db --by fan_in --limit 5 --json
```

Arm A's commands and both arms' complete outputs are in the committed
transcripts. The referee's stub check:

```
codetopo ancestors --db click-ct.db external::click.echo --limit 5 --json
# → total: 332
```
