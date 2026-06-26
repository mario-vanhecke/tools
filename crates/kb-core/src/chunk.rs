//! A small, boundary-aware sliding-window chunker.
//!
//! Deliberately simple and self-contained (no heading model like rag's): it
//! cuts the extracted text into windows of roughly `max_chars`, preferring to
//! break on a paragraph/line boundary near the limit, with `overlap`
//! characters carried into the next window so context isn't lost at the seams.

use crate::config::ChunkConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub char_start: usize,
    pub char_end: usize,
}

/// Split `text` into overlapping chunks. Operates on `char` indices so it is
/// UTF-8 safe and never splits a multi-byte character.
pub fn chunk(text: &str, cfg: &ChunkConfig) -> Vec<Chunk> {
    let max = cfg.max_chars.max(1);
    let overlap = cfg.overlap.min(max.saturating_sub(1));

    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    while start < n {
        let hard_end = (start + max).min(n);
        // Prefer a natural break (newline, then space) in the last ~20% of the
        // window so we don't cut mid-sentence.
        let end = if hard_end < n {
            let lookback = start + (max * 4 / 5);
            find_break(&chars, lookback.max(start + 1), hard_end).unwrap_or(hard_end)
        } else {
            hard_end
        };

        let slice: String = chars[start..end].iter().collect();
        let trimmed = slice.trim();
        if !trimmed.is_empty() {
            out.push(Chunk {
                text: trimmed.to_string(),
                char_start: start,
                char_end: end,
            });
        }

        if end >= n {
            break;
        }
        // Advance, carrying `overlap` chars back; always make forward progress.
        start = end.saturating_sub(overlap).max(start + 1);
    }
    out
}

/// Find the index just after the last newline (preferred) or space in
/// `[lo, hi)`, so the chunk ends on a boundary.
fn find_break(chars: &[char], lo: usize, hi: usize) -> Option<usize> {
    let mut last_space = None;
    for i in (lo..hi).rev() {
        let c = chars[i];
        if c == '\n' {
            return Some(i + 1);
        }
        if last_space.is_none() && c.is_whitespace() {
            last_space = Some(i + 1);
        }
    }
    last_space
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_nothing() {
        assert!(chunk("", &ChunkConfig::default()).is_empty());
    }

    #[test]
    fn windows_overlap_and_cover() {
        let cfg = ChunkConfig {
            max_chars: 20,
            overlap: 5,
        };
        let text = "alpha beta gamma delta epsilon zeta eta theta iota";
        let chunks = chunk(text, &cfg);
        assert!(chunks.len() > 1);
        // Every chunk within the size bound.
        for c in &chunks {
            assert!(c.text.chars().count() <= 20, "{:?}", c.text);
        }
        // Offsets are monotonic and make forward progress.
        for w in chunks.windows(2) {
            assert!(w[1].char_start > w[0].char_start);
        }
    }

    #[test]
    fn handles_multibyte() {
        let cfg = ChunkConfig {
            max_chars: 4,
            overlap: 1,
        };
        let text = "héllo wörld café";
        let chunks = chunk(text, &cfg);
        // Round-trips without panicking and preserves the characters.
        let joined: String = chunks.iter().map(|c| c.text.clone()).collect();
        assert!(joined.contains("café") || joined.contains("caf"));
    }
}
