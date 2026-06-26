//! The knowledge backend: an opened index + an embedder built from the config
//! the index recorded at build time, so queries are embedded the same way the
//! documents were — using nothing but the `.kb` file.

use anyhow::{Context, Result};
use kb_core::{DocumentText, Embedder, Hit, Index};
use std::path::Path;

pub struct Backend {
    index: Index,
    embedder: Embedder,
}

impl Backend {
    pub fn open(index_path: impl AsRef<Path>) -> Result<Self> {
        let index = Index::open(index_path).context("opening index")?;
        let emb_cfg = index.embedding_config()?;
        let embedder = Embedder::from_config(&emb_cfg).context("embedding config")?;
        Ok(Self { index, embedder })
    }

    pub fn model(&self) -> &str {
        self.index.model()
    }

    pub fn search(&self, query: &str, k: usize) -> Result<Vec<Hit>> {
        let v = self.embedder.embed_one(query)?;
        Ok(self.index.search(&v, k.max(1))?)
    }

    pub fn get(&self, locator: &str) -> Result<Option<DocumentText>> {
        Ok(self.index.document_text(locator)?)
    }

    pub fn doc_count(&self) -> i64 {
        self.index.stats().map(|s| s.documents).unwrap_or(0)
    }
}
