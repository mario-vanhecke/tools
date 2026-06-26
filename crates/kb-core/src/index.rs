//! The single, portable index artifact: SQLite + sqlite-vec.
//!
//! Schema (reference-only — no source bytes are stored, only a `locator` back
//! to the origin plus the derived, searchable chunk text):
//!
//! ```text
//! meta(key, value)                         -- embedding model, dims, version
//! documents(id, source, kind, locator, …)  -- one row per source document
//! chunks(id, doc_id, ordinal, text, page…) -- derived searchable text
//! vec_chunks USING vec0(chunk_id, embedding FLOAT[dims])
//! ```

use crate::chunk::Chunk;
use crate::config::EmbeddingConfig;
use crate::error::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Once;

const KB_VERSION: &str = "1";

static SQLITE_VEC_INIT: Once = Once::new();

/// Register sqlite-vec as a SQLite auto-extension for every connection opened
/// later in this process. (Standard sqlite-vec wiring; the only correct way to
/// make the `vec0` virtual table available.)
fn ensure_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| {
        type AutoExtFn = unsafe extern "C" fn(
            *mut rusqlite::ffi::sqlite3,
            *mut *const std::os::raw::c_char,
            *const rusqlite::ffi::sqlite3_api_routines,
        ) -> std::os::raw::c_int;
        let init: unsafe extern "C" fn() = sqlite_vec::sqlite3_vec_init;
        let init: AutoExtFn =
            unsafe { std::mem::transmute::<unsafe extern "C" fn(), AutoExtFn>(init) };
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(init));
        }
    });
}

/// Encode an f32 vector as little-endian bytes — the on-disk form sqlite-vec
/// expects for a `FLOAT[]` column.
pub fn floats_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// What `distill` knows about a document before chunking — all reference data,
/// no bytes.
#[derive(Debug, Clone)]
pub struct DocumentInput {
    pub source: String,
    pub kind: String,
    pub locator: String,
    pub title: String,
    pub content_hash: String,
    pub modified_at: Option<String>,
    pub size: Option<u64>,
}

/// Prior state of a document, for incremental rebuilds.
#[derive(Debug, Clone)]
pub struct DocState {
    pub id: i64,
    pub content_hash: String,
    pub modified_at: Option<String>,
    pub size: Option<i64>,
}

/// A search result: the chunk text plus the origin pointer to cite.
#[derive(Debug, Clone)]
pub struct Hit {
    pub distance: f32,
    pub text: String,
    pub locator: String,
    pub title: String,
    pub source: String,
    pub kind: String,
    pub page: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct Stats {
    pub documents: i64,
    pub chunks: i64,
}

/// The reassembled indexed text of one document (for `kb_get`).
#[derive(Debug, Clone)]
pub struct DocumentText {
    pub locator: String,
    pub title: String,
    pub source: String,
    pub kind: String,
    pub text: String,
}

pub struct Index {
    conn: Connection,
    dims: usize,
    model: String,
}

impl Index {
    /// Open an existing index or create a fresh one at `path`. On create, the
    /// `vec0` table is sized to `emb.dims` and the full embedding config
    /// (endpoint, model, dims, api_key_env) is recorded in `meta` so a server
    /// can serve the artifact with no other config. On open, a dims/model
    /// mismatch is rejected (you can't mix embedding spaces); the endpoint is
    /// refreshed to the latest config.
    pub fn open_or_create(path: impl AsRef<Path>, emb: &EmbeddingConfig) -> Result<Self> {
        ensure_sqlite_vec();
        let conn = Connection::open(path.as_ref())?;
        configure(&conn)?;
        let initialized: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta'",
                [],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);

        if !initialized {
            create_schema(&conn, emb)?;
            Ok(Self {
                conn,
                dims: emb.dims,
                model: emb.model.clone(),
            })
        } else {
            let idx = Self::from_initialized(conn)?;
            if idx.dims != emb.dims {
                return Err(Error::other(format!(
                    "index was built with dims {} but config says {} — rebuild or fix dims",
                    idx.dims, emb.dims
                )));
            }
            if idx.model != emb.model {
                return Err(Error::other(format!(
                    "index was built with model `{}` but config says `{}` — rebuild or restore the model",
                    idx.model, emb.model
                )));
            }
            // Keep the served endpoint fresh.
            meta_set(&idx.conn, "embedding_endpoint", &emb.endpoint)?;
            meta_set(
                &idx.conn,
                "embedding_api_key_env",
                emb.api_key_env.as_deref().unwrap_or(""),
            )?;
            Ok(idx)
        }
    }

    /// Open an existing index read-only-ish (for `recall`), reading the model
    /// and dims from its `meta`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        ensure_sqlite_vec();
        let p = path.as_ref();
        if !p.exists() {
            return Err(Error::other(format!(
                "index not found: {} — run `distill build` first",
                p.display()
            )));
        }
        let conn = Connection::open(p)?;
        configure(&conn)?;
        Self::from_initialized(conn)
    }

    fn from_initialized(conn: Connection) -> Result<Self> {
        let model: String = meta_get(&conn, "embedding_model")?
            .ok_or_else(|| Error::other("not a knowledge index (missing meta)"))?;
        let dims: usize = meta_get(&conn, "embedding_dims")?
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| Error::other("index meta is missing embedding_dims"))?;
        Ok(Self { conn, dims, model })
    }

    pub fn dims(&self) -> usize {
        self.dims
    }
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Reconstruct the embedding config recorded at build time, so a server can
    /// embed queries the same way the documents were embedded — using only the
    /// index file.
    pub fn embedding_config(&self) -> Result<EmbeddingConfig> {
        let endpoint = meta_get(&self.conn, "embedding_endpoint")?
            .ok_or_else(|| Error::other("index meta is missing embedding_endpoint"))?;
        let api_key_env = meta_get(&self.conn, "embedding_api_key_env")?.filter(|s| !s.is_empty());
        Ok(EmbeddingConfig {
            endpoint,
            model: self.model.clone(),
            dims: self.dims,
            api_key_env,
        })
    }

    pub fn document_state(&self, locator: &str) -> Result<Option<DocState>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, content_hash, modified_at, size FROM documents WHERE locator = ?1",
                params![locator],
                |r| {
                    Ok(DocState {
                        id: r.get(0)?,
                        content_hash: r.get(1)?,
                        modified_at: r.get(2)?,
                        size: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Insert or replace a document and all its chunks+vectors, transactionally.
    /// Replaces any prior chunks for the same `locator` (incremental rebuild).
    pub fn add_document(
        &mut self,
        doc: &DocumentInput,
        chunks: &[(Chunk, Vec<f32>)],
    ) -> Result<i64> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO documents (source, kind, locator, title, content_hash, modified_at, size, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
             ON CONFLICT(locator) DO UPDATE SET
               source=excluded.source, kind=excluded.kind, title=excluded.title,
               content_hash=excluded.content_hash, modified_at=excluded.modified_at,
               size=excluded.size, indexed_at=datetime('now')",
            params![
                doc.source,
                doc.kind,
                doc.locator,
                doc.title,
                doc.content_hash,
                doc.modified_at,
                doc.size.map(|s| s as i64),
            ],
        )?;
        let doc_id: i64 = tx.query_row(
            "SELECT id FROM documents WHERE locator = ?1",
            params![doc.locator],
            |r| r.get(0),
        )?;

        // Drop any prior chunks/vectors for this document.
        tx.execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN (SELECT id FROM chunks WHERE doc_id = ?1)",
            params![doc_id],
        )?;
        tx.execute("DELETE FROM chunks WHERE doc_id = ?1", params![doc_id])?;

        for (ordinal, (chunk, embedding)) in chunks.iter().enumerate() {
            if embedding.len() != self.dims {
                return Err(Error::other(format!(
                    "embedding dim {} != index dims {}",
                    embedding.len(),
                    self.dims
                )));
            }
            tx.execute(
                "INSERT INTO chunks (doc_id, ordinal, text, page, char_start, char_end)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    doc_id,
                    ordinal as i64,
                    chunk.text,
                    Option::<i64>::None,
                    chunk.char_start as i64,
                    chunk.char_end as i64,
                ],
            )?;
            let chunk_id = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO vec_chunks (chunk_id, embedding) VALUES (?1, ?2)",
                params![chunk_id, floats_to_bytes(embedding)],
            )?;
        }
        tx.commit()?;
        Ok(doc_id)
    }

    /// Delete documents (and their chunks/vectors) for `source` whose locator
    /// is not in `seen`. Used to retire documents that disappeared at origin.
    pub fn prune_source(&mut self, source: &str, seen: &HashSet<String>) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let stale: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, locator FROM documents WHERE source = ?1")?;
            let rows = stmt.query_map(params![source], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let mut v = Vec::new();
            for row in rows {
                let (id, locator): (i64, String) = row?;
                if !seen.contains(&locator) {
                    v.push((id, locator));
                }
            }
            v
        };
        for (id, _) in &stale {
            tx.execute(
                "DELETE FROM vec_chunks WHERE chunk_id IN (SELECT id FROM chunks WHERE doc_id = ?1)",
                params![id],
            )?;
            tx.execute("DELETE FROM chunks WHERE doc_id = ?1", params![id])?;
            tx.execute("DELETE FROM documents WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(stale.len())
    }

    /// k-nearest chunks to `query`, with their origin pointers for citation.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<Hit>> {
        if query.len() != self.dims {
            return Err(Error::other(format!(
                "query dim {} != index dims {}",
                query.len(),
                self.dims
            )));
        }
        let bytes = floats_to_bytes(query);
        let mut stmt = self.conn.prepare(
            "SELECT v.distance, c.text, c.page, d.locator, d.title, d.source, d.kind
             FROM vec_chunks v
             JOIN chunks c ON c.id = v.chunk_id
             JOIN documents d ON d.id = c.doc_id
             WHERE v.embedding MATCH ?1 AND k = ?2
             ORDER BY v.distance",
        )?;
        let rows = stmt.query_map(params![bytes, k as i64], |r| {
            Ok(Hit {
                distance: r.get(0)?,
                text: r.get(1)?,
                page: r.get::<_, Option<i64>>(2)?.map(|p| p as u32),
                locator: r.get(3)?,
                title: r.get(4)?,
                source: r.get(5)?,
                kind: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Reassemble the indexed text of a document (ordered chunks) by locator.
    /// Returns what the index holds — not the original file, which stays at its
    /// origin.
    pub fn document_text(&self, locator: &str) -> Result<Option<DocumentText>> {
        let doc = self
            .conn
            .query_row(
                "SELECT id, title, source, kind FROM documents WHERE locator = ?1",
                params![locator],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, title, source, kind)) = doc else {
            return Ok(None);
        };
        let mut stmt = self
            .conn
            .prepare("SELECT text FROM chunks WHERE doc_id = ?1 ORDER BY ordinal")?;
        let rows = stmt.query_map(params![id], |r| r.get::<_, String>(0))?;
        let mut parts = Vec::new();
        for r in rows {
            parts.push(r?);
        }
        Ok(Some(DocumentText {
            locator: locator.to_string(),
            title,
            source,
            kind,
            text: parts.join("\n\n"),
        }))
    }

    pub fn stats(&self) -> Result<Stats> {
        let documents: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))?;
        let chunks: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))?;
        Ok(Stats { documents, chunks })
    }
}

fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;",
    )?;
    Ok(())
}

fn create_schema(conn: &Connection, emb: &EmbeddingConfig) -> Result<()> {
    let (model, dims) = (emb.model.as_str(), emb.dims);
    conn.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE documents (
             id           INTEGER PRIMARY KEY,
             source       TEXT NOT NULL,
             kind         TEXT NOT NULL,
             locator      TEXT NOT NULL UNIQUE,
             title        TEXT NOT NULL,
             content_hash TEXT NOT NULL,
             modified_at  TEXT,
             size         INTEGER,
             indexed_at   TEXT NOT NULL
         );
         CREATE TABLE chunks (
             id         INTEGER PRIMARY KEY,
             doc_id     INTEGER NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
             ordinal    INTEGER NOT NULL,
             text       TEXT NOT NULL,
             page       INTEGER,
             char_start INTEGER NOT NULL,
             char_end   INTEGER NOT NULL
         );
         CREATE INDEX idx_chunks_doc ON chunks(doc_id);",
    )?;
    conn.execute(
        &format!(
            "CREATE VIRTUAL TABLE vec_chunks USING vec0(chunk_id INTEGER PRIMARY KEY, embedding FLOAT[{dims}])"
        ),
        [],
    )?;
    let mut set = conn.prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")?;
    set.execute(params!["kb_version", KB_VERSION])?;
    set.execute(params!["embedding_model", model])?;
    set.execute(params!["embedding_dims", dims.to_string()])?;
    set.execute(params!["embedding_endpoint", emb.endpoint])?;
    set.execute(params![
        "embedding_api_key_env",
        emb.api_key_env.as_deref().unwrap_or("")
    ])?;
    set.execute(params!["created_at", chrono::Utc::now().to_rfc3339()])?;
    Ok(())
}

fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Chunk;

    fn emb(model: &str, dims: usize) -> EmbeddingConfig {
        EmbeddingConfig {
            endpoint: "http://localhost:11434/v1".into(),
            model: model.into(),
            dims,
            api_key_env: None,
        }
    }

    fn mk_chunk(text: &str) -> Chunk {
        Chunk {
            text: text.to_string(),
            char_start: 0,
            char_end: text.len(),
        }
    }

    #[test]
    fn build_and_search_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.kb");
        let mut idx = Index::open_or_create(&path, &emb("test-model", 3)).unwrap();

        let doc = DocumentInput {
            source: "docs".into(),
            kind: "local".into(),
            locator: "file:///a.txt".into(),
            title: "a".into(),
            content_hash: "h1".into(),
            modified_at: None,
            size: None,
        };
        idx.add_document(
            &doc,
            &[
                (mk_chunk("apple banana"), vec![1.0, 0.0, 0.0]),
                (mk_chunk("carrot"), vec![0.0, 1.0, 0.0]),
            ],
        )
        .unwrap();

        let hits = idx.search(&[1.0, 0.0, 0.0], 2).unwrap();
        assert_eq!(hits[0].text, "apple banana");
        assert_eq!(hits[0].locator, "file:///a.txt");
        assert_eq!(idx.stats().unwrap().chunks, 2);
    }

    #[test]
    fn reindex_replaces_chunks_not_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.kb");
        let mut idx = Index::open_or_create(&path, &emb("m", 2)).unwrap();
        let mut doc = DocumentInput {
            source: "s".into(),
            kind: "local".into(),
            locator: "file:///x.txt".into(),
            title: "x".into(),
            content_hash: "h1".into(),
            modified_at: None,
            size: None,
        };
        idx.add_document(&doc, &[(mk_chunk("v1"), vec![1.0, 0.0])])
            .unwrap();
        doc.content_hash = "h2".into();
        idx.add_document(
            &doc,
            &[
                (mk_chunk("v2a"), vec![1.0, 0.0]),
                (mk_chunk("v2b"), vec![0.0, 1.0]),
            ],
        )
        .unwrap();

        let s = idx.stats().unwrap();
        assert_eq!(s.documents, 1, "still one document");
        assert_eq!(s.chunks, 2, "old chunk replaced, not accumulated");
        assert_eq!(
            idx.document_state("file:///x.txt")
                .unwrap()
                .unwrap()
                .content_hash,
            "h2"
        );
    }

    #[test]
    fn dims_mismatch_is_rejected_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("k.kb");
        Index::open_or_create(&path, &emb("m", 4)).unwrap();
        let err = Index::open_or_create(&path, &emb("m", 8)).err().unwrap();
        assert!(err.to_string().contains("dims"), "{err}");
    }
}
