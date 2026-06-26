use super::{ExtractedDocument, ExtractionResult, Extractor};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Replace `<span ...>INNER</span>` with `INNER`. Pandoc emits these as anchor
/// wrappers around heading text in EPUB output even when bracketed_spans /
/// header_attributes are disabled, because they originated as raw HTML in the
/// source XHTML. We rewrite them so the heading_path our chunker stores reads
/// as plain prose instead of mixed markdown+HTML.
fn strip_anchor_spans(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<span") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        match after.find('>') {
            Some(open_end) => {
                let after_open = &after[open_end + 1..];
                match after_open.find("</span>") {
                    Some(close) => {
                        out.push_str(&after_open[..close]);
                        rest = &after_open[close + "</span>".len()..];
                    }
                    None => {
                        // Unterminated; keep as-is and stop.
                        out.push_str(after);
                        return out;
                    }
                }
            }
            None => {
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

pub struct PandocExtractor {
    binary: PathBuf,
}

impl PandocExtractor {
    /// Construct iff `pandoc` is on PATH. If not, returns None and the index
    /// pipeline will treat docx/epub as failed with `no_extractor_available`.
    pub fn try_new() -> Option<Self> {
        crate::tool::locate("pandoc").map(|binary| Self { binary })
    }
}

impl Extractor for PandocExtractor {
    fn extensions(&self) -> &[&'static str] {
        // PDF is handled by `pdf::PdfExtractor` (pure Rust). Pandoc cannot
        // read PDF — it can only write it.
        &["docx", "epub"]
    }

    fn extract(&self, path: &Path) -> ExtractionResult {
        // Disable pandoc extensions that emit anchor-id cruft into the
        // output — header_attributes ({#id} after #), link_attributes
        // (style/class on links), bracketed_spans ({...} around inline
        // text), fenced_divs. Especially noisy for EPUBs.
        let output = Command::new(&self.binary)
            .arg(path)
            .arg("-t")
            .arg("markdown-header_attributes-link_attributes-bracketed_spans-fenced_divs")
            .arg("--wrap=none")
            .output();
        let output = match output {
            Ok(o) => o,
            Err(e) => {
                return ExtractionResult::Failed {
                    detail: "pandoc_spawn_failed".to_string(),
                    message: e.to_string(),
                };
            }
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return ExtractionResult::Failed {
                detail: "pandoc_failed".to_string(),
                message: stderr,
            };
        }
        let markdown = strip_anchor_spans(&String::from_utf8_lossy(&output.stdout));

        ExtractionResult::Ok(ExtractedDocument {
            markdown,
            metadata: json!({}),
            page_boundaries: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::strip_anchor_spans;

    #[test]
    fn strips_simple_span() {
        assert_eq!(
            strip_anchor_spans("Title <span id=\"x\">Inner</span> rest"),
            "Title Inner rest"
        );
    }
    #[test]
    fn strips_empty_span() {
        assert_eq!(
            strip_anchor_spans("## <span id=\"chap1\"></span>I. Intro"),
            "## I. Intro"
        );
    }
    #[test]
    fn strips_multiple_spans() {
        assert_eq!(
            strip_anchor_spans("<span>a</span> and <span>b</span>"),
            "a and b"
        );
    }
    #[test]
    fn passthrough_when_no_span() {
        assert_eq!(
            strip_anchor_spans("# Plain heading\n\nbody"),
            "# Plain heading\n\nbody"
        );
    }
}
