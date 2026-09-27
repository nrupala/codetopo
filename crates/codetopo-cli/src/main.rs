// Copyright (C) 2026 Nrupal Akolkar
// SPDX-License-Identifier: AGPL-3.0-or-later
// Part of codetopo — Owned by Nrupal Akolkar · Built with Muse by Meta.

//! `codetopo` binary — thin clap CLI over the `codetopo_cli` library.
//! All logic lives in `lib.rs` so integration tests can drive it directly.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use codetopo_cli::{
    index_repo, load_graph_from_db, print_id_list, print_stats, render_path, require_node,
    resolve_hmac_key, restore_snapshot, snapshot_db, verify_db,
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
    },
    /// Breakage propagation: descendants restricted to calls, inherits,
    /// implements, reads/writes edges (imports alone does not break).
    #[command(name = "blast-radius")]
    BlastRadius {
        id: String,
        #[arg(long)]
        db: PathBuf,
        #[arg(long)]
        expand: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Shortest typed path between two ids (edge-kind sequence).
    Path {
        a: String,
        b: String,
        #[arg(long)]
        db: PathBuf,
    },
    /// Aggregate graph statistics: node/edge totals and per-kind counts.
    Stats {
        #[arg(long)]
        db: PathBuf,
    },
    /// Verify the tamper-evident audit chain.
    Verify {
        #[arg(long)]
        db: PathBuf,
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
    /// Restore a snapshot JSON file into a (fresh) database.
    Restore {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        db: PathBuf,
    },
}

fn run(cli: Cli) -> Result<(), codetopo_cli::CliError> {
    match cli.command {
        Commands::Index { repo, db, package, snapshot, hmac_key } => {
            let package = package.unwrap_or_else(|| {
                repo.file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "repo".to_string())
            });
            let key = resolve_hmac_key(hmac_key.as_deref());
            index_repo(&repo, &db, &package, snapshot.as_deref(), key.as_deref())?;
        }
        Commands::Descendants { id, db, expand, limit } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &id)?;
            print_id_list(&format!("descendants of {id}"), &graph.descendants(&id), expand, limit);
        }
        Commands::Ancestors { id, db, expand, limit } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &id)?;
            print_id_list(&format!("ancestors of {id}"), &graph.ancestors(&id), expand, limit);
        }
        Commands::BlastRadius { id, db, expand, limit } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &id)?;
            print_id_list(
                &format!("blast radius of {id}"),
                &graph.blast_radius(&id),
                expand,
                limit,
            );
        }
        Commands::Path { a, b, db } => {
            let graph = load_graph_from_db(&db)?;
            require_node(&graph, &a)?;
            require_node(&graph, &b)?;
            match graph.path(&a, &b) {
                Some(steps) if steps.is_empty() => println!("{a}"),
                Some(steps) => println!("{}", render_path(&steps)),
                None => println!("no path"),
            }
        }
        Commands::Stats { db } => print_stats(&load_graph_from_db(&db)?),
        Commands::Verify { db } => verify_db(&db)?,
        Commands::Snapshot { db, out, hmac_key } => {
            let key = resolve_hmac_key(hmac_key.as_deref());
            snapshot_db(&db, &out, key.as_deref())?;
        }
        Commands::Restore { snapshot, db } => {
            restore_snapshot(&snapshot, &db)?;
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
