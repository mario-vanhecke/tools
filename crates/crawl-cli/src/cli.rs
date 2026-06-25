use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "crawl",
    version,
    about = "Discover documents across local directories, network shares, and SharePoint",
    long_about = None
)]
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
    /// Create a new discovery vault
    Init(InitCmd),
    /// Manage crawl sources (where to look)
    Source(SourceCmd),
    /// Discover documents across all sources into the registry
    #[command(alias = "run")]
    Discover(RunCmd),
    /// Materialize discovered documents into a local tree (copy/download)
    Fetch(FetchCmd),
    /// List discovered documents
    Ls(LsCmd),
    /// Report vault state: sources, document counts, what's new
    Status(StatusCmd),
    /// Search discovered documents by name or path
    Find(FindCmd),
    /// Deregister documents by URI
    Rm(RmCmd),
    /// Delete documents in a terminal status (default: gone)
    Prune(PruneCmd),
    /// Emit discovered documents (paths/jsonl/csv) — feed `rag add` / `md add`
    Export(ExportCmd),
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
pub struct SourceCmd {
    #[command(subcommand)]
    pub action: SourceAction,
}

#[derive(Debug, Subcommand)]
pub enum SourceAction {
    /// Register a new source
    Add(SourceAddCmd),
    /// List registered sources
    Ls,
    /// Show one source in detail
    Show { name: String },
    /// Remove a source and all its discovered documents
    Rm { name: String },
    /// Enable a disabled source
    Enable { name: String },
    /// Disable a source (skipped by `crawl run` without --all)
    Disable { name: String },
}

#[derive(Debug, Args)]
pub struct SourceAddCmd {
    /// Short handle for the source, e.g. "team-share"
    pub name: String,
    /// Source kind: local | smb | sharepoint
    pub kind: String,
    /// Root locator: a directory path, a UNC/smb URL, or a SharePoint label
    pub uri: String,
    /// Traversal strategy: recursive | shallow | incremental | targeted
    #[arg(long)]
    pub strategy: Option<String>,
    /// Raw JSON merged into the source config (advanced)
    #[arg(long)]
    pub config: Option<String>,
    /// Set a config key, repeatable: --set tenant_id=... --set drive_id=...
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
    /// Targeted-strategy include glob (repeatable), e.g. --include '*.pdf'
    #[arg(long = "include")]
    pub include: Vec<String>,
    /// Exclude glob (repeatable)
    #[arg(long = "exclude")]
    pub exclude: Vec<String>,
    /// Cap traversal depth from the root (0 = unlimited)
    #[arg(long)]
    pub max_depth: Option<u64>,
    /// Register the source disabled
    #[arg(long)]
    pub disabled: bool,
}

#[derive(Debug, Args)]
pub struct RunCmd {
    /// Crawl only this source
    #[arg(long)]
    pub source: Option<String>,
    /// Override the strategy for this run
    #[arg(long)]
    pub strategy: Option<String>,
    /// Compute content hashes for local/smb documents this run
    #[arg(long, conflicts_with = "no_hash")]
    pub hash: bool,
    /// Skip content hashing this run
    #[arg(long)]
    pub no_hash: bool,
    /// Enumerate and report, but write nothing
    #[arg(long)]
    pub dry_run: bool,
    /// Include disabled sources
    #[arg(long)]
    pub all: bool,
    /// Discard cached SharePoint tokens and sign in again
    #[arg(long)]
    pub reauth: bool,
    /// Fail immediately if another crawl holds the lock
    #[arg(long)]
    pub no_wait: bool,
    /// Seconds to wait for the lock
    #[arg(long, default_value = "60")]
    pub wait: u64,
}

#[derive(Debug, Args)]
pub struct FetchCmd {
    /// Directory to materialize documents under (as <out>/<source>/<rel_path>)
    #[arg(long, default_value = "files")]
    pub out: PathBuf,
    /// Fetch only this source
    #[arg(long)]
    pub source: Option<String>,
    #[arg(long)]
    pub ext: Option<String>,
    /// Status filter (default: present + modified)
    #[arg(long)]
    pub status: Option<String>,
    /// Re-fetch even if an up-to-date local copy exists
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub no_wait: bool,
    #[arg(long, default_value = "60")]
    pub wait: u64,
}

#[derive(Debug, Args)]
pub struct LsCmd {
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub source: Option<String>,
    #[arg(long)]
    pub ext: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct StatusCmd {}

#[derive(Debug, Args)]
pub struct FindCmd {
    /// Substring matched against document name and URI
    pub query: String,
    #[arg(long)]
    pub ext: Option<String>,
    #[arg(long)]
    pub source: Option<String>,
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long, default_value = "50")]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct RmCmd {
    #[arg(required = true)]
    pub uris: Vec<String>,
}

#[derive(Debug, Args)]
pub struct PruneCmd {
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct ExportCmd {
    /// Output format: paths | jsonl | csv
    #[arg(long, default_value = "paths")]
    pub format: String,
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub source: Option<String>,
    #[arg(long)]
    pub ext: Option<String>,
    /// Include every status (default: only present/modified)
    #[arg(long)]
    pub all: bool,
    /// Write to a file instead of stdout
    #[arg(long)]
    pub output: Option<PathBuf>,
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
pub struct InfoCmd {
    /// Run consistency checks
    #[arg(long)]
    pub check: bool,
}
