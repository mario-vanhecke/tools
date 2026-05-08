#![allow(clippy::type_complexity)]

use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod exit_codes;
mod output;

fn main() {
    let exit = run();
    std::process::exit(exit);
}

fn run() -> i32 {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // rag_core at info so users see the device-pick line during
                // index; everything else at warn so transitive crates stay
                // quiet.
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rag_core=info,warn")),
        )
        .with_target(false)
        .without_time()
        .init();

    let args = cli::Cli::parse();
    match dispatch(args) {
        Ok(code) => code,
        Err(e) => {
            // The colored "error:" prefix matches the rest of the CLI surface.
            eprintln!("error: {}", e);
            // Try to extract a typed exit code from the error chain.
            for cause in e.chain() {
                if let Some(rag_err) = cause.downcast_ref::<rag_core::Error>() {
                    return exit_codes::for_error(rag_err);
                }
            }
            exit_codes::GENERAL
        }
    }
}

fn dispatch(args: cli::Cli) -> Result<i32> {
    use cli::Command::*;
    match args.command {
        Init(c) => commands::init::run(c, args.json),
        Add(c) => commands::add::run(c, args.json, args.vault.as_deref()),
        Rm(c) => commands::rm::run(c, args.json, args.vault.as_deref()),
        Prune(c) => commands::prune::run(c, args.json, args.vault.as_deref()),
        Ls(c) => commands::ls::run(c, args.json, args.vault.as_deref()),
        Status(c) => commands::status::run(c, args.json, args.vault.as_deref()),
        Index(c) => commands::index::run(c, args.json, args.vault.as_deref()),
        Search(c) => commands::search::run(c, args.json, args.vault.as_deref()),
        Show(c) => commands::show::run(c, args.json, args.vault.as_deref()),
        Config(c) => commands::config::run(c, args.json, args.vault.as_deref()),
        Info(c) => commands::info::run(c, args.json, args.vault.as_deref()),
    }
}
