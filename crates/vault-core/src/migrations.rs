use crate::error::Result;
use rusqlite::{params, Connection};

/// One forward-only schema migration. The runner ensures it's applied at most
/// once per database; bodies are free to assume any prior versions have run.
pub struct Migration {
    pub version: u32,
    pub sql: &'static str,
}

/// Apply all pending migrations in `migrations` (must be sorted by version).
/// Returns the list of versions newly applied this call.
///
/// Each migration runs inside its own transaction, with a row inserted into
/// `schema_migrations` so subsequent calls skip it. Idempotent across runs.
pub fn apply_pending(conn: &mut Connection, migrations: &[Migration]) -> Result<Vec<u32>> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
         );",
    )?;

    let applied: std::collections::HashSet<u32> = conn
        .prepare("SELECT version FROM schema_migrations")?
        .query_map([], |r| r.get::<_, u32>(0))?
        .collect::<std::result::Result<_, _>>()?;

    let now = chrono::Utc::now().timestamp_millis();

    let mut newly_applied = Vec::new();
    for m in migrations {
        if applied.contains(&m.version) {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)?;
        // Belt-and-suspenders: migration bodies *may* insert their own row; we
        // INSERT OR IGNORE so a redundant insert is fine.
        tx.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            params![m.version, now],
        )?;
        tx.commit()?;
        newly_applied.push(m.version);
    }

    Ok(newly_applied)
}

pub fn current_version(conn: &Connection) -> Result<u32> {
    let v: Option<u32> = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .ok();
    Ok(v.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::open_in_memory;

    const M1: &str = "CREATE TABLE t1 (id INTEGER PRIMARY KEY);";
    const M2: &str = "CREATE TABLE t2 (id INTEGER PRIMARY KEY);";

    fn migs() -> Vec<Migration> {
        vec![
            Migration {
                version: 1,
                sql: M1,
            },
            Migration {
                version: 2,
                sql: M2,
            },
        ]
    }

    #[test]
    fn applies_in_order_and_is_idempotent() {
        let mut conn = open_in_memory().unwrap();
        let m = migs();
        assert_eq!(apply_pending(&mut conn, &m).unwrap(), vec![1, 2]);
        assert_eq!(current_version(&conn).unwrap(), 2);
        // second call: nothing new
        assert!(apply_pending(&mut conn, &m).unwrap().is_empty());
    }

    #[test]
    fn upgrades_from_partial_state() {
        let mut conn = open_in_memory().unwrap();
        let m = migs();
        // Apply only version 1 first
        apply_pending(&mut conn, &m[..1]).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 1);
        // Now feed the full list — only version 2 should be applied
        assert_eq!(apply_pending(&mut conn, &m).unwrap(), vec![2]);
        assert_eq!(current_version(&conn).unwrap(), 2);
    }
}
