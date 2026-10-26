// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar (https://nrupalakolkar.com) · Built with Muse by Meta (https://muse.ai/join) — join Muse, then redeem code 101EEK in Settings within 48 hours of joining and we both earn tokens.

//! `codetopo` binary — thin clap CLI over the `codetopo_cli` library.
//! All logic lives in `lib.rs` so integration tests can drive it directly.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use codetopo_cli::{
    default_package_name, describe_schema, index_repo, load_graph_from_db, node_kind_name,
    print_diagnostics, print_diagnostics_json, print_id_list, print_id_list_json, print_stats,
    print_stats_json, render_path, require_node, resolve_hmac_key, restore_snapshot, snapshot_db,
    verify_db, verify_proof_file,
};

#[derive(Parser)]
#[command(
    name = "codetopo",
    version,
    about = "Agent-optimized code visualization: index source trees into a queryable topology graph."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Walk a repo, extract symbols, and store the code graph.
    Index {
        /// Repository root to walk.
        repo: PathBuf,
        /// SQLite database path for the graph + audit log.
        #[arg(long)]
        db: PathBuf,
        /// Fallback package name for symbol ids (default: file stem of
        /// <repo>). Files under a Cargo.toml / package.json manifest
        /// belong to that manifest's package instead.
        #[arg(long)]
        package: Option<String>,
        /// Also write a JSON snapshot of the graph to this path.
        #[arg(long)]
        snapshot: Option<PathBuf>,
        /// HMAC key for the snapshot proof certificate
        /// (else CODETOPO_HMAC_KEY env, else no proof).
        #[arg(long, value_name = "KEY")]
        hmac_key: Option<String>,
    },
    /// Transitive closure over calls+imports edges (what this id affects).
    Descendants {
        id: String,
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        expand: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit the HTTP response shape as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Reverse of descendants: transitive callers/importers of this id.
    Ancestors {
        id: String,
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        expand: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit the HTTP response shape as JSON.
        #[arg(long)]
        json: bool,
    },
    /// What breaks if this changes: the dependents of the id, over calls,
    /// inherits, implements, reads/writes edges (imports alone does not break).
    #[command(name = "blast-radius")]
    BlastRadius {
        id: String,
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        expand: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit the HTTP response shape as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Shortest typed path between two ids (edge-kind sequence).
    Path {
        a: String,
        b: String,
        #[arg(long)]
        db: PathBuf,
        /// Emit the HTTP response shape as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Rank symbols by a measured count — fan-in, fan-out,
    /// blast-radius size, or cyclomatic complexity. A ranking of
    /// measurements (method stated in the output), never a score of
    /// code quality.
    Rank {
        #[arg(long)]
        db: PathBuf,
        /// Metric: fan-in | fan-out | blast-radius | complexity.
        #[arg(long, default_value = "blast-radius")]
        by: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit the shared JSON shape ({metric, method, total, results}).
        #[arg(long)]
        json: bool,
    },
    /// Coupling and cohesion measurements for every type and package:
    /// CBO, RFC, LCOM pair counts, TCC, ATFD, WMC, and package
    /// Ca/Ce/instability with external coupling named separately
    /// (CODE_SCHEMA §9). Measurements with stated methods — no
    /// thresholds, no verdicts.
    Metrics {
        #[arg(long)]
        db: PathBuf,
        /// Emit the shared JSON shape ({types, packages, methods}).
        #[arg(long)]
        json: bool,
    },
    /// Compare two indexes (or two snapshot files): added, removed,
    /// and changed nodes and edges, plus the blast-radius movement of
    /// every touched symbol — change-impact analysis (Bohner & Arnold)
    /// over two frozen graphs.
    Diff {
        /// Before index database (pair with --after-db).
        #[arg(long)]
        before_db: Option<PathBuf>,
        /// After index database.
        #[arg(long)]
        after_db: Option<PathBuf>,
        /// Before snapshot file (pair with --after-snapshot).
        #[arg(long)]
        before_snapshot: Option<PathBuf>,
        /// After snapshot file.
        #[arg(long)]
        after_snapshot: Option<PathBuf>,
        /// Emit the shared JSON shape (the core GraphDiff object).
        #[arg(long)]
        json: bool,
    },
    /// Aggregate graph statistics: node/edge totals and per-kind counts.
    Stats {
        #[arg(long)]
        db: PathBuf,
        /// Emit the HTTP response shape as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Render the graph deterministically: a layout spec (JSON)
    /// and/or an SVG drawing of it. Same graph → same bytes.
    Render {
        #[arg(long)]
        db: PathBuf,
        /// Write the layout spec JSON here (default: print to stdout).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Write the SVG rendering here.
        #[arg(long)]
        svg: Option<PathBuf>,
    },
    /// Verify the tamper-evident audit chain.
    Verify {
        #[arg(long)]
        db: PathBuf,
    },
    /// Print the diagnostics recorded when the index was built.
    Diagnostics {
        #[arg(long)]
        db: PathBuf,
        /// Emit the shared JSON shape ({total, diagnostics}) — the same
        /// object the HTTP route and the MCP tool return.
        #[arg(long)]
        json: bool,
    },
    /// Everything within `depth` hops of an id, both directions
    /// (the schema's neighborhood query, now on every surface).
    Neighborhood {
        id: String,
        #[arg(long)]
        db: PathBuf,
        #[arg(long, default_value_t = 1)]
        depth: u32,
        #[arg(long)]
        json: bool,
    },
    /// One symbol in full: the node, its incident edges grouped by kind
    /// and direction, and the counts (the DESIGN §3 dossier).
    Symbol {
        id: String,
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Find symbols by name — the discovery step the id-based
    /// queries assume you already did.
    Search {
        query: String,
        #[arg(long)]
        db: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// The schema this build speaks: kinds, tiers, query vocabulary.
    Schema {
        #[arg(long)]
        json: bool,
    },
    /// Export the stored graph as a JSON snapshot (+ .proof with a key).
    Snapshot {
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_name = "KEY")]
        hmac_key: Option<String>,
    },
    /// Restore a snapshot JSON file into a database (verified first:
    /// schema, audit chain, and the proof certificate when present;
    /// the stored graph is replaced, never unioned).
    Restore {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        db: PathBuf,
        /// HMAC key the snapshot's proof certificate was signed with
        /// (else CODETOPO_HMAC_KEY env).
        #[arg(long, value_name = "KEY")]
        hmac_key: Option<String>,
        /// Restore without proof verification (audit chain only).
        #[arg(long)]
        allow_unverified: bool,
    },
    /// Verify a snapshot (schema, audit chain, proof certificate)
    /// without restoring it.
    #[command(name = "verify-proof")]
    VerifyProof {
        #[arg(long)]
        snapshot: PathBuf,
        /// HMAC key the snapshot's proof certificate was signed with
        /// (else CODETOPO_HMAC_KEY env).
        #[arg(long, value_name = "KEY")]
        hmac_key: Option<String>,
        /// Accept a snapshot with no verifiable proof (audit chain only).
        #[arg(long)]
        allow_unverified: bool,
    },
}

fn run(cli: Cli) -> Result<(), codetopo_cli::CliError> {
    match cli.command {
        Commands::Index { repo, db, package, snapshot, hmac_key } => {
            let package = package.unwrap_or_else(|| default_package_name(&repo));
            let key = resolve_hmac_key(hmac_key.as_deref());
            index_repo(&repo, &db, &package, snapshot.as_deref(), key.as_deref())?;
        }
        Commands::Descendants { id, db, expand, limit, json } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &id)?;
            let ids = graph.descendants(&id);
            if json {
                print_id_list_json("descendants", &id, &ids, expand, limit);
            } else {
                print_id_list(&format!("descendants of {id}"), &ids, expand, limit);
            }
        }
        Commands::Ancestors { id, db, expand, limit, json } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &id)?;
            let ids = graph.ancestors(&id);
            if json {
                print_id_list_json("ancestors", &id, &ids, expand, limit);
            } else {
                print_id_list(&format!("ancestors of {id}"), &ids, expand, limit);
            }
        }
        Commands::BlastRadius { id, db, expand, limit, json } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &id)?;
            let ids = graph.blast_radius(&id);
            if json {
                print_id_list_json("blast_radius", &id, &ids, expand, limit);
            } else {
                print_id_list(&format!("blast radius of {id}"), &ids, expand, limit);
            }
        }
        Commands::Path { a, b, db, json } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &a)?;
            require_node(&graph, &b)?;
            match graph.path(&a, &b) {
                Some(steps) => {
                    let rendered = if steps.is_empty() { a.clone() } else { render_path(&steps) };
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"from": a, "to": b, "steps": steps, "rendered": rendered})
                        );
                    } else {
                        println!("{rendered}");
                    }
                }
                None => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"from": a, "to": b, "steps": null, "rendered": "no path"})
                        );
                    } else {
                        println!("no path");
                    }
                }
            }
        }
        Commands::Rank { db, by, limit, json } => {
            let metric = codetopo_core::RankMetric::parse(&by).ok_or_else(|| {
                codetopo_cli::CliError::Usage(format!(
                    "unknown rank metric '{by}' (expected fan-in, fan-out, blast-radius, or complexity)"
                ))
            })?;
            let graph = load_graph_from_db(&db)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&codetopo_cli::rank_value(&graph, metric, limit))
                        .expect("rank serializes")
                );
            } else {
                println!(
                    "{} symbols ranked by {}",
                    graph.rank_total(metric),
                    metric.as_str()
                );
                println!("method: {}", metric.method());
                for r in graph.rank(metric, limit) {
                    println!("{}  {}", r.count, r.id);
                }
            }
        }
        Commands::Metrics { db, json } => {
            let graph = load_graph_from_db(&db)?;
            let metrics = graph.metrics();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&codetopo_cli::metrics_value(&graph))
                        .expect("metrics serialize")
                );
            } else {
                println!("types: {}", metrics.types.len());
                for t in &metrics.types {
                    println!(
                        "{}: methods {} · cbo {} · rfc {} · atfd {}",
                        t.id, t.method_count, t.cbo, t.rfc, t.atfd
                    );
                    match (t.wmc, t.lcom_sharing_pairs, t.lcom_non_sharing_pairs, t.tcc) {
                        (wmc, Some(sh), Some(nsh), Some(tcc)) => println!(
                            "  wmc {} · lcom {sh} sharing / {nsh} non-sharing · tcc {tcc:.2}",
                            wmc.map(|w| w.to_string()).unwrap_or_else(|| "—".to_string())
                        ),
                        (Some(w), None, None, None) => println!("  wmc {w} · cohesion not measured"),
                        (None, Some(sh), Some(nsh), Some(tcc)) => println!(
                            "  wmc not measured · lcom {sh} sharing / {nsh} non-sharing · tcc {tcc:.2}"
                        ),
                        _ => println!("  wmc not measured · cohesion not measured"),
                    }
                }
                println!("packages: {}", metrics.packages.len());
                for p in &metrics.packages {
                    match p.instability {
                        Some(i) => println!(
                            "{}: ca {} · ce {} · external-ce {} · instability {i:.2}",
                            p.id, p.ca, p.ce, p.external_ce
                        ),
                        None => println!(
                            "{}: ca {} · ce {} · external-ce {} · instability not measured",
                            p.id, p.ca, p.ce, p.external_ce
                        ),
                    }
                }
            }
        }
        Commands::Diff {
            before_db,
            after_db,
            before_snapshot,
            after_snapshot,
            json,
        } => {
            fn load_side(
                db: &Option<PathBuf>,
                snap: &Option<PathBuf>,
                side: &str,
            ) -> Result<codetopo_core::Graph, codetopo_cli::CliError> {
                match (db, snap) {
                    (Some(db), None) => load_graph_from_db(db),
                    (None, Some(snap)) => codetopo_cli::read_snapshot_graph(snap),
                    _ => Err(codetopo_cli::CliError::Usage(format!(
                        "give exactly one of --{side}-db / --{side}-snapshot"
                    ))),
                }
            }
            let before = load_side(&before_db, &before_snapshot, "before")?;
            let after = load_side(&after_db, &after_snapshot, "after")?;
            let diff = codetopo_core::diff_graphs(&before, &after);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&diff).expect("diff serializes")
                );
            } else {
                println!(
                    "nodes: +{} -{} ~{}",
                    diff.added_nodes.len(),
                    diff.removed_nodes.len(),
                    diff.changed_nodes.len()
                );
                println!(
                    "edges: +{} -{}",
                    diff.added_edges.len(),
                    diff.removed_edges.len()
                );
                for n in &diff.added_nodes {
                    println!("+ node {}", n.id);
                }
                for n in &diff.removed_nodes {
                    println!("- node {}", n.id);
                }
                for c in &diff.changed_nodes {
                    println!(
                        "~ node {} ({}:{} -> {}:{})",
                        c.id, c.before.loc.file, c.before.loc.line, c.after.loc.file,
                        c.after.loc.line
                    );
                }
                for e in &diff.added_edges {
                    println!("+ edge {} --{}--> {}", e.from, e.kind.as_str(), e.to);
                }
                for e in &diff.removed_edges {
                    println!("- edge {} --{}--> {}", e.from, e.kind.as_str(), e.to);
                }
                for i in &diff.impact {
                    println!(
                        "impact {}: blast radius {} -> {} ({:+})",
                        i.id, i.blast_radius_before, i.blast_radius_after, i.delta
                    );
                }
            }
        }
        Commands::Stats { db, json } => {
            let graph = load_graph_from_db(&db)?;
            if json {
                print_stats_json(&graph);
            } else {
                print_stats(&graph);
            }
        }
        Commands::Render { db, out, svg } => {
            let graph = load_graph_from_db(&db)?;
            let spec = codetopo_core::layout(&graph);
            let spec_json = serde_json::to_string_pretty(&spec)?;
            match &out {
                Some(path) => {
                    std::fs::write(path, format!("{spec_json}\n"))?;
                    eprintln!(
                        "layout spec: {} nodes, {} edges → {}",
                        spec.nodes.len(),
                        spec.edges.len(),
                        path.display()
                    );
                }
                None => println!("{spec_json}"),
            }
            if let Some(path) = &svg {
                std::fs::write(path, codetopo_core::render_svg(&spec))?;
                eprintln!("svg: {}", path.display());
            }
        }
        Commands::Neighborhood { id, db, depth, json } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &id)?;
            let sub = graph.neighborhood(&id, depth);
            if json {
                println!(
                    "{}",
                    serde_json::json!({"node": id, "depth": depth, "nodes": sub.nodes, "edges": sub.edges})
                );
            } else {
                println!("{} nodes within {depth} hop(s) of {id}", sub.nodes.len());
                for n in &sub.nodes {
                    println!("{}", n.id);
                }
            }
        }
        Commands::Symbol { id, db, json } => {
            let graph = load_graph_from_db(&db)?;
            let dossier = graph
                .dossier(&id)
                .ok_or_else(|| codetopo_cli::CliError::UnknownId(id.clone()))?;
            let node = &dossier.node;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&dossier).expect("dossier serializes")
                );
            } else {
                println!("{} ({})", node.label, node_kind_name(node.kind));
                println!("id: {}", node.id);
                if !node.loc.file.is_empty() {
                    println!("at: {}:{}", node.loc.file, node.loc.line);
                }
                println!("tier: {}", match node.tier {
                    codetopo_core::Tier::Core => "core",
                    codetopo_core::Tier::Recommended => "recommended",
                    codetopo_core::Tier::Optional => "optional",
                });
                println!("defines: {}", node.defines.len());
                println!("refs: {}", node.refs.len());
                if let Some(m) = &dossier.measurements {
                    if let Some(v) = m.complexity {
                        println!("complexity: {v} (cyclomatic — McCabe: decision points + 1)");
                    }
                    if let Some(s) = m.size_lines {
                        println!("size: {s} lines");
                    }
                }
                println!("outgoing: {}", dossier.outgoing_count);
                for (kind, edges) in &dossier.outgoing {
                    for e in edges {
                        match e.line {
                            Some(line) => println!("  {kind} -> {} (line {line})", e.to),
                            None => println!("  {kind} -> {}", e.to),
                        }
                    }
                }
                println!("incoming: {}", dossier.incoming_count);
                for (kind, edges) in &dossier.incoming {
                    for e in edges {
                        match e.line {
                            Some(line) => println!("  {kind} <- {} (line {line})", e.from),
                            None => println!("  {kind} <- {}", e.from),
                        }
                    }
                }
            }
        }
        Commands::Search { query, db, limit, json } => {
            let graph = load_graph_from_db(&db)?;
            let hits = graph.search_symbols(&query, limit);
            if json {
                println!(
                    "{}",
                    serde_json::json!({"query": query, "total": hits.len(), "results": hits})
                );
            } else {
                println!("{} result(s) for '{query}'", hits.len());
                for n in hits {
                    println!("{}  ({}, {}:{})", n.id, node_kind_name(n.kind), n.loc.file, n.loc.line);
                }
            }
        }
        Commands::Schema { json } => {
            let schema = describe_schema();
            if json {
                println!("{}", serde_json::to_string_pretty(&schema).expect("schema serializes"));
            } else {
                println!("schema version: {}", schema["schema_version"].as_str().unwrap_or(""));
                println!("node kinds: {}", schema["node_kinds"]);
                println!("edge kinds: {}", schema["edge_kinds"]);
                println!("queries: {}", schema["queries"]);
            }
        }
        Commands::Verify { db } => verify_db(&db)?,
        Commands::Diagnostics { db, json } => {
            if json {
                print_diagnostics_json(&db)?;
            } else {
                print_diagnostics(&db)?;
            }
        }
        Commands::Snapshot { db, out, hmac_key } => {
            let key = resolve_hmac_key(hmac_key.as_deref());
            snapshot_db(&db, &out, key.as_deref())?;
        }
        Commands::Restore { snapshot, db, hmac_key, allow_unverified } => {
            let key = resolve_hmac_key(hmac_key.as_deref());
            restore_snapshot(&snapshot, &db, key.as_deref(), allow_unverified)?;
        }
        Commands::VerifyProof { snapshot, hmac_key, allow_unverified } => {
            let key = resolve_hmac_key(hmac_key.as_deref());
            verify_proof_file(&snapshot, key.as_deref(), allow_unverified)?;
        }
    }
    Ok(())
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("codetopo: error: {e}");
        std::process::exit(e.exit_code());
    }
}
