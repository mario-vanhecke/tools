pub mod markdown;
pub mod pandoc;
pub mod pdf;
pub mod plaintext;
pub mod tool;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedDocument {
    pub markdown: String,
    pub metadata: serde_json::Value,
    pub page_boundaries: Option<Vec<PageBoundary>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageBoundary {
    pub page: u32,
    pub start_offset: usize,
    pub end_offset: usize,
}

#[derive(Debug)]
pub enum ExtractionResult {
    Ok(ExtractedDocument),
    NeedsOcr,
    Failed { detail: String, message: String },
}

pub trait Extractor: Send + Sync {
    fn extensions(&self) -> &[&'static str];
    fn extract(&self, path: &Path) -> ExtractionResult;
}

pub struct ExtractorRegistry {
    map: HashMap<String, Arc<dyn Extractor>>,
}

impl ExtractorRegistry {
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Standard registry: markdown, plaintext, PDF (pure Rust), and — if
    /// pandoc is available (on PATH or bundled beside the binary) — DOCX/EPUB.
    pub fn standard() -> Self {
        let mut s = Self::empty();
        let md: Arc<dyn Extractor> = Arc::new(markdown::MarkdownExtractor);
        let txt: Arc<dyn Extractor> = Arc::new(plaintext::PlaintextExtractor);
        let pdf: Arc<dyn Extractor> = Arc::new(pdf::PdfExtractor::new());
        for ext in md.extensions() {
            s.map.insert((*ext).to_string(), md.clone());
        }
        for ext in txt.extensions() {
            s.map.insert((*ext).to_string(), txt.clone());
        }
        for ext in pdf.extensions() {
            s.map.insert((*ext).to_string(), pdf.clone());
        }
        if let Some(p) = pandoc::PandocExtractor::try_new() {
            let p: Arc<dyn Extractor> = Arc::new(p);
            for ext in p.extensions() {
                s.map.insert((*ext).to_string(), p.clone());
            }
        }
        s
    }

    pub fn register(&mut self, ext: &str, e: Arc<dyn Extractor>) {
        self.map.insert(ext.to_string(), e);
    }

    pub fn for_extension(&self, ext: &str) -> Option<&Arc<dyn Extractor>> {
        self.map.get(&ext.to_lowercase())
    }
}
