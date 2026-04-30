// Bridge for the future CodeViewer overlay. Until the editor view
// consumes these helpers, we silence dead-code warnings.
#![allow(dead_code)]

//! Bridge between the editor and the claude-ex SQLite database.
//!
//! The editor's CodeViewer overlay (outline, dependents-count badges,
//! import-jump markers) all read from the same `.claude-ex.db` that
//! powers the SearchPanel. This module owns:
//!
//! 1. A workspace → [`ClaudeExDb`] cache so we don't reopen the db on
//!    every keystroke. Connections are cheap (no I/O after open) but
//!    reopening per query would still trigger a stat() per call.
//! 2. Typed helpers that wrap [`linear_taco_claudex::queries`] in
//!    editor-friendly types — paths instead of strings, optional
//!    return values where claude-ex hasn't indexed yet.
//!
//! All entry points are *best-effort* — when the db is missing or the
//! sidecar hasn't finished its first index, we return empty lists
//! rather than blocking the editor.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use linear_taco_claudex::{ClaudeExDb, Symbol, SymbolKind, SymbolSearchResult};
use tracing::warn;

/// Lazy per-workspace connection cache.
fn cache() -> &'static Mutex<HashMap<PathBuf, ClaudeExDb>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, ClaudeExDb>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Open or reuse the connection for `workspace_root`. Returns `None`
/// when `.claude-ex.db` doesn't exist yet (sidecar still running its
/// first index, or claude-ex isn't installed).
fn with_db<R>(workspace_root: &Path, f: impl FnOnce(&ClaudeExDb) -> R) -> Option<R> {
    let canonical = std::fs::canonicalize(workspace_root).ok()?;
    let mut map = cache().lock().ok()?;
    if let Some(existing) = map.get(&canonical) {
        return Some(f(existing));
    }
    match ClaudeExDb::open(&canonical) {
        Ok(db) => {
            let result = f(&db);
            map.insert(canonical, db);
            Some(result)
        }
        Err(err) => {
            warn!(target: "linear_taco", workspace = %canonical.display(), ?err, "claude-ex db unavailable for code viewer");
            None
        }
    }
}

/// Outline for the editor. The path must be relative to the workspace
/// root the same way claude-ex stores it (e.g. `src/lib.rs`).
pub fn outline(workspace_root: &Path, file_relative_path: &str) -> Vec<OutlineEntry> {
    let Some(symbols) = with_db(workspace_root, |db| {
        db.file_symbols(file_relative_path).unwrap_or_default()
    }) else {
        return Vec::new();
    };
    symbols.into_iter().map(OutlineEntry::from).collect()
}

/// Caller count for an exported symbol. Powers the "3 callers" badge.
/// Returns `0` when the symbol isn't indexed.
pub fn caller_count(workspace_root: &Path, symbol_id: i64) -> u64 {
    with_db(workspace_root, |db| db.caller_count(symbol_id).unwrap_or(0)).unwrap_or(0)
}

/// Top-N callers of a symbol, ordered by claude-ex's PageRank.
pub fn callers(workspace_root: &Path, symbol_id: i64, limit: usize) -> Vec<Symbol> {
    with_db(workspace_root, |db| {
        db.callers(symbol_id, limit).unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Workspace symbol search — used by the Cmd+T "Go to symbol in
/// workspace" command. Falls back to FTS5, then trigram, then empty
/// if claude-ex hasn't indexed.
pub fn search(workspace_root: &Path, query: &str, limit: usize) -> Vec<SymbolSearchResult> {
    with_db(workspace_root, |db| {
        db.search_symbols(query, limit).unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Forget the cached connection for a workspace. Called from
/// `linear_taco::claudex_session::shutdown_all` so we don't keep stale
/// handles alive after a workspace closes.
#[allow(dead_code)]
pub fn invalidate(workspace_root: &Path) {
    let canonical = match std::fs::canonicalize(workspace_root) {
        Ok(p) => p,
        Err(_) => workspace_root.to_path_buf(),
    };
    if let Ok(mut map) = cache().lock() {
        map.remove(&canonical);
    }
}

/// Editor-side outline row. Mirrors [`linear_taco_claudex::Symbol`] but
/// keeps just the fields the overlay actually renders, so the overlay
/// doesn't pin a `Path` for every symbol.
#[derive(Debug, Clone)]
pub struct OutlineEntry {
    pub id: i64,
    pub name: String,
    pub qualified_name: Option<String>,
    pub kind: SymbolKind,
    pub line_start: u64,
    pub line_end: Option<u64>,
    pub exported: bool,
    pub signature: Option<String>,
}

impl From<Symbol> for OutlineEntry {
    fn from(s: Symbol) -> Self {
        Self {
            id: s.id,
            name: s.name,
            qualified_name: s.qualified_name,
            kind: s.kind,
            line_start: s.line_start,
            line_end: s.line_end,
            exported: s.exported,
            signature: s.signature,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_returns_empty_for_uninitialized_workspace() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(outline(dir.path(), "src/lib.rs").is_empty());
        assert_eq!(caller_count(dir.path(), 1), 0);
        assert!(callers(dir.path(), 1, 10).is_empty());
        assert!(search(dir.path(), "anything", 10).is_empty());
    }
}
