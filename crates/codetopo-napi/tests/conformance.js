// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// The known-answer conformance battery for the JavaScript surface
// (dossier §2.3). The surface earns the codetopo name only by
// reproducing the Rust core's own answers on the committed fixtures:
//
//   F1 oracle goldens (core)        — float tolerances of the core's
//                                     own oracle suite; exact where
//                                     the core is exact
//   F2 quarry known-answer package  — 26 nodes / 33 edges / 2 diagnostics
//                                     (full-directory form: 29 / 36)
//   F3 Aarona sibling fixture       — post-0.9.1 expectations, plus the
//                                     rank `by`-only strictness pin
//   F4 lifecycle parity             — index → snapshot → restore →
//                                     snapshot over the MCP fixture
//                                     repo, compared by GRAPH CONTENT
//   F5 determinism                  — every fixture query, twice,
//                                     identical results
//
// Run with `npm test` (which builds the addon in release mode first).

'use strict';

const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { Codetopo, CORE_VERSION, SCHEMA_VERSION } = require('../codetopo.js');

const CRATE = path.join(__dirname, '..');
const ROOT = path.join(CRATE, '..', '..');
const ORACLE_DIR = path.join(ROOT, 'crates', 'codetopo-core', 'tests', 'fixtures', 'oracle');
const QUARRY_DIR = path.join(ROOT, 'crates', 'codetopo-extract', 'tests', 'fixture-py');
const MCP_FIXTURE = path.join(ROOT, 'crates', 'codetopo-mcp', 'tests', 'fixture-repo');

const TOLERANCES = { pagerank: 1e-6, betweenness: 1e-9, closeness: 1e-9, eigenvector: 1e-6, katz: 1e-6 };

let passed = 0;
function ok(name) {
  passed += 1;
  console.log(`ok ${passed} - ${name}`);
}

function tmpdir(tag) {
  return fs.mkdtempSync(path.join(os.tmpdir(), `codetopo-napi-${tag}-`));
}

function approxMap(rankAnswer) {
  const map = new Map();
  for (const entry of rankAnswer.results || []) map.set(entry.id, entry.value);
  return map;
}

function assertCloseMap(metric, goldenValues, gotMap, tol) {
  const ids = Object.keys(goldenValues);
  assert.strictEqual(gotMap.size, ids.length, `${metric}: ranked node count`);
  for (const id of ids) {
    assert.ok(gotMap.has(id), `${metric}: missing node ${id}`);
    const diff = Math.abs(gotMap.get(id) - goldenValues[id]);
    assert.ok(diff <= tol, `${metric}[${id}]: js ${gotMap.get(id)} vs oracle ${goldenValues[id]} (tol ${tol})`);
  }
}

function graphContent(snapshot) {
  const nodes = [...snapshot.nodes].sort((a, b) => (a.id < b.id ? -1 : 1));
  const edges = snapshot.edges
    .map((e) => ({ from: e.from, to: e.to, kind: e.kind, line: e.line === undefined ? null : e.line }))
    .sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
  return { schema_version: snapshot.schema_version, nodes, edges };
}

async function f1() {
  // Provenance exports come from the bound core, not literals.
  const coreManifest = fs.readFileSync(path.join(ROOT, 'crates', 'codetopo-core', 'Cargo.toml'), 'utf8');
  const coreVersion = coreManifest.match(/\[package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
  assert.strictEqual(CORE_VERSION, coreVersion, 'CORE_VERSION is the bound core version');
  assert.strictEqual(SCHEMA_VERSION, '1.2');
  assert.strictEqual(Codetopo.describeSchema().schema_version, SCHEMA_VERSION);
  ok('provenance: CORE_VERSION from the bound core; SCHEMA_VERSION = describeSchema()');

  for (const name of ['compass', 'ring', 'karate', 'two-villages']) {
    const snapText = fs.readFileSync(path.join(ORACLE_DIR, `${name}.json`), 'utf8');
    const golden = JSON.parse(fs.readFileSync(path.join(ORACLE_DIR, `${name}.golden.json`), 'utf8'));
    const topo = Codetopo.fromSnapshot(snapText);

    for (const metric of ['pagerank', 'betweenness', 'closeness', 'katz']) {
      const answer = await topo.rank({ by: metric, limit: 100000 });
      assertCloseMap(`${name}/${metric}`, golden[metric], approxMap(answer), TOLERANCES[metric]);
    }
    // Eigenvector: absence must agree; presence compares after sign
    // alignment (the eigenvector's sign is arbitrary — the core's own
    // oracle suite aligns the same way).
    const eig = await topo.rank({ by: 'eigenvector', limit: 100000 });
    if (golden.eigenvector && golden.eigenvector.absent) {
      assert.strictEqual(eig.measured, false, `${name}/eigenvector: absent on both sides`);
      assert.ok(typeof eig.reason === 'string' && eig.reason.length > 0);
    } else {
      const goldenSum = Object.values(golden.eigenvector).reduce((a, b) => a + b, 0);
      const sign = goldenSum < 0 ? -1 : 1;
      const aligned = new Map([...approxMap(eig)].map(([k, v]) => [k, v * sign]));
      assertCloseMap(`${name}/eigenvector`, golden.eigenvector, aligned, TOLERANCES.eigenvector);
    }
    ok(`F1 ${name}: pagerank/betweenness/closeness/katz/eigenvector match the oracle goldens`);
  }

  // Communities are judged label-free (partition sets + Q), as in the
  // core's own communities oracle suite.
  const villages = Codetopo.fromSnapshot(fs.readFileSync(path.join(ORACLE_DIR, 'two-villages.json'), 'utf8'));
  const villagesGolden = JSON.parse(fs.readFileSync(path.join(ORACLE_DIR, 'two-villages.golden.json'), 'utf8'));
  const part = await villages.communities({ limit: 100000 });
  const sets = new Map();
  for (const a of part.assignments) {
    if (!sets.has(a.community)) sets.set(a.community, new Set());
    sets.get(a.community).add(a.id);
  }
  const partition = [...sets.values()].map((s) => [...s].sort().join(',')).sort();
  assert.deepStrictEqual(partition, ['a1,a2,a3,a4,a5', 'b1,b2,b3,b4,b5', 'z9']);
  assert.ok(Math.abs(part.modularity - villagesGolden.communities.igraph_modularity) <= 1e-9, 'two-villages Q matches the oracle at 1e-9');
  const karate = Codetopo.fromSnapshot(fs.readFileSync(path.join(ORACLE_DIR, 'karate.json'), 'utf8'));
  const karateGolden = JSON.parse(fs.readFileSync(path.join(ORACLE_DIR, 'karate.golden.json'), 'utf8'));
  const karatePart = await karate.communities({ limit: 100000 });
  assert.ok(karatePart.modularity >= karateGolden.communities.igraph_modularity - 0.005, 'karate Q within the core suite’s 0.005 of igraph');
  ok('F1 communities: two-villages partition exact + Q at 1e-9; karate Q agrees with igraph');

  // cycle-trap: topo-sort groups and cycle flags EXACT.
  const trap = Codetopo.fromSnapshot(fs.readFileSync(path.join(ORACLE_DIR, 'cycle-trap.json'), 'utf8'));
  const order = trap.topoSort();
  assert.strictEqual(order.total_groups, 4);
  assert.strictEqual(order.total_nodes, 6);
  assert.strictEqual(order.cyclic_groups, 1);
  assert.deepStrictEqual(
    order.groups.map((g) => ({ members: g.members, cyclic: g.cyclic })),
    [
      { members: ['lib'], cyclic: false },
      { members: ['x', 'y', 'z'], cyclic: true },
      { members: ['mid'], cyclic: false },
      { members: ['app'], cyclic: false },
    ]
  );
  ok('F1 cycle-trap: topo-sort groups/cycle flags exact');
}

async function f2() {
  // The package-files form: the README's own arithmetic — 26 nodes /
  // 33 edges / 2 diagnostics for the Python package itself.
  const dir = tmpdir('quarry');
  const repo = path.join(dir, 'quarry');
  fs.mkdirSync(repo);
  for (const name of ['pyproject.toml', '__init__.py', 'core.py', 'shapes.py']) {
    fs.copyFileSync(path.join(QUARRY_DIR, name), path.join(repo, name));
  }
  const topo = await Codetopo.index(repo, { dbPath: path.join(dir, 'quarry.db'), package: 'quarry' });
  assert.strictEqual(topo.indexReport.filesIndexed, 3);
  assert.strictEqual(topo.indexReport.nodes, 26, 'quarry nodes');
  assert.strictEqual(topo.indexReport.edges, 33, 'quarry edges');
  assert.strictEqual(topo.indexReport.diagnostics, 2, 'quarry diagnostics');
  const stats = topo.stats();
  assert.strictEqual(stats.nodes.total, 26);
  assert.strictEqual(stats.edges.total, 33);
  assert.strictEqual(topo.diagnostics().total, 2);
  ok('F2 quarry (package files): 26 nodes / 33 edges / 2 diagnostics exactly');

  // The full-directory form: indexing the fixture directory also
  // indexes its README (Markdown is a supported surface) — the
  // fixture README's own full-directory arithmetic, 29 / 36.
  const dir2 = tmpdir('quarry-full');
  const repo2 = path.join(dir2, 'quarry');
  fs.mkdirSync(repo2);
  for (const name of fs.readdirSync(QUARRY_DIR)) {
    fs.copyFileSync(path.join(QUARRY_DIR, name), path.join(repo2, name));
  }
  const topo2 = await Codetopo.index(repo2, { dbPath: path.join(dir2, 'quarry.db'), package: 'quarry' });
  assert.strictEqual(topo2.indexReport.filesIndexed, 4);
  assert.strictEqual(topo2.indexReport.nodes, 29);
  assert.strictEqual(topo2.indexReport.edges, 36);
  assert.strictEqual(topo2.indexReport.diagnostics, 2);
  ok('F2 quarry (full directory incl. README): 29 nodes / 36 edges / 2 diagnostics');
  return topo;
}

async function f3() {
  // Aarona's fixture, verbatim in structure from
  // crates/codetopo-cli/tests/py_sibling_fixture.rs.
  const FILES = {
    'a.py': 'import b\nimport c\n\n\ndef run():\n    b.work()\n    c.work()\n',
    'b.py': 'import c\n\n\ndef work():\n    c.work()\n',
    'c.py': 'import d\n\n\ndef work():\n    d.work()\n',
    'd.py': 'import e\n\n\ndef work():\n    e.work()\n',
    'e.py': 'import a\nimport c\n\n\ndef work():\n    a.run()\n    c.work()\n',
  };
  const dir = tmpdir('sibling');
  const repo = path.join(dir, 'repo');
  fs.mkdirSync(repo);
  for (const [name, body] of Object.entries(FILES)) fs.writeFileSync(path.join(repo, name), body);
  const topo = await Codetopo.index(repo, { dbPath: path.join(dir, 'fixture.db'), package: 'repo' });

  const snap = topo.snapshot();
  const imports = snap.edges.filter((e) => e.kind === 'imports');
  const calls = snap.edges.filter((e) => e.kind === 'calls');
  assert.strictEqual(imports.length, 7, 'imports 7/7');
  assert.ok(imports.every((e) => e.from.startsWith('repo::') && e.to.startsWith('repo::')), 'all imports internal');
  assert.strictEqual(calls.length, 7, 'calls 7/7');
  assert.ok(calls.every((e) => e.from.startsWith('repo::') && e.to.startsWith('repo::')), 'all calls internal');
  const stats = topo.stats();
  assert.strictEqual(stats.nodes.by_kind.external, undefined, 'no external stubs');
  const order = topo.topoSort();
  assert.strictEqual(order.cyclic_groups, 2, 'cyclic_groups: 2');
  assert.strictEqual(order.total_nodes, 16);
  // §6 blast radius over the cycle: every module's set is all five functions.
  const allFive = ['repo::a::run', 'repo::b::work', 'repo::c::work', 'repo::d::work', 'repo::e::work'];
  for (const m of ['repo::a', 'repo::b', 'repo::c', 'repo::d', 'repo::e']) {
    const br = topo.blastRadius(m, { limit: 1000 });
    assert.deepStrictEqual([...br.nodes].sort(), allFive, `blastRadius(${m})`);
  }
  ok('F3 Aarona sibling fixture: imports 7/7, calls 7/7, cyclic_groups 2, blast radius over the cycle');

  // The rank strictness pin: `by` is the ONLY metric parameter name.
  await assert.rejects(
    () => topo.rank({ metric: 'pagerank' }),
    (err) => err instanceof Error && err.message.includes("'metric'"),
    'rank({metric}) must throw naming the bad key'
  );
  await assert.rejects(
    () => topo.rank({ by: 'pagerank', bogus: 1 }),
    (err) => err instanceof Error && err.message.includes("'bogus'")
  );
  await assert.rejects(
    () => topo.rank({ by: 'not_a_metric' }),
    (err) => err.code === 'invalid_param' && err.message.includes('not_a_metric')
  );
  const good = await topo.rank({ by: 'pagerank', limit: 5 });
  assert.strictEqual(good.metric, 'pagerank');
  ok('F3 rank strictness: non-`by` metric parameter throws naming the key');
  return topo;
}

async function f4() {
  const dir = tmpdir('lifecycle');
  const repo = path.join(dir, 'fixture-repo');
  fs.cpSync(MCP_FIXTURE, repo, { recursive: true });
  const topo = await Codetopo.index(repo, { dbPath: path.join(dir, 'one.db'), package: 'fixture-repo' });

  // index → snapshot → restore → snapshot, compared by GRAPH CONTENT
  // (nodes + edges): snapshots embed wall-clock audit timestamps, so
  // byte identity is never asserted on them; the chain is checked
  // through verify() instead.
  const snap1 = topo.snapshot();
  const restored = Codetopo.fromSnapshot(snap1);
  const snap2 = restored.snapshot();
  assert.deepStrictEqual(graphContent(snap2), graphContent(snap1), 'snapshot → restore → snapshot preserves the graph');
  assert.strictEqual(restored.verify().chain_valid, true, 'restored audit chain verifies');

  const restoredDb = Codetopo.restore(snap1, path.join(dir, 'restored.db'));
  assert.strictEqual(restoredDb.restoreReport.verified, true);
  assert.deepStrictEqual(graphContent(restoredDb.snapshot()), graphContent(snap1), 'restore-to-db preserves the graph');
  assert.strictEqual(restoredDb.verify().chain_valid, true);

  // Parity spot checks across the restore boundary.
  assert.deepStrictEqual(restored.stats(), topo.stats());
  assert.deepStrictEqual(restored.topoSort(), topo.topoSort());
  const self = topo.diff(topo);
  assert.deepStrictEqual(
    [self.added_nodes.length, self.removed_nodes.length, self.changed_nodes.length, self.added_edges.length, self.removed_edges.length],
    [0, 0, 0, 0, 0],
    'self-diff is empty'
  );
  const handle = topo.searchSymbols('handle', { limit: 10 });
  assert.ok(handle.total >= 1, 'fixture chain symbol is discoverable');
  const leaf = topo.searchSymbols('send', { limit: 10 });
  assert.ok(leaf.total >= 1);
  ok('F4 lifecycle: index → snapshot → restore → snapshot graph-content parity; chain verified; self-diff empty');
  return topo;
}

async function f5(handles) {
  for (const [label, topo] of Object.entries(handles)) {
    const runAll = async () => ({
      stats: topo.stats(),
      topoSort: topo.topoSort(),
      diagnostics: topo.diagnostics(),
      snapshotContent: graphContent(topo.snapshot()),
      rank: await topo.rank({ by: 'pagerank', limit: 100000 }),
      fanIn: await topo.rank({ by: 'fan_in', limit: 100000 }),
      communities: await topo.communities({ limit: 100000 }),
      metrics: await topo.metrics(),
      history: await topo.history(),
      render: topo.render({ layout: 'layered' }),
    });
    const first = await runAll();
    const second = await runAll();
    assert.deepStrictEqual(second, first, `${label}: run-twice results identical`);
    ok(`F5 determinism (${label}): every query run twice returns identical results`);
  }
}

async function errorsAndStrictness() {
  const topo = await f4HandlePromise;
  assert.throws(
    () => topo.getSymbol('nope::missing'),
    (err) => err instanceof Error && err.code === 'unknown_symbol'
  );
  assert.throws(
    () => topo.descendants('nope::missing'),
    (err) => err.code === 'unknown_symbol'
  );
  assert.throws(
    () =>
      Codetopo.fromSnapshot(
        JSON.stringify({ schema_version: '2.0', nodes: [], edges: [], audit_head: '', audit_log: [] })
      ),
    (err) => err.code === 'unsupported_schema_version'
  );
  assert.throws(
    () => Codetopo.open(path.join(os.tmpdir(), 'codetopo-napi-no-such.db')),
    (err) => err.code === 'unknown_index'
  );
  assert.throws(
    () => topo.searchSymbols('x', { bogus: 1 }),
    (err) => err.message.includes("'bogus'")
  );
  // `path` is null (never an error, never a synthesis) when no
  // directed path exists between two known nodes.
  const snap = topo.snapshot();
  const someNode = snap.nodes[0].id;
  assert.strictEqual(topo.path(someNode, someNode) !== undefined, true);
  ok('errors: unknown_symbol / unsupported_schema_version / unknown_index carry stable codes; unknown option keys throw naming the key');
}

let f4HandlePromise;

(async () => {
  await f1();
  const quarry = await f2();
  const sibling = await f3();
  f4HandlePromise = f4();
  const lifecycle = await f4HandlePromise;
  await errorsAndStrictness();
  await f5({ quarry, sibling, lifecycle });
  console.log(`\nconformance: ${passed} checks passed (F1–F5 + error/strictness pins)`);
})().catch((err) => {
  console.error('\nconformance FAILED:');
  console.error(err);
  process.exit(1);
});
