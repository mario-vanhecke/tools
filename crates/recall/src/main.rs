//! `recall` — serve a `distill`-built knowledge index to an LLM harness over
//! MCP. Local via stdio (the harness spawns it), or remote via HTTP. The model
//! calls `kb_search` / `kb_get` itself and cites the origin locators.

mod backend;
mod mcp;
mod server;

use anyhow::{Context, Result};
use backend::Backend;
use clap::{Parser, Subcommand};
use kb_core::{locator, Config};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "recall",
    version,
    about = "Serve a knowledge index to an LLM harness via MCP"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Serve the index over MCP. Defaults to stdio (local); use --http to
    /// expose it to remote clients.
    Serve {
        /// Path to the .kb index. If omitted, taken from `output.path` in
        /// --config.
        index: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        /// Local transport (default).
        #[arg(long)]
        stdio: bool,
        /// Remote transport: bind address, e.g. 0.0.0.0:7077.
        #[arg(long, value_name = "ADDR")]
        http: Option<String>,
    },
    /// One-off search against the index (debugging; not MCP).
    Search {
        query: String,
        index: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(short = 'k', long, default_value_t = 5)]
        k: usize,
    },
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Serve {
            index,
            config,
            stdio,
            http,
        } => {
            let path = resolve_index(index, config)?;
            let backend = Backend::open(&path)?;
            match http {
                Some(addr) => {
                    eprintln!(
                        "recall: serving {} ({} docs, model {}) over HTTP",
                        path.display(),
                        backend.doc_count(),
                        backend.model()
                    );
                    server::serve_http(&backend, &addr)
                }
                None => {
                    let _ = stdio; // stdio is the default whether or not the flag is set
                    eprintln!(
                        "recall: serving {} ({} docs, model {}) over stdio",
                        path.display(),
                        backend.doc_count(),
                        backend.model()
                    );
                    server::serve_stdio(&backend)
                }
            }
        }
        Cmd::Search {
            query,
            index,
            config,
            k,
        } => {
            let path = resolve_index(index, config)?;
            let backend = Backend::open(&path)?;
            let hits = backend.search(&query, k)?;
            if hits.is_empty() {
                println!("No matches.");
            }
            for (i, h) in hits.iter().enumerate() {
                println!(
                    "{}. [{:.4}] {}  ({})",
                    i + 1,
                    h.distance,
                    h.title,
                    locator::with_page(&h.locator, h.page)
                );
            }
            Ok(())
        }
    }
}

/// Resolve the index path: an explicit path wins; otherwise read `output.path`
/// from the config (defaulting to ./knowledge.toml).
fn resolve_index(index: Option<PathBuf>, config: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = index {
        return Ok(p);
    }
    let cfg_path = config.unwrap_or_else(|| PathBuf::from("knowledge.toml"));
    let cfg = Config::load(&cfg_path).with_context(|| {
        format!(
            "no index path given and could not read {}",
            cfg_path.display()
        )
    })?;
    Ok(PathBuf::from(cfg.output.path))
}
