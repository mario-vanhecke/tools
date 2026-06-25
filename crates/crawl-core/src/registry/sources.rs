use crate::error::{Error, Result};
use crate::source::{Source, SourceKind, Strategy};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

const SELECT: &str = "SELECT id, name, kind, uri, strategy, config, enabled, added_at,
                      last_crawled, last_run_id, last_status, last_error
                      FROM sources";

fn map_source(r: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    let kind_str: String = r.get(2)?;
    let strategy_str: String = r.get(4)?;
    let config_str: String = r.get(5)?;
    Ok(Source {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: SourceKind::from_str(&kind_str).unwrap_or(SourceKind::Local),
        uri: r.get(3)?,
        strategy: Strategy::from_str(&strategy_str).unwrap_or(Strategy::Recursive),
        config: serde_json::from_str(&config_str).unwrap_or(Value::Null),
        enabled: r.get::<_, i64>(6)? != 0,
        added_at: r.get(7)?,
        last_crawled: r.get(8)?,
        last_run_id: r.get(9)?,
        last_status: r.get(10)?,
        last_error: r.get(11)?,
    })
}

#[derive(Debug, Clone)]
pub struct AddSourceOptions {
    pub name: String,
    pub kind: SourceKind,
    pub uri: String,
    pub strategy: Strategy,
    pub config: Value,
    pub enabled: bool,
}

pub fn add_source(conn: &Connection, opts: &AddSourceOptions) -> Result<Source> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sources WHERE name = ?1",
            params![opts.name],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);
    if exists {
        return Err(Error::DuplicateSource(opts.name.clone()));
    }

    let now = chrono::Utc::now().timestamp_millis();
    let config_str = serde_json::to_string(&opts.config)?;
    conn.execute(
        "INSERT INTO sources (name, kind, uri, strategy, config, enabled, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            opts.name,
            opts.kind.as_str(),
            opts.uri,
            opts.strategy.as_str(),
            config_str,
            opts.enabled as i64,
            now,
        ],
    )?;
    get_source_by_name(conn, &opts.name)?
        .ok_or_else(|| Error::other("source vanished immediately after insert"))
}

pub fn list_sources(conn: &Connection) -> Result<Vec<Source>> {
    let sql = format!("{SELECT} ORDER BY name");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map([], map_source)?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn get_source_by_name(conn: &Connection, name: &str) -> Result<Option<Source>> {
    let sql = format!("{SELECT} WHERE name = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let row = stmt.query_row(params![name], map_source).optional()?;
    Ok(row)
}

pub fn get_source(conn: &Connection, id: i64) -> Result<Option<Source>> {
    let sql = format!("{SELECT} WHERE id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    let row = stmt.query_row(params![id], map_source).optional()?;
    Ok(row)
}

/// Remove a source and (via ON DELETE CASCADE) all its documents and runs.
pub fn remove_source(conn: &Connection, name: &str) -> Result<bool> {
    let n = conn.execute("DELETE FROM sources WHERE name = ?1", params![name])?;
    Ok(n > 0)
}

pub fn set_enabled(conn: &Connection, name: &str, enabled: bool) -> Result<bool> {
    let n = conn.execute(
        "UPDATE sources SET enabled = ?1 WHERE name = ?2",
        params![enabled as i64, name],
    )?;
    Ok(n > 0)
}

/// Persist post-run bookkeeping on a source: when it was crawled, the run id,
/// status, error, and any updated `config` (e.g. a refreshed SharePoint delta
/// link). Pass `config = None` to leave config untouched.
#[allow(clippy::too_many_arguments)]
pub fn update_after_run(
    conn: &Connection,
    source_id: i64,
    last_crawled: i64,
    run_id: i64,
    status: &str,
    error: Option<&str>,
    config: Option<&Value>,
) -> Result<()> {
    match config {
        Some(c) => {
            let config_str = serde_json::to_string(c)?;
            conn.execute(
                "UPDATE sources SET last_crawled = ?1, last_run_id = ?2, last_status = ?3,
                 last_error = ?4, config = ?5 WHERE id = ?6",
                params![last_crawled, run_id, status, error, config_str, source_id],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE sources SET last_crawled = ?1, last_run_id = ?2, last_status = ?3,
                 last_error = ?4 WHERE id = ?5",
                params![last_crawled, run_id, status, error, source_id],
            )?;
        }
    }
    Ok(())
}
