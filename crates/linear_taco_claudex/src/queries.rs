//! Typed queries against the claude-ex SQLite schema.
//!
//! claude-ex's schema (see <https://github.com/vihaanshahh/claude-ex>) writes
//! `files`, `symbols`, `symbols_fts` (FTS5), `symbols_trigram` (FTS5),
//! `edges`, and `rankings` tables. We expose just the queries the IDE
//! actually needs:
//!
//! - [`ClaudeExDb::search_symbols`] — primary fuzzy/FTS search for the
//!   SearchPanel; falls back to trigram for substring matches when FTS
//!   misses (camelCase, partial identifiers).
//! - [`ClaudeExDb::file_symbols`] — outline for the editor's CodeViewer
//!   overlay.
//! - [`ClaudeExDb::callers`] — "who calls X" lookup for hover context.
//!
//! All queries are stateless and prepared on each call; rusqlite's
//! statement cache amortizes parsing. With <5 ms typical latencies this
//! is fast enough for keystroke-driven UIs.

use std::path::PathBuf;

use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;

use crate::db::{ClaudeExDb, DbError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Type,
    Variable,
    Constant,
    Module,
    Other,
}

impl SymbolKind {
    fn parse(s: &str) -> Self {
        match s {
            "function" | "fn" => SymbolKind::Function,
            "method" => SymbolKind::Method,
            "class" => SymbolKind::Class,
            "interface" => SymbolKind::Interface,
            "type" | "type_alias" | "alias" => SymbolKind::Type,
            "variable" | "var" | "let" => SymbolKind::Variable,
            "const" | "constant" => SymbolKind::Constant,
            "module" => SymbolKind::Module,
            _ => SymbolKind::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Symbol {
    pub id: i64,
    pub name: String,
    pub qualified_name: Option<String>,
    pub kind: SymbolKind,
    pub file_path: PathBuf,
    pub line_start: u64,
    pub line_end: Option<u64>,
    pub signature: Option<String>,
    pub exported: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolSearchResult {
    pub symbol: Symbol,
    /// PageRank score from claude-ex's `rankings` table, when present.
    pub rank_score: Option<f64>,
}

impl ClaudeExDb {
    /// Search symbols by an FTS5 query. Falls back to trigram substring
    /// search if FTS returns nothing — handy for camelCase fragments.
    /// Results are sorted by claude-ex's PageRank (highest first).
    pub fn search_symbols(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, DbError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut results = self.search_symbols_fts(query, limit)?;
        if results.is_empty() {
            results = self.search_symbols_trigram(query, limit)?;
        }
        Ok(results)
    }

    fn search_symbols_fts(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, DbError> {
        // FTS5 NEAR + match ranking. We attempt the join optimistically;
        // if symbols_fts isn't present yet the query errors and we let the
        // caller fall back.
        let sql = "
            SELECT s.id, s.name, s.qualified_name, s.kind,
                   f.path, s.line_start, s.line_end, s.signature, s.exported,
                   r.score
            FROM symbols_fts fts
            JOIN symbols s ON s.id = fts.rowid
            JOIN files f   ON f.id = s.file_id
            LEFT JOIN rankings r ON r.symbol_id = s.id
            WHERE symbols_fts MATCH ?1
            ORDER BY r.score DESC NULLS LAST, s.name ASC
            LIMIT ?2
        ";
        let stmt = self.conn.prepare_cached(sql);
        let mut stmt = match stmt {
            Ok(s) => s,
            // Schema mismatch (e.g. older claude-ex without rankings) →
            // pretend we found nothing so the caller can fall back.
            Err(_) => return Ok(Vec::new()),
        };
        let rows = stmt
            .query_map(params![query, limit as i64], symbol_search_row)
            .ok();
        let Some(rows) = rows else {
            return Ok(Vec::new());
        };
        let out: Vec<SymbolSearchResult> = rows.filter_map(Result::ok).collect();
        Ok(out)
    }

    fn search_symbols_trigram(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SymbolSearchResult>, DbError> {
        let sql = "
            SELECT s.id, s.name, s.qualified_name, s.kind,
                   f.path, s.line_start, s.line_end, s.signature, s.exported,
                   r.score
            FROM symbols_trigram tri
            JOIN symbols s ON s.id = tri.rowid
            JOIN files f   ON f.id = s.file_id
            LEFT JOIN rankings r ON r.symbol_id = s.id
            WHERE symbols_trigram MATCH ?1
            ORDER BY r.score DESC NULLS LAST, s.name ASC
            LIMIT ?2
        ";
        let stmt = self.conn.prepare_cached(sql);
        let mut stmt = match stmt {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()),
        };
        let rows = stmt
            .query_map(params![query, limit as i64], symbol_search_row)
            .ok();
        let Some(rows) = rows else {
            return Ok(Vec::new());
        };
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Symbols defined in a single file, ordered by `line_start` —
    /// the data backing the CodeViewer outline.
    pub fn file_symbols(&self, path: &str) -> Result<Vec<Symbol>, DbError> {
        let sql = "
            SELECT s.id, s.name, s.qualified_name, s.kind,
                   f.path, s.line_start, s.line_end, s.signature, s.exported
            FROM symbols s
            JOIN files f ON f.id = s.file_id
            WHERE f.path = ?1
            ORDER BY s.line_start ASC
        ";
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt
            .query_map(params![path], symbol_row)?
            .filter_map(Result::ok)
            .collect();
        Ok(rows)
    }

    /// Caller-side of the call graph: symbols whose edges point at `symbol_id`
    /// with kind 'calls'. Returns up to `limit` callers ordered by their
    /// own PageRank desc.
    pub fn callers(&self, symbol_id: i64, limit: usize) -> Result<Vec<Symbol>, DbError> {
        let sql = "
            SELECT s.id, s.name, s.qualified_name, s.kind,
                   f.path, s.line_start, s.line_end, s.signature, s.exported
            FROM edges e
            JOIN symbols s ON s.id = e.from_symbol
            JOIN files f   ON f.id = s.file_id
            LEFT JOIN rankings r ON r.symbol_id = s.id
            WHERE e.to_symbol = ?1 AND e.kind = 'calls'
            ORDER BY r.score DESC NULLS LAST, s.name ASC
            LIMIT ?2
        ";
        let stmt = self.conn.prepare_cached(sql);
        let mut stmt = match stmt {
            Ok(s) => s,
            Err(_) => return Ok(Vec::new()),
        };
        let rows = stmt
            .query_map(params![symbol_id, limit as i64], symbol_row)
            .ok();
        let Some(rows) = rows else {
            return Ok(Vec::new());
        };
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// "How many callers does this symbol have?" — feeds the
    /// dependents-count badge in CodeViewer.
    pub fn caller_count(&self, symbol_id: i64) -> Result<u64, DbError> {
        let sql = "SELECT COUNT(*) FROM edges WHERE to_symbol = ?1 AND kind = 'calls'";
        let stmt = self.conn.prepare_cached(sql);
        let mut stmt = match stmt {
            Ok(s) => s,
            Err(_) => return Ok(0),
        };
        let n: Option<i64> = stmt
            .query_row(params![symbol_id], |row| row.get(0))
            .optional()?;
        Ok(n.unwrap_or(0) as u64)
    }
}

fn symbol_row(row: &Row<'_>) -> rusqlite::Result<Symbol> {
    Ok(Symbol {
        id: row.get::<_, i64>(0)?,
        name: row.get::<_, String>(1)?,
        qualified_name: row.get::<_, Option<String>>(2)?,
        kind: SymbolKind::parse(row.get::<_, String>(3)?.as_str()),
        file_path: PathBuf::from(row.get::<_, String>(4)?),
        line_start: row.get::<_, i64>(5)? as u64,
        line_end: row.get::<_, Option<i64>>(6)?.map(|n| n as u64),
        signature: row.get::<_, Option<String>>(7)?,
        exported: row.get::<_, i64>(8)? != 0,
    })
}

fn symbol_search_row(row: &Row<'_>) -> rusqlite::Result<SymbolSearchResult> {
    let symbol = symbol_row(row)?;
    let rank_score = row.get::<_, Option<f64>>(9)?;
    Ok(SymbolSearchResult { symbol, rank_score })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn populate_minimal(dir: &std::path::Path) -> std::path::PathBuf {
        let db_path = dir.join(".claude-ex.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT NOT NULL);
            CREATE TABLE symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                qualified_name TEXT,
                kind TEXT NOT NULL,
                line_start INTEGER NOT NULL,
                line_end INTEGER,
                signature TEXT,
                exported INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(file_id) REFERENCES files(id)
            );
            CREATE TABLE rankings (symbol_id INTEGER PRIMARY KEY, score REAL NOT NULL);
            CREATE TABLE edges (
                id INTEGER PRIMARY KEY,
                from_symbol INTEGER NOT NULL,
                to_symbol INTEGER NOT NULL,
                kind TEXT NOT NULL
            );
            INSERT INTO files (id, path) VALUES (1, 'src/lib.rs'), (2, 'src/main.rs');
            INSERT INTO symbols (id, file_id, name, qualified_name, kind, line_start, exported)
                VALUES (1, 1, 'do_thing', 'crate::do_thing', 'function', 10, 1),
                       (2, 2, 'main', 'main', 'function', 1, 1),
                       (3, 1, 'helper', 'crate::helper', 'function', 50, 0);
            INSERT INTO rankings (symbol_id, score) VALUES (1, 0.9), (2, 0.5), (3, 0.1);
            INSERT INTO edges (from_symbol, to_symbol, kind)
                VALUES (2, 1, 'calls'), (3, 1, 'calls');
            "#,
        )
        .unwrap();
        db_path
    }

    #[test]
    fn file_symbols_returns_in_line_order() {
        let dir = TempDir::new().unwrap();
        let _ = populate_minimal(dir.path());
        let db = ClaudeExDb::open(dir.path()).unwrap();
        let syms = db.file_symbols("src/lib.rs").unwrap();
        assert_eq!(syms.len(), 2);
        assert_eq!(syms[0].name, "do_thing");
        assert_eq!(syms[1].name, "helper");
        assert!(syms[0].exported);
        assert!(!syms[1].exported);
    }

    #[test]
    fn callers_resolves_call_graph() {
        let dir = TempDir::new().unwrap();
        let _ = populate_minimal(dir.path());
        let db = ClaudeExDb::open(dir.path()).unwrap();
        let callers = db.callers(1, 10).unwrap();
        let names: Vec<_> = callers.iter().map(|s| s.name.clone()).collect();
        assert!(names.contains(&"main".to_string()));
        assert!(names.contains(&"helper".to_string()));
        assert_eq!(db.caller_count(1).unwrap(), 2);
        assert_eq!(db.caller_count(2).unwrap(), 0);
    }

    #[test]
    fn search_returns_empty_when_fts_table_absent() {
        // Minimal schema lacks symbols_fts/trigram → search returns []
        // rather than erroring.
        let dir = TempDir::new().unwrap();
        let _ = populate_minimal(dir.path());
        let db = ClaudeExDb::open(dir.path()).unwrap();
        let res = db.search_symbols("do_thing", 10).unwrap();
        assert!(res.is_empty());
    }
}
