//! Read-only SQLite connection to the workspace's `.claude-ex.db`.
//!
//! claude-ex is the single writer — its `watch` daemon owns mutations.
//! Linear Taco only ever reads, so we open with `SQLITE_OPEN_READ_ONLY`
//! plus WAL-friendly settings to avoid blocking the writer.
//!
//! Prepared statements are cached in [`rusqlite::CachedStatement`] under
//! the hood; callers don't need to manage that themselves.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("claude-ex database not found at {0}")]
    NotFound(PathBuf),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Opens a read-only handle to a workspace's `.claude-ex.db`.
///
/// One connection per query loop is fine for our scale (claude-ex's own
/// benchmarks show <5 ms queries even on large repos). If we ever need
/// connection pooling, swap this for `r2d2` or similar.
pub struct ClaudeExDb {
    pub(crate) conn: Connection,
    db_path: PathBuf,
}

impl std::fmt::Debug for ClaudeExDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeExDb")
            .field("db_path", &self.db_path)
            .finish_non_exhaustive()
    }
}

impl ClaudeExDb {
    /// Open the db file at `<workspace>/.claude-ex.db`. Returns
    /// [`DbError::NotFound`] if the file isn't there yet — typical when
    /// the sidecar hasn't finished `claude-ex init`.
    pub fn open(workspace_root: impl AsRef<Path>) -> Result<Self, DbError> {
        let db_path = workspace_root.as_ref().join(".claude-ex.db");
        if !db_path.exists() {
            return Err(DbError::NotFound(db_path));
        }

        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&db_path, flags)?;

        // Don't attempt journal_mode toggles in RO mode; just hint the
        // pragmas needed to play nicely with claude-ex's WAL writer.
        conn.busy_timeout(std::time::Duration::from_millis(250))?;
        conn.pragma_update(None, "query_only", "ON")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        Ok(Self { conn, db_path })
    }

    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Tables the schema is expected to expose. Returns the subset that
    /// actually exists in the connected db, useful for capability checks
    /// when the schema rev shifts.
    pub fn detected_tables(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name",
        )?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect();
        Ok(names)
    }

    /// Convenience: borrow the underlying connection for ad-hoc queries.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_db_reports_not_found() {
        let dir = TempDir::new().unwrap();
        let err = ClaudeExDb::open(dir.path()).unwrap_err();
        assert!(matches!(err, DbError::NotFound(_)));
    }

    #[test]
    fn opens_and_reads_table_list_for_minimal_schema() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join(".claude-ex.db");
        // Write a minimal schema using a writeable connection.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT NOT NULL);
                CREATE TABLE symbols (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                "#,
            )
            .unwrap();
        }
        let db = ClaudeExDb::open(dir.path()).unwrap();
        let tables = db.detected_tables().unwrap();
        assert!(tables.contains(&"files".to_string()));
        assert!(tables.contains(&"symbols".to_string()));
    }
}
