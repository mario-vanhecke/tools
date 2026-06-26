//! `kb-core` — the shared core for the `distill` (producer) and `recall`
//! (MCP server) tools.
//!
//! Design, deliberately unlike the rag/md/crawl family:
//!   * **One declarative config** (`knowledge.toml`) lists the sources.
//!   * **Reference-only** — sources stay at their origin; the index stores a
//!     `locator` (file://, smb://, SharePoint webUrl) back to each document,
//!     never a copy of it.
//!   * **Pluggable embeddings** over an OpenAI-compatible HTTP endpoint (e.g.
//!     a local Ollama), so there is no multi-gigabyte model baked into the
//!     binary.
//!   * **SQLite + sqlite-vec** is the single, portable index artifact.

pub mod chunk;
pub mod config;
pub mod embed;
pub mod error;
pub mod index;
pub mod locator;

pub use config::{Config, EmbeddingConfig, SourceConfig};
pub use embed::Embedder;
pub use error::{Error, Result};
pub use index::{DocumentText, Hit, Index};
