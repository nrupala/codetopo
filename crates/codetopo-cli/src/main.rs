// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! `codetopo` binary — thin clap CLI over the `codetopo_cli` library.
//! All logic lives in `lib.rs` so integration tests can drive it directly.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use codetopo_cli::{
    default_package_name, describe_schema, index_repo, load_graph_from_db, node_kind_name,
    print_diagnostics, print_id_list, print_id_list_json, print_stats, print_stats_json,
    render_path, require_node, resolve_hmac_key, restore_snapshot, snapshot_db, verify_db,
    verify_proof_file,
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
        /// Package name for symbol ids (default: file stem of <repo>).
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
    /// Aggregate graph statistics: node/edge totals and per-kind counts.
    Stats {
        #[arg(long)]
        db: PathBuf,
        /// Emit the HTTP response shape as JSON.
        #[arg(long)]
        json: bool,
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
    /// One symbol in full: kind, location, tier, defines/refs.
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
        Commands::Stats { db, json } => {
            let graph = load_graph_from_db(&db)?;
            if json {
                print_stats_json(&graph);
            } else {
                print_stats(&graph);
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
            let node = require_node(&graph, &id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(node).expect("node serializes"));
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
        Commands::Diagnostics { db } => print_diagnostics(&db)?,
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
