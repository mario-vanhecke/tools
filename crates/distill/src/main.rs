//! `distill` — build a reference-only knowledge index (SQLite + sqlite-vec)
//! from local folders, SMB shares, and (soon) SharePoint sites, embedding via a
//! pluggable OpenAI-compatible endpoint. Sources stay at their origin; only the
//! index is written. Serve it with `recall`.

mod build;
mod extract;
mod sources;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kb_core::{locator, Config, Embedder, Index};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "distill",
    version,
    about = "Build a reference-only knowledge index from your sources"
)]
struct Cli {
    /// Path to the knowledge.toml config.
    #[arg(long, global = true, default_value = "knowledge.toml")]
    config: PathBuf,
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a starter knowledge.toml.
    Init,
    /// Build or refresh the index from all configured sources.
    Build {
        /// Re-index everything, ignoring the incremental skip checks.
        #[arg(long)]
        force: bool,
    },
    /// Search the index directly (handy for testing without `recall`).
    Search {
        query: String,
        #[arg(short = 'k', long, default_value_t = 5)]
        k: usize,
    },
    /// Show index statistics.
    Stats,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if cli.json {
                let obj = serde_json::json!({ "error": e.to_string() });
                println!("{obj}");
            } else {
                eprintln!("error: {e:#}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.cmd {
        Cmd::Init => cmd_init(&cli.config),
        Cmd::Build { force } => cmd_build(cli, *force),
        Cmd::Search { query, k } => cmd_search(cli, query, *k),
        Cmd::Stats => cmd_stats(cli),
    }
}

fn cmd_init(path: &PathBuf) -> Result<()> {
    if path.exists() {
        anyhow::bail!("{} already exists — not overwriting", path.display());
    }
    std::fs::write(path, Config::template())
        .with_context(|| format!("writing {}", path.display()))?;
    println!("Wrote {}", path.display());
    println!("Edit it, then run `distill build`.");
    Ok(())
}

fn cmd_build(cli: &Cli, force: bool) -> Result<()> {
    let cfg = Config::load(&cli.config)?;
    let report = build::build(&cfg, force)?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    for s in &report.sources {
        println!(
            "{} ({}): {} indexed, {} skipped, {} unsupported, {} pruned",
            s.name, s.kind, s.indexed, s.skipped, s.unsupported, s.pruned
        );
        for e in &s.errors {
            println!("  ! {e}");
        }
    }
    println!(
        "\nIndex {} now holds {} documents / {} chunks.",
        report.output, report.documents, report.chunks
    );
    println!("Serve it:  recall serve {} --stdio", report.output);
    Ok(())
}

fn cmd_search(cli: &Cli, query: &str, k: usize) -> Result<()> {
    let cfg = Config::load(&cli.config)?;
    let embedder = Embedder::from_config(&cfg.embedding)?;
    let index = Index::open(&cfg.output.path)?;
    let qv = embedder.embed_one(query)?;
    let hits = index.search(&qv, k)?;

    if cli.json {
        let arr: Vec<_> = hits
            .iter()
            .map(|h| {
                serde_json::json!({
                    "distance": h.distance,
                    "locator": locator::with_page(&h.locator, h.page),
                    "title": h.title,
                    "source": h.source,
                    "text": h.text,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&arr)?);
        return Ok(());
    }

    if hits.is_empty() {
        println!("No matches.");
        return Ok(());
    }
    for (i, h) in hits.iter().enumerate() {
        println!(
            "{}. [{:.4}] {}  ({})",
            i + 1,
            h.distance,
            h.title,
            locator::with_page(&h.locator, h.page)
        );
        println!("   {}", snippet(&h.text, 200));
    }
    Ok(())
}

fn cmd_stats(cli: &Cli) -> Result<()> {
    let cfg = Config::load(&cli.config)?;
    let index = Index::open(&cfg.output.path)?;
    let s = index.stats()?;
    if cli.json {
        println!(
            "{}",
            serde_json::json!({
                "output": cfg.output.path,
                "model": index.model(),
                "dims": index.dims(),
                "documents": s.documents,
                "chunks": s.chunks,
            })
        );
    } else {
        println!("Index:     {}", cfg.output.path);
        println!("Model:     {} ({} dims)", index.model(), index.dims());
        println!("Documents: {}", s.documents);
        println!("Chunks:    {}", s.chunks);
    }
    Ok(())
}

fn snippet(s: &str, max: usize) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let truncated: String = one_line.chars().take(max).collect();
        format!("{truncated}…")
    }
}
