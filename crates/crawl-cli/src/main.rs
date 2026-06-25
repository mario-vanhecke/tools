#![allow(clippy::type_complexity)]

use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod exit_codes;
mod output;

fn main() {
    // Restore default SIGPIPE so `crawl ls | head` exits quietly instead of
    // panicking on a broken pipe (Rust ignores SIGPIPE by default).
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    std::process::exit(run());
}

fn run() -> i32 {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("crawl_core=info,warn")),
        )
        .with_target(false)
        .without_time()
        .init();

    let args = cli::Cli::parse();
    match dispatch(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {}", e);
            for cause in e.chain() {
                if let Some(err) = cause.downcast_ref::<crawl_core::Error>() {
                    return exit_codes::for_error(err);
                }
            }
            exit_codes::GENERAL
        }
    }
}

fn dispatch(args: cli::Cli) -> Result<i32> {
    use cli::Command::*;
    let json = args.json;
    let vault = args.vault.as_deref();
    match args.command {
        Init(c) => commands::init::run(c, json),
        Source(c) => commands::source::run(c, json, vault),
        Discover(c) => commands::run::run(c, json, vault),
        Fetch(c) => commands::fetch::run(c, json, vault),
        Ls(c) => commands::ls::run(c, json, vault),
        Status(c) => commands::status::run(c, json, vault),
        Find(c) => commands::find::run(c, json, vault),
        Rm(c) => commands::rm::run(c, json, vault),
        Prune(c) => commands::prune::run(c, json, vault),
        Export(c) => commands::export::run(c, json, vault),
        Config(c) => commands::config::run(c, json, vault),
        Info(c) => commands::info::run(c, json, vault),
    }
}
