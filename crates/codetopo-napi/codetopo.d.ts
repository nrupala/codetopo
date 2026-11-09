// Copyright (C) 2026 2351485 ALBERTA INC.
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Hand-written declarations for the codetopo JavaScript surface.
// Wire values (node kinds, edge kinds, metric names) stay the core's
// snake_case strings, so an answer pastes unchanged into an MCP call,
// an HTTP query, or a plotly.js pipeline.

/** Node kinds (CODE_SCHEMA §2). */
export type NodeKind =
  | 'package' | 'module' | 'file' | 'class' | 'interface'
  | 'function' | 'method' | 'variable' | 'external' | 'doc';

/** Edge kinds (CODE_SCHEMA §4). */
export type EdgeKind =
  | 'contains' | 'imports' | 'calls' | 'inherits' | 'implements'
  | 'defines' | 'reads' | 'writes' | 'references';

/** The nine rank metrics. The rank parameter is named `by` — only. */
export type RankMetric =
  | 'fan_in' | 'fan_out' | 'blast_radius' | 'complexity'
  | 'pagerank' | 'betweenness' | 'closeness' | 'eigenvector' | 'katz';

/** An error thrown by this surface always carries a stable `code`. */
export interface CodetopoError extends Error {
  code: string;
}

export interface IndexReport {
  package: string;
  repoPath: string;
  dbPath: string;
  filesIndexed: number;
  filesFailed: number;
  nodes: number;
  edges: number;
  diagnostics: number;
}

export interface StatsAnswer {
  nodes: { total: number; by_kind: Partial<Record<NodeKind, number>> };
  edges: { total: number; by_kind: Partial<Record<EdgeKind, number>> };
}

/** A CODE_SCHEMA node record; measured fields are absent when unmeasured. */
export interface GraphNode {
  id: string;
  kind: NodeKind;
  label: string;
  loc: { file: string; line: number };
  tier: 'core' | 'recommended' | 'optional';
  defines: string[];
  refs: string[];
  end_line?: number;
  signature?: string;
  decisions?: number;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: EdgeKind;
  line?: number;
  [key: string]: unknown;
}

export interface ClosureAnswer {
  relation: 'descendants' | 'ancestors' | 'blast_radius';
  node: string;
  total: number;
  expand: boolean;
  limit: number;
  truncated: boolean;
  nodes: Array<string | GraphNode>;
}

export interface RankAnswer {
  metric: RankMetric;
  method: string;
  total?: number;
  results?: Array<{
    id: string;
    kind: NodeKind;
    label: string;
    count?: number;
    value?: number;
  }>;
  /** Present (false) only when the measure is absent on this graph. */
  measured?: false;
  reason?: string;
}

export interface Snapshot {
  schema_version: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
  audit_head: string;
  audit_log: Array<Record<string, unknown>>;
}

export interface VerifyAnswer {
  entries: number;
  chain_valid: boolean;
  head: string;
}

/** Plain CODE_SCHEMA JSON, exactly as the other surfaces return it. */
export type JsonObject = Record<string, unknown>;

export declare class Codetopo {
  /** The database this handle was opened from, when it has one. */
  readonly dbPath?: string;
  /** Present on handles returned by `Codetopo.index`. */
  readonly indexReport?: IndexReport;
  /** Present on handles returned by `Codetopo.restore`. */
  restoreReport?: { nodes: number; edges: number; verified: boolean };

  static index(
    repoPath: string,
    options?: { dbPath?: string; package?: string }
  ): Promise<Codetopo>;
  static open(dbPath: string): Codetopo;
  static fromSnapshot(snapshot: Snapshot | string | JsonObject): Codetopo;
  static restore(
    snapshot: Snapshot | string | JsonObject,
    dbPath: string
  ): Codetopo;
  static describeSchema(): JsonObject;

  stats(): StatsAnswer;
  searchSymbols(
    query: string,
    limitOrOptions?: number | { limit?: number }
  ): { query: string; total: number; results: GraphNode[] };
  getSymbol(id: string): JsonObject;
  descendants(id: string, options?: { limit?: number; expand?: boolean }): ClosureAnswer;
  ancestors(id: string, options?: { limit?: number; expand?: boolean }): ClosureAnswer;
  blastRadius(id: string, options?: { limit?: number; expand?: boolean }): ClosureAnswer;
  path(fromId: string, toId: string): JsonObject | null;
  neighborhood(
    id: string,
    depthOrOptions?: number | { depth?: number }
  ): { node: string; depth: number; nodes: GraphNode[]; edges: GraphEdge[] };
  diagnostics(): { total: number; diagnostics: Array<Record<string, unknown>> };
  rank(options: { by: RankMetric; limit?: number }): Promise<RankAnswer>;
  metrics(): Promise<JsonObject>;
  communities(limitOrOptions?: number | { limit?: number }): Promise<JsonObject>;
  topoSort(): JsonObject & {
    total_groups: number;
    total_nodes: number;
    cyclic_groups: number;
    groups: Array<{ members: string[]; cyclic: boolean }>;
  };
  render(layoutOrOptions?: 'layered' | 'force' | { layout?: 'layered' | 'force' }): JsonObject;
  history(repoPath?: string, options?: { since?: string; minSupport?: number }): Promise<JsonObject>;
  history(options?: { since?: string; minSupport?: number }): Promise<JsonObject>;
  diff(other: Codetopo | Snapshot | string): JsonObject;
  snapshot(): Snapshot;
  verify(): VerifyAnswer;
}

/** The bound core's version, from the core crate itself. */
export declare const CORE_VERSION: string;
/** The CODE_SCHEMA version the bound core emits ("1.2"). */
export declare const SCHEMA_VERSION: string;
export declare const NODE_KINDS: NodeKind[];
export declare const EDGE_KINDS: EdgeKind[];
export declare const RANK_METRICS: RankMetric[];
