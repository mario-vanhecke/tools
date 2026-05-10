//! `md-core` — anything-to-markdown converter with a vault lifecycle.
//!
//! Companion to `rag-core` in the same workspace. Reuses `vault-core` for
//! lifecycle plumbing (db, locking, walker, paths) and `extract-core` for
//! the actual document extractors. What `md-core` adds:
//!   - A registry of input files and their conversion state
//!   - A converter pipeline that writes `.md` outputs to a configurable dir
//!   - HTML-comment annotation on every output for portable lineage
//!   - A bidirectional lookup (`whence`) by either DB or annotation
//!
//! Unlike `rag-core`, no embedder, no chunker, no search.

#![allow(clippy::should_implement_trait)]
#![allow(clippy::type_complexity)]

pub mod annotation;
pub mod config;
pub mod convert;
pub mod db;
pub mod error;
pub mod info;
pub mod registry;
pub mod status;
pub mod vault;
pub mod whence;

pub use error::{Error, Result};
pub use vault::MdVault;

// Re-export shared crates so consumers see one coherent surface.
pub use extract_core;
pub use rusqlite;
pub use vault_core;
