//! HTML-comment annotation written at the top of every converted .md file.
//!
//! Format (single line):
//!     <!-- md: source=<path> hash=<sha256-12> extractor=<name> at=<unix-ms> -->
//!
//! Designed for portability: travels with the file via `cp`/`mv`/sync, parses
//! cheaply, invisible in rendered markdown. The DB has the canonical metadata
//! (this is the recovery / external-lookup mechanism — see `whence`).

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Annotation {
    pub source: String,
    pub source_hash: String,
    pub extractor: String,
    pub converted_at_ms: i64,
}

impl Annotation {
    /// Render as a single HTML comment line, terminated with one newline.
    /// The hash is truncated to 12 chars in the comment for readability;
    /// the full hash lives in the DB.
    pub fn render(&self) -> String {
        let hash_short = &self.source_hash[..self.source_hash.len().min(12)];
        format!(
            "<!-- md: source={} hash={} extractor={} at={} -->\n",
            sanitize(&self.source),
            sanitize(hash_short),
            sanitize(&self.extractor),
            self.converted_at_ms
        )
    }

    /// Parse the first line of `text` as an annotation, if present.
    pub fn parse(text: &str) -> Option<Annotation> {
        let first = text.lines().next()?;
        let inner = first.strip_prefix("<!-- md:")?.strip_suffix("-->")?;
        let inner = inner.trim();
        let mut kv: HashMap<&str, &str> = HashMap::new();
        for token in inner.split_whitespace() {
            if let Some((k, v)) = token.split_once('=') {
                kv.insert(k, v);
            }
        }
        Some(Annotation {
            source: kv.get("source")?.to_string(),
            source_hash: kv.get("hash").map(|s| s.to_string()).unwrap_or_default(),
            extractor: kv
                .get("extractor")
                .map(|s| s.to_string())
                .unwrap_or_default(),
            converted_at_ms: kv.get("at").and_then(|s| s.parse().ok()).unwrap_or(0),
        })
    }
}

/// Replace whitespace and comment-terminators in token values so they survive
/// a single-line round-trip. Source paths with spaces get %20 etc.; full hex
/// hashes and extractor names are already safe.
fn sanitize(s: &str) -> String {
    s.replace(' ', "%20")
        .replace('\t', "%09")
        .replace('\n', "%0a")
        .replace("-->", "--&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_simple() {
        let a = Annotation {
            source: "books/foo.pdf".into(),
            source_hash: "a1b2c3d4e5f6".into(),
            extractor: "pdftotext".into(),
            converted_at_ms: 1_700_000_000_000,
        };
        let s = a.render();
        assert!(s.starts_with("<!-- md:"));
        assert!(s.ends_with("-->\n"));
        let parsed = Annotation::parse(&s).unwrap();
        assert_eq!(parsed.source, "books/foo.pdf");
        assert_eq!(parsed.extractor, "pdftotext");
        assert_eq!(parsed.converted_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn round_trip_path_with_spaces() {
        let a = Annotation {
            source: "books/My Book.pdf".into(),
            source_hash: "abcdef".into(),
            extractor: "pdftotext".into(),
            converted_at_ms: 0,
        };
        let s = a.render();
        let parsed = Annotation::parse(&s).unwrap();
        // Sanitizer keeps it as a single token; whitespace is encoded as %20.
        assert_eq!(parsed.source, "books/My%20Book.pdf");
    }

    #[test]
    fn no_annotation_returns_none() {
        assert!(Annotation::parse("# Title\n\nbody").is_none());
        assert!(Annotation::parse("").is_none());
    }

    #[test]
    fn ignores_other_html_comments() {
        let s = "<!-- some other comment -->\n# body";
        assert!(Annotation::parse(s).is_none());
    }
}
