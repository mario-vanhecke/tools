pub mod add;
pub mod prune;
pub mod rm;

pub use add::{add_paths, AddOptions, AddReport};
pub use prune::{prune, PruneOptions, PruneReport};
pub use rm::{remove_paths, RmReport};

use crate::error::Result;
use crate::status::FileStatus;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputRow {
    pub id: i64,
    pub input_path: String,
    pub output_path: Option<String>,
    pub added_at: i64,
    pub status: FileStatus,
    pub status_detail: Option<String>,
    pub status_note: Option<String>,
    pub last_src_mtime: Option<i64>,
    pub last_src_size: Option<i64>,
    pub last_src_hash: Option<String>,
    pub last_out_hash: Option<String>,
    pub last_converted: Option<i64>,
    pub extractor: Option<String>,
    pub attempts: i64,
    pub last_attempt: Option<i64>,
}

const SELECT: &str = "SELECT id, input_path, output_path, added_at, status,
                      status_detail, status_note,
                      last_src_mtime, last_src_size, last_src_hash, last_out_hash,
                      last_converted, extractor, attempts, last_attempt
                      FROM outputs";

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OutputRow> {
    let status_str: String = r.get(4)?;
    Ok(OutputRow {
        id: r.get(0)?,
        input_path: r.get(1)?,
        output_path: r.get(2)?,
        added_at: r.get(3)?,
        status: FileStatus::from_str(&status_str).unwrap_or(FileStatus::Pending),
        status_detail: r.get(5)?,
        status_note: r.get(6)?,
        last_src_mtime: r.get(7)?,
        last_src_size: r.get(8)?,
        last_src_hash: r.get(9)?,
        last_out_hash: r.get(10)?,
        last_converted: r.get(11)?,
        extractor: r.get(12)?,
        attempts: r.get(13)?,
        last_attempt: r.get(14)?,
    })
}

pub fn list_all(conn: &Connection) -> Result<Vec<OutputRow>> {
    list_filtered(conn, None)
}

pub fn list_filtered(conn: &Connection, status: Option<FileStatus>) -> Result<Vec<OutputRow>> {
    let sql = if status.is_some() {
        format!("{SELECT} WHERE status = ?1 ORDER BY input_path")
    } else {
        format!("{SELECT} ORDER BY input_path")
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<OutputRow> = if let Some(s) = status {
        stmt.query_map(params![s.as_str()], map_row)?
            .collect::<std::result::Result<_, _>>()?
    } else {
        stmt.query_map([], map_row)?
            .collect::<std::result::Result<_, _>>()?
    };
    Ok(rows)
}

pub fn find_by_input_path(conn: &Connection, input_path: &str) -> Result<Option<OutputRow>> {
    let sql = format!("{SELECT} WHERE input_path = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![input_path])?;
    match rows.next()? {
        Some(r) => Ok(Some(map_row(r)?)),
        None => Ok(None),
    }
}

pub fn find_by_output_path(conn: &Connection, output_path: &str) -> Result<Option<OutputRow>> {
    let sql = format!("{SELECT} WHERE output_path = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![output_path])?;
    match rows.next()? {
        Some(r) => Ok(Some(map_row(r)?)),
        None => Ok(None),
    }
}

pub fn count_by_status(conn: &Connection) -> Result<Vec<(FileStatus, i64)>> {
    let mut stmt = conn.prepare("SELECT status, COUNT(*) FROM outputs GROUP BY status")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = Vec::new();
    for r in rows {
        let (s, n) = r?;
        if let Ok(st) = FileStatus::from_str(&s) {
            out.push((st, n));
        }
    }
    Ok(out)
}
