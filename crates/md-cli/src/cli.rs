use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "md", version, about = "Anything-to-markdown converter with a vault lifecycle", long_about = None)]
pub struct Cli {
    /// Override walk-up vault discovery
    #[arg(long, global = true)]
    pub vault: Option<PathBuf>,

    /// Emit JSON output to stdout
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-error output
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Additional human-readable detail
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new conversion vault
    Init(InitCmd),
    /// Register input files
    Add(AddCmd),
    /// Deregister files
    Rm(RmCmd),
    /// Remove rows in non-`converted` states
    Prune(PruneCmd),
    /// List registered files
    Ls(LsCmd),
    /// Report vault state with source/output drift
    Status(StatusCmd),
    /// Convert pending/changed files to markdown
    Convert(ConvertCmd),
    /// Display a converted file or input row
    Show(ShowCmd),
    /// Find the source of a converted .md (DB lookup or annotation parse)
    Whence(WhenceCmd),
    /// Read or modify vault settings
    Config(ConfigCmd),
    /// Vault metadata and counts
    Info(InfoCmd),
}

#[derive(Debug, Args)]
pub struct InitCmd {
    pub directory: Option<PathBuf>,
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct AddCmd {
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
    #[arg(long)]
    pub skip_unsupported: bool,
    #[arg(long)]
    pub no_ignore: bool,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct RmCmd {
    pub paths: Vec<PathBuf>,
    #[arg(long, conflicts_with = "paths")]
    pub all: bool,
    #[arg(long)]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct PruneCmd {
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub all_non_converted: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct LsCmd {
    #[arg(long)]
    pub status: Option<String>,
}

#[derive(Debug, Args)]
pub struct StatusCmd {
    #[arg(long)]
    pub filter: Option<String>,
    #[arg(long)]
    pub no_stat: bool,
}

#[derive(Debug, Args)]
pub struct ConvertCmd {
    /// Re-convert clean rows (overrides the source-changed check)
    #[arg(long)]
    pub force: bool,
    /// Retry rows in `failed` state
    #[arg(long)]
    pub retry_failed: bool,
    /// Re-convert conflicts, discarding hand edits to the output
    #[arg(long, conflicts_with = "keep_existing")]
    pub overwrite: bool,
    /// Treat hand-edited output as the new baseline (clears conflict)
    #[arg(long)]
    pub keep_existing: bool,
    #[arg(long)]
    pub paths: Vec<PathBuf>,
    #[arg(long)]
    pub no_wait: bool,
    #[arg(long, default_value = "60")]
    pub wait: u64,
}

#[derive(Debug, Args)]
pub struct ShowCmd {
    /// Either an input path (lists chunks of the row) or an output path
    /// (prints the converted file)
    pub target: String,
}

#[derive(Debug, Args)]
pub struct WhenceCmd {
    /// A converted .md file (inside a vault or anywhere on disk)
    pub path: PathBuf,
}

#[derive(Debug, Args)]
pub struct ConfigCmd {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    Get {
        key: String,
    },
    Set {
        key: String,
        value: String,
    },
    Unset {
        key: String,
    },
    List {
        #[arg(long)]
        modified: bool,
        #[arg(long)]
        defaults: bool,
    },
}

#[derive(Debug, Args)]
pub struct InfoCmd {}
