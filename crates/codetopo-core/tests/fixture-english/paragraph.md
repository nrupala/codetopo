Every existing code-visualization tool is built for human eyes: diagrams to glance at, trees to browse, waterfalls to scan.
An AI agent working over a codebase doesn't glance — it needs answers: what calls this, what breaks if I touch that, what's the shortest path between A and B, where's the cycle.
Today the agent gets pixels, formatted text trees, or nothing at all.
codebase-mcp (Apache-2.0) proved agents want tools over dashboards and symbols over files — but it indexes symbols as nodes with no edges.
No import graph, no call graph, nothing traversable.
Our entire reason to exist is the edges.
Arize Phoenix (ELv2, clean-room only) proved the right architecture — typed entity schema → MCP/agent query surface → server-side aggregation — but its substrate is runtime telemetry, not static code structure.
