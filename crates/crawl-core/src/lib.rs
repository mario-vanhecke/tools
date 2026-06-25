//! `crawl-core` — document *discovery* with a vault lifecycle.
//!
//! The third tool in the same family as `rag-core` (index + search) and
//! `md-core` (convert). Where they operate on files you already have, `crawl`
//! goes and *finds* documents: it registers crawl **sources** (local
//! directories, mounted network/SMB shares, SharePoint drives), applies a
//! per-source **strategy** to enumerate them, and records discovered
//! **documents** into a registry that can feed `rag add` / `md add`.
//!
//! It reuses `vault-core` for the lifecycle plumbing (SQLite connection,
//! migrations runner, walk-up discovery, file locking, gitignore-style
//! matcher). What `crawl-core` adds:
//!   - A two-table model: `sources` (where to look) and `documents` (what was
//!     found), joined by crawl `runs` for provenance.
//!   - A `Crawler` trait with one implementation per source kind, and a set of
//!     traversal strategies (recursive / shallow / incremental / targeted).
//!   - An orchestrator that upserts discoveries and tracks a document
//!     lifecycle (`present` / `modified` / `gone` / `too_large` / `error`).
//!
//! No embedder, no chunker, no conversion — discovery only.

#![allow(clippy::should_implement_trait)]

pub mod config;
pub mod crawl;
pub mod db;
pub mod error;
pub mod info;
pub mod registry;
pub mod source;
pub mod status;
pub mod vault;

pub use error::{Error, Result};
pub use source::{Source, SourceKind, Strategy, StrategyParams};
pub use status::DocStatus;
pub use vault::CrawlVault;

// Re-export shared crates so consumers see one coherent surface.
pub use rusqlite;
pub use vault_core;
