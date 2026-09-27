# code-yeongyu/oh-my-openagent (OmO) — survey for agent-optimized code-viz library

Repo: https://github.com/code-yeongyu/oh-my-openagent · ~69.5k stars · default branch `dev` · TUI coding-agent harness ("Just type `mass ulw` — now you are the master of graph engineering")
Related: **jc01rho/omo-herdr-dag** (MIT) — third-party extension rendering live OmO workflow DAGs in a terminal side pane, driven by OmO's `omo.dag.updated` event.

> Reality check for our study: OmO is a **coding agent, not a code-viz library**. Its "graph engineering" is **agent-orchestration graphs** — a DAG of subagents with dependencies. The value for us is the *data model of an agent-consumable graph* (nodes with id/prompt/category/dependsOn, live state updates, retry/amend ops) and the omo-herdr-dag *event-driven snapshot* pattern — both directly transferable to visualizing agent workflows over a codebase.

## What it renders

- OmO itself renders a **TUI** (terminal UI on its "senpi" runtime): conversation + task lists.
- The **graph surface** comes from `mass-ulw` orchestration: the whole job becomes "a graph of agents" — each node = a worker agent with `{id, prompt, category, dependsOn}`; `/dag` opens a detail view.
- `omo-herdr-dag` (separate repo, MIT) renders the **live workflow DAG in a Herdr terminal side pane** (~35% width): node states (running/pending/failed), dependency edges, task details beside the conversation; auto-expands running nodes, folds the rest; keeps completed/failed runs visible after disconnect; snapshots persist to local JSON.
- Notable: the DAG is rendered with **box-drawing characters in a terminal** — deliberately low-fidelity, text-first. The audience for the picture is… a human watching agents. But the *underlying model* (id → deps → state machine) is exactly agent-consumable.

## Data model — how data is structured, fed in, queried

- **`mass-ulw` run definition:** nodes declared from an eval cell as `{id, prompt, category, dependsOn}` — one run covers one phase; the next phase is a new run. Failed nodes recovered via `retry`, steered via `send`, edited via `amend` — i.e., the graph is **mutable by named operations**, not rebuilt from scratch.
- **State machine per node:** pending → running → done/failed; `/dag` detail view shows task description, agent/model, progress excerpt (512 chars), elapsed time, turn/tool counts, timestamps.
- **om-herdr-dag snapshot schema** (local JSON, no network): `{sessionId, runId, nodeLabels, nodeStates, taskIds, dependencyEdges[], errorMessages}`. Task details are linked to workflow nodes **by task ID**; a child-task relationship is kept **separate from dependency edges** (the viewer explicitly does not turn dep edges into parent/child links) — a good modeling discipline: *dependency* vs *containment* are different edge types.
- **Event-driven updates:** the extension subscribes to `omo.dag.updated`; snapshots checkpoint to `<task state dir>/dag/runs/`; viewer reuses the session pane across updates and restores checkpoints without rerunning tasks.
- **Query surface:** nodes are queryable by id/state; ordinary session subtasks are shown even with no DAG. But there is no graph-*analysis* API (no transitive-closure, no critical-path, no "what blocks X") — state is for display and manual steering.

## What makes it good (distinctive strengths)

1. **The graph is the interface, not a picture of the interface.** Agents are *created, routed, retried, amended* through the graph model. The DAG isn't a visualization of work — it *is* the work. This is the posture our library should copy: the structured graph is primary; rendering is a side effect.
2. **Minimal, stable node schema** (`id`, `prompt`, `category`, `dependsOn`) — small enough for an LLM to emit reliably, expressive enough to orchestrate hundreds of agents in parallel. Lesson: our code-graph node schema must be equally LLM-emittable.
3. **Live state + persistence via snapshots.** Event-sourced updates (`omo.dag.updated`) plus checkpointed JSON snapshots that survive session restarts — exactly the shape an agent needs for long-running code analysis: the graph is a durable artifact, queryable later.

## Where it is human-eyes-first (the gap we exploit)

- **The DAG is a status board for a human watcher.** Auto-expand running nodes, fold finished ones, progress excerpts, elapsed timers — all of it serves *a person watching agents work*. The agent itself never consumes the rendered DAG; it holds the raw state internally. We flip this: the graph model must be the agent's *working memory* (queryable, aggregatable), with the human view as the byproduct.
- **No structural queries.** "Which nodes does X transitively depend on?", "what's the critical path?", "blast radius of changing file F" — none of this exists; humans eyeball the TUI. For code graphs this is the whole game: dependency impact analysis is a *query*, not a glance.
- **Terminal-box-drawing ceiling.** Fine for ~20 agent nodes; useless for a 10k-node codebase graph. It has no aggregation semantics (collapse package → module), because a human watcher doesn't need them — but an agent drowning in 10k nodes does.
- **Edge types are impoverished.** Only "depends on". Code graphs need typed edges (imports, calls, inherits, reads/writes, spawns) — OmO's model never needed them because agent orchestration has one relationship.

## License — IMPORTANT

- **oh-my-openagent itself: Sustainable Use License 1.0 (SUL-1.0)** — confirmed from `LICENSE.md` on `dev` (GitHub shows "Other/NOASSERTION"; badge reads `license-SUL--1.0`).
  - Terms: use/modify **only for your own internal business purposes or non-commercial/personal use**; distribution to others **only free of charge for non-commercial purposes**. Non-transferable, non-sublicensable.
  - **NOT compatible with our product.** We may **not reuse its code** (not Apache-2.0/MIT/BSD). Per our standing rule (incorporate only from compatibly-licensed code with attribution): **OmO code is off-limits** — clean-room only, from observable behavior and public docs, at most.
- **jc01rho/omo-herdr-dag: MIT** (GitHub license field) — **fair to incorporate with attribution.** Its snapshot schema (`{session, run, nodeLabels, nodeStates, taskIds, dependencyEdges, errors}`), the event-subscription pattern (`omo.dag.updated`-style event bus), and the separation of dependency-edges vs containment-links are all reusable code-level material with an MIT attribution line.

## Steal-worthy ideas (incorporate)

1. **Graph-as-workspace: named mutation ops (`retry`/`send`/`amend`) instead of rebuild-from-scratch.** Our code-graph API should expose *surgical graph ops* — `expand(node)`, `collapse(subgraph)`, `annotate(node, …)`, `prune(predicate)` — that mutate the agent's working graph incrementally, the way OmO steers a live DAG. Behavior-level idea (clean-room OK); schema wording from the MIT omo-herdr-dag side is directly reusable.
2. **Event-sourced snapshots as the persistence format.** Steal from omo-herdr-dag (MIT, attributable): the graph is a JSON snapshot stream (`{runId, nodeStates, dependencyEdges, errors}`) that survives restarts and is viewable without recomputation. Our library should checkpoint the code graph the same way — an agent can resume analysis days later, and the *same snapshot* feeds both agent queries and the human render. Separate **dependency edges** from **containment links** as distinct edge types in the schema — this is the one modeling discipline worth copying verbatim.
3. **LLM-emittable minimal node schema.** `mass-ulw`'s `{id, prompt, category, dependsOn}` proves a tiny schema can orchestrate massive parallel work because it's easy for a model to emit correctly. Design our code-node schema the same way: a small required core (`id`, `kind`, `label`, `loc`) + optional typed edge list — keep the *write path* (agent emits/updates graph) trivially simple, and put richness in derived/queried views.

---
Surveyed 2026-09-26 · sources: repo README + `docs/guide/orchestration.md` (dependency graphs: mass-ulw, /dag), LICENSE.md (SUL-1.0), jc01rho/omo-herdr-dag README (MIT, snapshot schema, controls).
