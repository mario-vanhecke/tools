//! Document → text extraction.
//!
//! Policy (matches the project's rules):
//!   * **In-memory first** — plain text, HTML, PDF (pure-Rust `pdf-extract`),
//!     and Office Open XML (DOCX/PPTX/XLSX parsed straight from the zip in
//!     memory) need no temp files at all.
//!   * **Bounded, self-cleaning temp-file fallback** — when a higher-fidelity
//!     external converter is available (`pdftotext` for PDF, `pandoc` for
//!     office/epub), the bytes are written to ONE uniquely-named temp file in a
//!     dedicated dir, converted, and the temp file is deleted immediately via
//!     RAII (even on error). Documents are processed one at a time, so at most
//!     one temp file exists at any instant — never "all converted files."
//!   * **Sweep on startup** clears any orphan temp files from a crashed run.

use anyhow::{Context, Result};
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// External converters resolved once at startup.
pub struct Converters {
    pub pdftotext: Option<PathBuf>,
    pub pandoc: Option<PathBuf>,
}

impl Converters {
    /// Resolve `pdftotext`/`pandoc` from PATH, then from beside this executable
    /// and a sibling `bin/` (so the shipped bundle's converters are found with
    /// no PATH config).
    pub fn detect() -> Self {
        Self {
            pdftotext: locate("pdftotext"),
            pandoc: locate("pandoc"),
        }
    }
}

fn locate(name: &str) -> Option<PathBuf> {
    if let Ok(p) = which::which(name) {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    [dir.join(&file), dir.join("bin").join(&file)]
        .into_iter()
        .find(|c| c.is_file())
}

/// Dedicated temp dir for transient conversion files.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("distill")
}

/// Remove any orphan temp files from a previous (possibly crashed) run.
pub fn sweep_temp_dir() {
    let dir = temp_dir();
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Extract text from `bytes` given a file `ext`. Returns `Ok(None)` for
/// unsupported types (they're skipped, not errors).
pub fn extract(ext: &str, bytes: &[u8], conv: &Converters) -> Result<Option<String>> {
    let ext = ext.to_ascii_lowercase();
    let text = match ext.as_str() {
        "txt" | "text" | "log" | "csv" | "tsv" | "md" | "markdown" | "rst" | "org" => {
            Some(plaintext(bytes))
        }
        "html" | "htm" | "xhtml" => Some(strip_html(&plaintext(bytes))),
        "pdf" => Some(extract_pdf(bytes, conv)?),
        "docx" | "pptx" | "xlsx" => match &conv.pandoc {
            // pandoc gives better structure; otherwise parse the zip in memory.
            Some(p) => Some(convert_via_tempfile(p, &ext, bytes, &["-t", "plain"])?),
            None => Some(extract_ooxml(&ext, bytes)?),
        },
        "epub" | "odt" | "rtf" => match &conv.pandoc {
            Some(p) => Some(convert_via_tempfile(p, &ext, bytes, &["-t", "plain"])?),
            None => None, // no in-memory path for these
        },
        _ => None,
    };
    Ok(text.map(|t| normalize_ws(&t)).filter(|t| !t.is_empty()))
}

fn plaintext(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn extract_pdf(bytes: &[u8], conv: &Converters) -> Result<String> {
    // Prefer pdftotext (higher fidelity) via a transient temp file; fall back
    // to the pure-Rust in-memory extractor.
    if let Some(bin) = &conv.pdftotext {
        match convert_via_tempfile(bin, "pdf", bytes, &["-q", "-enc", "UTF-8", "{IN}", "-"]) {
            Ok(t) if !t.trim().is_empty() => return Ok(t),
            _ => {} // fall through to pure-Rust
        }
    }
    // pdf-extract operates on bytes directly — no temp file.
    pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| anyhow::anyhow!("pdf-extract failed: {e}"))
}

/// Office Open XML in memory: unzip and pull the text-bearing XML parts,
/// stripping tags. Best-effort but dependency-free and temp-file-free.
fn extract_ooxml(ext: &str, bytes: &[u8]) -> Result<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("not a valid OOXML zip")?;
    let parts: Vec<String> = match ext {
        "docx" => vec!["word/document.xml".to_string()],
        "pptx" => (1..=200)
            .map(|i| format!("ppt/slides/slide{i}.xml"))
            .collect(),
        "xlsx" => vec!["xl/sharedStrings.xml".to_string()],
        _ => vec![],
    };
    let mut out = String::new();
    for name in parts {
        if let Ok(mut f) = zip.by_name(&name) {
            let mut xml = String::new();
            if f.read_to_string(&mut xml).is_ok() {
                out.push_str(&strip_xml(&xml));
                out.push('\n');
            }
        }
    }
    Ok(out)
}

/// Run an external converter on a transient temp file that is deleted the
/// instant this function returns (RAII via `NamedTempFile`). `args` may contain
/// the placeholder `{IN}` for the input path; otherwise the path is appended.
fn convert_via_tempfile(bin: &Path, ext: &str, bytes: &[u8], args: &[&str]) -> Result<String> {
    let dir = temp_dir();
    std::fs::create_dir_all(&dir).context("creating temp dir")?;
    let mut tmp = tempfile::Builder::new()
        .prefix("doc-")
        .suffix(&format!(".{ext}"))
        .tempfile_in(&dir)
        .context("creating temp file")?;
    tmp.write_all(bytes).context("writing temp file")?;
    tmp.flush().ok();
    let in_path = tmp.path().to_string_lossy().to_string();

    let mut cmd = Command::new(bin);
    let mut had_placeholder = false;
    for a in args {
        if *a == "{IN}" {
            cmd.arg(&in_path);
            had_placeholder = true;
        } else {
            cmd.arg(a);
        }
    }
    if !had_placeholder {
        cmd.arg(&in_path);
    }

    let output = cmd
        .output()
        .with_context(|| format!("running {}", bin.display()))?;
    // `tmp` drops here → temp file removed, even if we early-return on error.
    if !output.status.success() {
        anyhow::bail!(
            "{} exited with {}: {}",
            bin.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn strip_html(s: &str) -> String {
    strip_tags(s, true)
}
fn strip_xml(s: &str) -> String {
    strip_tags(s, false)
}

/// Remove `<...>` tags. For HTML, also drop `<script>`/`<style>` bodies.
fn strip_tags(s: &str, html: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Skip script/style bodies in HTML.
            if html {
                let rest = &s[i..];
                for tag in ["script", "style"] {
                    let open = format!("<{tag}");
                    if rest.len() >= open.len() && rest[..open.len()].eq_ignore_ascii_case(&open) {
                        let close = format!("</{tag}>");
                        if let Some(end) = rest.to_ascii_lowercase().find(&close) {
                            i += end + close.len();
                        } else {
                            i = bytes.len();
                        }
                    }
                }
            }
            // Skip to '>'.
            while i < bytes.len() && bytes[i] != b'>' {
                i += 1;
            }
            i += 1; // past '>'
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

/// Collapse runs of whitespace; keep paragraph breaks readable.
fn normalize_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines() {
        let t = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if t.is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                out.push('\n');
            }
        } else {
            blank_run = 0;
            out.push_str(&t);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv_none() -> Converters {
        Converters {
            pdftotext: None,
            pandoc: None,
        }
    }

    #[test]
    fn plaintext_and_unsupported() {
        let c = conv_none();
        assert_eq!(
            extract("txt", b"hello\n\n\nworld", &c).unwrap().as_deref(),
            Some("hello\n\nworld")
        );
        assert!(extract("xyz", b"data", &c).unwrap().is_none());
    }

    #[test]
    fn strips_html() {
        let c = conv_none();
        let html =
            b"<html><body><h1>Title</h1><p>Hi <b>there</b></p><script>bad()</script></body></html>";
        let text = extract("html", html, &c).unwrap().unwrap();
        assert!(text.contains("Title"));
        assert!(text.contains("there"));
        assert!(
            !text.contains("bad()"),
            "script body should be dropped: {text}"
        );
    }

    #[test]
    fn temp_dir_is_swept() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("orphan.tmp");
        std::fs::write(&marker, b"x").unwrap();
        sweep_temp_dir();
        assert!(!marker.exists(), "sweep should remove orphans");
    }
}
