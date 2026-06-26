//! A **locator** is the reference-only pointer the index stores back to a
//! document at its origin. The source file itself is never copied.

use std::path::Path;

/// Build a `file://` URL for a local/SMB path. On Windows this yields
/// `file:///C:/...`; on Unix `file:///home/...`. Spaces and other reserved
/// characters are percent-encoded so the locator is a valid, clickable URL.
pub fn file_url(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    // Ensure a leading slash so Windows `C:/...` becomes `/C:/...`.
    let rooted = if s.starts_with('/') {
        s
    } else {
        format!("/{s}")
    };
    format!("file://{}", encode_path(&rooted))
}

/// Append a page anchor (`#page=N`) for deep-linking into PDFs.
pub fn with_page(locator: &str, page: Option<u32>) -> String {
    match page {
        Some(p) if p > 0 => format!("{locator}#page={p}"),
        _ => locator.to_string(),
    }
}

/// Percent-encode everything except characters that are safe in a path.
fn encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn encodes_spaces_and_keeps_drive() {
        let p = PathBuf::from("/C:/Users/me/My Docs/spec.pdf");
        let url = file_url(&p);
        assert!(
            url.starts_with("file:///C:/Users/me/My%20Docs/spec.pdf"),
            "{url}"
        );
    }

    #[test]
    fn page_anchor() {
        assert_eq!(with_page("file:///a.pdf", Some(4)), "file:///a.pdf#page=4");
        assert_eq!(with_page("file:///a.pdf", None), "file:///a.pdf");
    }
}
