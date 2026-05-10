use crate::error::Result;
use rusqlite::Connection;
use vault_core::Migration;

pub const SCHEMA_VERSION: u32 = 1;

const MIGRATION_001: &str = include_str!("migrations/001_initial.sql");

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: MIGRATION_001,
}];

pub fn apply_pending(conn: &mut Connection) -> Result<Vec<u32>> {
    Ok(vault_core::apply_pending(conn, MIGRATIONS)?)
}

pub fn current_version(conn: &Connection) -> Result<u32> {
    Ok(vault_core::current_version(conn)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use vault_core::open_in_memory;

    #[test]
    fn schema_has_expected_tables() {
        let mut conn = open_in_memory().unwrap();
        apply_pending(&mut conn).unwrap();
        for tbl in &["vault_meta", "settings", "outputs"] {
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE name = ?1",
                    params![tbl],
                    |_| Ok(true),
                )
                .unwrap_or(false);
            assert!(exists, "missing table {tbl}");
        }
    }

    #[test]
    fn applies_idempotently() {
        let mut conn = open_in_memory().unwrap();
        let a = apply_pending(&mut conn).unwrap();
        assert_eq!(a, vec![1]);
        let b = apply_pending(&mut conn).unwrap();
        assert!(b.is_empty());
        assert_eq!(current_version(&conn).unwrap(), 1);
    }
}
