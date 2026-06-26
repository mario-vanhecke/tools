use super::{ExtractedDocument, ExtractionResult, Extractor};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

/// PDF text extraction.
///
/// Replaces the v0.1.0–v0.1.3 attempt at routing PDFs through pandoc, which
/// never worked: pandoc can write PDF but cannot read it.
///
/// Two backends, picked at construction time:
///   - **pdftotext** (poppler) — used if `pdftotext` is on PATH. Higher
///     quality on hard PDFs (unusual fonts, embedded forms, complex layout).
///   - **pdf-extract** (pure Rust) — fallback when pdftotext is missing.
///     Works on most academic/textbook PDFs. Some PDFs trigger panics inside
///     the crate (encoding edge cases); we catch those and mark the file
///     `failed` rather than crashing the index run.
pub struct PdfExtractor {
    backend: PdfBackend,
}

enum PdfBackend {
    PdfToText(PathBuf),
    PureRust,
}

impl PdfExtractor {
    pub fn new() -> Self {
        let backend = match crate::tool::locate("pdftotext") {
            Some(p) => PdfBackend::PdfToText(p),
            None => PdfBackend::PureRust,
        };
        Self { backend }
    }

    pub fn backend_name(&self) -> &'static str {
        match self.backend {
            PdfBackend::PdfToText(_) => "pdftotext",
            PdfBackend::PureRust => "pdf-extract",
        }
    }
}

impl Default for PdfExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl Extractor for PdfExtractor {
    fn extensions(&self) -> &[&'static str] {
        &["pdf"]
    }

    fn extract(&self, path: &Path) -> ExtractionResult {
        let text_result = match &self.backend {
            PdfBackend::PdfToText(bin) => extract_with_pdftotext(bin, path),
            PdfBackend::PureRust => extract_with_pdf_extract(path),
        };

        let text = match text_result {
            Ok(t) => t,
            Err(e) => return e,
        };

        // Image-only PDFs and broken-encoding PDFs typically extract
        // approximately zero text. Anything under ~500 non-whitespace chars
        // — regardless of file size — is almost certainly not a normal
        // text-bearing PDF. (The previous file-size-based heuristic
        // overcounted "estimated pages" because PDFs include images, fonts,
        // and metadata that bloat the file far past the text-only size,
        // pushing the threshold above what a real text PDF can hit.)
        let chars = text.chars().filter(|c| !c.is_whitespace()).count();
        if chars < 500 {
            return ExtractionResult::NeedsOcr;
        }

        ExtractionResult::Ok(ExtractedDocument {
            markdown: text,
            metadata: json!({}),
            page_boundaries: None,
        })
    }
}

fn extract_with_pdftotext(bin: &Path, path: &Path) -> Result<String, ExtractionResult> {
    let output = Command::new(bin)
        .arg("-enc")
        .arg("UTF-8")
        .arg("-q") // suppress non-fatal warnings on stderr
        .arg(path)
        .arg("-") // emit to stdout
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            return Err(ExtractionResult::Failed {
                detail: "pdftotext_spawn_failed".to_string(),
                message: e.to_string(),
            });
        }
    };
    if !output.status.success() {
        return Err(ExtractionResult::Failed {
            detail: "pdftotext_failed".to_string(),
            message: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn extract_with_pdf_extract(path: &Path) -> Result<String, ExtractionResult> {
    // pdf-extract panics on some malformed inputs; isolate via catch_unwind
    // so a single bad PDF doesn't poison the whole index run.
    let path_owned = path.to_path_buf();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pdf_extract::extract_text(&path_owned)
    })) {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(e)) => Err(ExtractionResult::Failed {
            detail: "pdf_parse_failed".to_string(),
            message: format!(
                "{e}. Install poppler (`brew install poppler`) for higher-quality extraction."
            ),
        }),
        Err(_) => Err(ExtractionResult::Failed {
            detail: "pdf_parse_panic".to_string(),
            message: "pdf-extract panicked on this file (likely an unusual font encoding). \
                 Install poppler (`brew install poppler`) for higher-quality extraction."
                .to_string(),
        }),
    }
}
