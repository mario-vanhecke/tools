pub mod prune;
pub mod rm;
pub mod sources;

pub use prune::{prune, PruneOptions, PruneReport};
pub use rm::{remove_uris, RmReport};
pub use sources::{
    add_source, get_source, get_source_by_name, list_sources, remove_source, set_enabled,
    AddSourceOptions,
};

use crate::error::Result;
use crate::status::DocStatus;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// One discovered document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRow {
    pub id: i64,
    pub source_id: i64,
    pub uri: String,
    pub name: String,
    pub rel_path: Option<String>,
    pub extension: Option<String>,
    pub size: Option<i64>,
    pub modified_ms: Option<i64>,
    pub content_hash: Option<String>,
    pub metadata: Value,
    pub status: DocStatus,
    pub status_note: Option<String>,
    pub discovered_at: i64,
    pub first_run_id: Option<i64>,
    pub last_seen: i64,
    pub last_run_id: Option<i64>,
}

const SELECT: &str = "SELECT id, source_id, uri, name, rel_path, extension, size, modified_ms,
                      content_hash, metadata, status, status_note,
                      discovered_at, first_run_id, last_seen, last_run_id
                      FROM documents";

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRow> {
    let status_str: String = r.get(10)?;
    let metadata_str: String = r.get(9)?;
    Ok(DocumentRow {
        id: r.get(0)?,
        source_id: r.get(1)?,
        uri: r.get(2)?,
        name: r.get(3)?,
        rel_path: r.get(4)?,
        extension: r.get(5)?,
        size: r.get(6)?,
        modified_ms: r.get(7)?,
        content_hash: r.get(8)?,
        metadata: serde_json::from_str(&metadata_str).unwrap_or(Value::Null),
        status: DocStatus::from_str(&status_str).unwrap_or(DocStatus::Present),
        status_note: r.get(11)?,
        discovered_at: r.get(12)?,
        first_run_id: r.get(13)?,
        last_seen: r.get(14)?,
        last_run_id: r.get(15)?,
    })
}

/// Filters for listing/searching documents. All conditions AND together.
#[derive(Debug, Clone, Default)]
pub struct DocQuery {
    pub status: Option<DocStatus>,
    pub source_id: Option<i64>,
    pub extension: Option<String>,
    /// Case-insensitive substring match against name OR uri.
    pub name_like: Option<String>,
    pub limit: Option<usize>,
}

pub fn query_documents(conn: &Connection, q: &DocQuery) -> Result<Vec<DocumentRow>> {
    let mut sql = String::from(SELECT);
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(s) = q.status {
        clauses.push(format!("status = ?{}", binds.len() + 1));
        binds.push(Box::new(s.as_str().to_string()));
    }
    if let Some(sid) = q.source_id {
        clauses.push(format!("source_id = ?{}", binds.len() + 1));
        binds.push(Box::new(sid));
    }
    if let Some(ext) = &q.extension {
        clauses.push(format!("extension = ?{}", binds.len() + 1));
        binds.push(Box::new(ext.to_lowercase()));
    }
    if let Some(like) = &q.name_like {
        let pat = format!("%{}%", like.to_lowercase());
        clauses.push(format!(
            "(LOWER(name) LIKE ?{0} OR LOWER(uri) LIKE ?{0})",
            binds.len() + 1
        ));
        binds.push(Box::new(pat));
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY name COLLATE NOCASE, uri");
    if let Some(n) = q.limit {
        sql.push_str(&format!(" LIMIT {n}"));
    }

    let mut stmt = conn.prepare(&sql)?;
    let bind_refs: Vec<&dyn rusqlite::types::ToSql> = binds.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bind_refs), map_row)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn find_by_uri(conn: &Connection, source_id: i64, uri: &str) -> Result<Option<DocumentRow>> {
    let sql = format!("{SELECT} WHERE source_id = ?1 AND uri = ?2");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![source_id, uri])?;
    match rows.next()? {
        Some(r) => Ok(Some(map_row(r)?)),
        None => Ok(None),
    }
}

pub fn count_by_status(conn: &Connection) -> Result<HashMap<DocStatus, i64>> {
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM documents GROUP BY status")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = HashMap::new();
    for r in rows {
        let (s, n) = r?;
        if let Ok(st) = DocStatus::from_str(&s) {
            out.insert(st, n);
        }
    }
    Ok(out)
}

/// Map of source_id → source name, for display joins.
pub fn source_name_map(conn: &Connection) -> Result<HashMap<i64, String>> {
    let mut stmt = conn.prepare("SELECT id, name FROM sources")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = HashMap::new();
    for r in rows {
        let (id, name) = r?;
        out.insert(id, name);
    }
    Ok(out)
}
