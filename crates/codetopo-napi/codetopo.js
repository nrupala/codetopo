// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// codetopo — the hand-written JavaScript layer over the generated
// napi-rs bindings (index.js). A local native library: it runs in your
// own Node process, over your files, and makes no network calls. It is
// not a client of the hosted API and it is not an MCP server; it is the
// same Rust core the CLI, HTTP, and MCP surfaces serve, bound in
// process, answering with the same CODE_SCHEMA JSON by construction.
//
// Strictness is deliberate (the 0.9.1 lesson): option objects are
// validated — an unknown option key throws an Error NAMING the key —
// and rank's metric parameter is `by` only, with no synonyms. The
// surface refuses to guess.

'use strict';

const path = require('node:path');
const native = require('./index.js');

/** Wire vocabularies — the core's snake_case values, unchanged. */
const NODE_KINDS = [
  'package', 'module', 'file', 'class', 'interface',
  'function', 'method', 'variable', 'external', 'doc',
];
const EDGE_KINDS = [
  'contains', 'imports', 'calls', 'inherits', 'implements',
  'defines', 'reads', 'writes', 'references',
];
const RANK_METRICS = [
  'fan_in', 'fan_out', 'blast_radius', 'complexity',
  'pagerank', 'betweenness', 'closeness', 'eigenvector', 'katz',
];
const DEFAULT_LIMIT = 20;

/** Provenance, exported from the bound core — never literals here. */
const CORE_VERSION = native.coreVersion();
const SCHEMA_VERSION = native.schemaVersion();

/**
 * Decode a native failure into a real Error with a stable `code`.
 * The native layer encodes its errors as `{"code", "message"}` JSON in
 * the error reason; anything else becomes an `internal` error carrying
 * the original text.
 */
function decodeError(err) {
  const raw = err && err.message ? err.message : String(err);
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed.code === 'string' && typeof parsed.message === 'string') {
      const out = new Error(parsed.message);
      out.code = parsed.code;
      return out;
    }
  } catch (_) {
    /* not an envelope — fall through */
  }
  const out = new Error(raw);
  out.code = 'internal';
  return out;
}

function call(fn) {
  try {
    return fn();
  } catch (err) {
    throw decodeError(err);
  }
}

async function callAsync(promise) {
  try {
    return await promise;
  } catch (err) {
    throw decodeError(err);
  }
}

function parse(jsonText) {
  return JSON.parse(jsonText);
}

/**
 * Validate an option object against its allowed keys. An unknown key
 * throws an Error naming the key — the surface refuses to guess what
 * was meant (a silently ignored `metric:` is exactly the defect the
 * 0.9.1 rank fix removed from the HTTP surface).
 */
function validateOptions(options, allowed, where) {
  if (options === undefined || options === null) return {};
  if (typeof options !== 'object' || Array.isArray(options)) {
    const err = new Error(`${where}: options must be a plain object`);
    err.code = 'invalid_param';
    throw err;
  }
  for (const key of Object.keys(options)) {
    if (!allowed.includes(key)) {
      const err = new Error(
        `${where}: unknown option '${key}' (allowed: ${allowed.join(', ')})`
      );
      err.code = 'invalid_param';
      throw err;
    }
  }
  return options;
}

function limitOf(options, where) {
  const limit = options.limit === undefined ? DEFAULT_LIMIT : options.limit;
  if (!Number.isInteger(limit) || limit < 0) {
    const err = new Error(`${where}: 'limit' must be a non-negative integer, got ${JSON.stringify(options.limit)}`);
    err.code = 'invalid_param';
    throw err;
  }
  return limit;
}

function expandOf(options, where) {
  const expand = options.expand === undefined ? false : options.expand;
  if (typeof expand !== 'boolean') {
    const err = new Error(`${where}: 'expand' must be a boolean, got ${JSON.stringify(options.expand)}`);
    err.code = 'invalid_param';
    throw err;
  }
  return expand;
}

function requireId(id, where) {
  if (typeof id !== 'string' || id.length === 0) {
    const err = new Error(`${where}: a non-empty symbol id string is required`);
    err.code = 'missing_param';
    throw err;
  }
  return id;
}

/**
 * A codetopo graph handle: a frozen code graph loaded in this process,
 * plus the index it came from. Queries are plain JSON in, plain JSON
 * out — the exact objects the CLI, HTTP, and MCP surfaces return.
 * Absent measurements stay absent (`undefined` fields are omitted by
 * the core's serializers), never coerced to 0.
 */
class Codetopo {
  constructor(nativeHandle, extras) {
    this._handle = nativeHandle;
    if (extras && extras.dbPath) this.dbPath = extras.dbPath;
    if (extras && extras.indexReport) this.indexReport = extras.indexReport;
  }

  /**
   * Index a repository into a queryable graph (heavy — asynchronous).
   * Writes a SQLite index in the shared CLI/store format, so the CLI,
   * MCP, and JS surfaces open each other's indexes.
   *
   * @param {string} repoPath local repository path
   * @param {{dbPath?: string, package?: string}} [options]
   *   dbPath defaults to `<repoPath>/codetopo.db`; package defaults to
   *   the repository directory's name, as on the other surfaces.
   * @returns {Promise<Codetopo>} the handle; its `indexReport` carries
   *   `{package, filesIndexed, filesFailed, nodes, edges, diagnostics}`.
   */
  static async index(repoPath, options) {
    const opts = validateOptions(options, ['dbPath', 'package'], 'Codetopo.index');
    if (typeof repoPath !== 'string' || repoPath.length === 0) {
      const err = new Error('Codetopo.index: a repository path string is required');
      err.code = 'missing_param';
      throw err;
    }
    const dbPath = opts.dbPath === undefined ? path.join(repoPath, 'codetopo.db') : opts.dbPath;
    const reportJson = await callAsync(
      native.indexRepoAsync(repoPath, dbPath, opts.package === undefined ? null : opts.package)
    );
    const raw = parse(reportJson);
    const indexReport = {
      package: raw.package,
      repoPath: raw.repo_path,
      dbPath: raw.db_path,
      filesIndexed: raw.files_indexed,
      filesFailed: raw.files_failed,
      nodes: raw.nodes,
      edges: raw.edges,
      diagnostics: raw.diagnostics,
    };
    const handle = call(() => native.openHandle(dbPath));
    return new Codetopo(handle, { dbPath, indexReport });
  }

  /**
   * Open an existing index database (written by any codetopo surface).
   * @param {string} dbPath
   * @returns {Codetopo}
   */
  static open(dbPath) {
    if (typeof dbPath !== 'string' || dbPath.length === 0) {
      const err = new Error('Codetopo.open: a database path string is required');
      err.code = 'missing_param';
      throw err;
    }
    const handle = call(() => native.openHandle(dbPath));
    return new Codetopo(handle, { dbPath });
  }

  /**
   * Build a handle from a snapshot (object or JSON text). A snapshot
   * whose schema major differs from this build's throws
   * `unsupported_schema_version` (CODE_SCHEMA §8).
   * @param {object|string} snapshot
   * @returns {Codetopo}
   */
  static fromSnapshot(snapshot) {
    const text = typeof snapshot === 'string' ? snapshot : JSON.stringify(snapshot);
    if (typeof text !== 'string') {
      const err = new Error('Codetopo.fromSnapshot: a snapshot object or JSON string is required');
      err.code = 'missing_param';
      throw err;
    }
    const handle = call(() => native.handleFromSnapshot(text));
    return new Codetopo(handle, {});
  }

  /**
   * Restore a snapshot into a database at `dbPath`, through the full
   * verification gate (schema major + audit chain), then open it.
   * @param {object|string} snapshot
   * @param {string} dbPath
   * @returns {Codetopo} the restored handle; its `restoreReport`
   *   carries `{nodes, edges, verified}`.
   */
  static restore(snapshot, dbPath) {
    const text = typeof snapshot === 'string' ? snapshot : JSON.stringify(snapshot);
    if (typeof text !== 'string' || typeof dbPath !== 'string' || dbPath.length === 0) {
      const err = new Error('Codetopo.restore: a snapshot and a database path are required');
      err.code = 'missing_param';
      throw err;
    }
    const report = parse(call(() => native.restoreSnapshot(text, dbPath)));
    const topo = Codetopo.open(dbPath);
    topo.restoreReport = report;
    return topo;
  }

  /** The schema description every surface serves. Needs no database. */
  static describeSchema() {
    return parse(call(() => native.describeSchema()));
  }

  /** Aggregate-first statistics: totals plus per-kind breakdowns. */
  stats() {
    return parse(call(() => this._handle.stats()));
  }

  /**
   * Find symbols by name fragment (case-insensitive) — the discovery
   * step before the exact-id queries.
   * @param {string} query
   * @param {number|{limit?: number}} [limitOrOptions]
   */
  searchSymbols(query, limitOrOptions) {
    let limit = DEFAULT_LIMIT;
    if (typeof limitOrOptions === 'number') {
      limit = limitOrOptions;
    } else if (limitOrOptions !== undefined) {
      const opts = validateOptions(limitOrOptions, ['limit'], 'searchSymbols');
      limit = limitOf(opts, 'searchSymbols');
    }
    return parse(call(() => this._handle.searchSymbols(String(query), limit)));
  }

  /** One symbol in full: node + incident edges + measurements. */
  getSymbol(id) {
    requireId(id, 'getSymbol');
    return parse(call(() => this._handle.getSymbol(id)));
  }

  descendants(id, options) {
    requireId(id, 'descendants');
    const opts = validateOptions(options, ['limit', 'expand'], 'descendants');
    return parse(call(() => this._handle.descendants(id, limitOf(opts, 'descendants'), expandOf(opts, 'descendants'))));
  }

  ancestors(id, options) {
    requireId(id, 'ancestors');
    const opts = validateOptions(options, ['limit', 'expand'], 'ancestors');
    return parse(call(() => this._handle.ancestors(id, limitOf(opts, 'ancestors'), expandOf(opts, 'ancestors'))));
  }

  blastRadius(id, options) {
    requireId(id, 'blastRadius');
    const opts = validateOptions(options, ['limit', 'expand'], 'blastRadius');
    return parse(call(() => this._handle.blastRadius(id, limitOf(opts, 'blastRadius'), expandOf(opts, 'blastRadius'))));
  }

  /**
   * The shortest directed typed path between two symbols, or `null`
   * when no directed path exists — never a reversed-edge synthesis.
   */
  path(fromId, toId) {
    requireId(fromId, 'path');
    requireId(toId, 'path');
    return parse(call(() => this._handle.path(fromId, toId)));
  }

  /**
   * The k-hop subgraph around a symbol, both directions.
   * @param {string} id
   * @param {number|{depth?: number}} [depthOrOptions]
   */
  neighborhood(id, depthOrOptions) {
    requireId(id, 'neighborhood');
    let depth = 1;
    if (typeof depthOrOptions === 'number') {
      depth = depthOrOptions;
    } else if (depthOrOptions !== undefined) {
      const opts = validateOptions(depthOrOptions, ['depth'], 'neighborhood');
      depth = opts.depth === undefined ? 1 : opts.depth;
    }
    if (!Number.isInteger(depth) || depth < 0) {
      const err = new Error(`neighborhood: 'depth' must be a non-negative integer, got ${JSON.stringify(depth)}`);
      err.code = 'invalid_param';
      throw err;
    }
    return parse(call(() => this._handle.neighborhood(id, depth)));
  }

  /** The diagnostics recorded when this index was built, as data. */
  diagnostics() {
    return parse(call(() => this._handle.diagnostics()));
  }

  /**
   * Rank symbols by a measurement (heavy on the float metrics —
   * asynchronous). The metric parameter is `by` ONLY; any other key
   * throws, naming the key. A measure that does not converge on this
   * graph answers `{measured: false, reason}` — an absence, never a
   * partial result.
   * @param {{by: string, limit?: number}} options
   * @returns {Promise<object>}
   */
  async rank(options) {
    const opts = validateOptions(options, ['by', 'limit'], 'rank');
    const by = opts.by === undefined ? 'blast_radius' : opts.by;
    if (!RANK_METRICS.includes(by)) {
      const err = new Error(
        `rank: unknown metric '${by}' for 'by' (expected one of: ${RANK_METRICS.join(', ')})`
      );
      err.code = 'invalid_param';
      throw err;
    }
    const limit = limitOf(opts, 'rank');
    return parse(await callAsync(this._handle.rank(by, limit)));
  }

  /** Coupling and cohesion measurements for every type and package. */
  async metrics() {
    return parse(await callAsync(this._handle.metrics()));
  }

  /**
   * The Leiden partition of the dependency graph (async).
   * @param {number|{limit?: number}} [limitOrOptions]
   */
  async communities(limitOrOptions) {
    let limit = DEFAULT_LIMIT;
    if (typeof limitOrOptions === 'number') {
      limit = limitOrOptions;
    } else if (limitOrOptions !== undefined) {
      const opts = validateOptions(limitOrOptions, ['limit'], 'communities');
      limit = limitOf(opts, 'communities');
    }
    return parse(await callAsync(this._handle.communities(limit)));
  }

  /** The safe change order: SCC-condensed topo-sort, cycles flagged. */
  topoSort() {
    return parse(call(() => this._handle.topoSort()));
  }

  /**
   * The layout spec (`layered` v1 / `force` v2) or SVG-derivable JSON.
   * @param {string|{layout?: string}} [layoutOrOptions]
   */
  render(layoutOrOptions) {
    let layout = 'layered';
    if (typeof layoutOrOptions === 'string') {
      layout = layoutOrOptions;
    } else if (layoutOrOptions !== undefined) {
      const opts = validateOptions(layoutOrOptions, ['layout'], 'render');
      layout = opts.layout === undefined ? 'layered' : opts.layout;
    }
    return parse(call(() => this._handle.render(layout)));
  }

  /**
   * Change-history measurements (hotspots, change coupling) from a
   * local working tree's git history (async). With no repo path the
   * answer is the unmeasured report with its reason.
   * @param {string} [repoPath]
   * @param {{since?: string, minSupport?: number}} [options]
   */
  async history(repoPath, options) {
    let repo = repoPath === undefined ? null : repoPath;
    let opts = options;
    if (repo !== null && typeof repo === 'object') {
      opts = repo;
      repo = null;
    }
    opts = validateOptions(opts, ['since', 'minSupport'], 'history');
    const minSupport = opts.minSupport === undefined ? 2 : opts.minSupport;
    if (!Number.isInteger(minSupport) || minSupport < 1) {
      const err = new Error(`history: 'minSupport' must be a positive integer, got ${JSON.stringify(opts.minSupport)}`);
      err.code = 'invalid_param';
      throw err;
    }
    const since = opts.since === undefined ? null : opts.since;
    return parse(await callAsync(this._handle.history(repo, since, minSupport)));
  }

  /**
   * The change-impact diff against another handle (or a snapshot).
   * @param {Codetopo|object|string} other
   */
  diff(other) {
    const otherTopo = other instanceof Codetopo ? other : Codetopo.fromSnapshot(other);
    return parse(call(() => this._handle.diff(otherTopo._handle)));
  }

  /** The portable frozen form of this graph, bound to its audit head. */
  snapshot() {
    return parse(call(() => this._handle.snapshot()));
  }

  /** Verify the tamper-evident audit chain: `{entries, chain_valid, head}`. */
  verify() {
    return parse(call(() => this._handle.verify()));
  }
}

module.exports = {
  Codetopo,
  CORE_VERSION,
  SCHEMA_VERSION,
  NODE_KINDS,
  EDGE_KINDS,
  RANK_METRICS,
};
