//! Linear Taco — `claude-ex` integration.
//!
//! Wires the [claude-ex](https://github.com/vihaanshahh/claude-ex) local
//! code intelligence layer into the Linear Taco workspace lifecycle:
//!
//! - [`sidecar`] spawns `claude-ex watch` as a long-lived child process for
//!   each open workspace, restarting it on crash and tearing it down on
//!   workspace close.
//! - [`settings_writer`] merges Linear Taco's MCP server entry and the
//!   SessionStart / PreToolUse / PostToolUse hooks into the workspace's
//!   `.claude/settings.json`, preserving whatever else is already there.
//! - [`db`] opens a read-only `rusqlite` connection to the workspace's
//!   `.claude-ex.db`, which the SearchPanel and CodeViewer overlay query
//!   directly for fast IDE responsiveness.
//! - [`queries`] is a thin, prepared-statement-cached wrapper over the
//!   tables claude-ex writes (`symbols`, `symbols_fts`, `rankings`,
//!   `edges`, `files`).

#![deny(rust_2018_idioms)]

pub mod db;
pub mod queries;
pub mod settings_writer;
pub mod sidecar;

pub use db::{ClaudeExDb, DbError};
pub use queries::{Symbol, SymbolKind, SymbolSearchResult};
pub use settings_writer::{SettingsWriteError, merge_settings};
pub use sidecar::{ClaudeExSidecar, SidecarError, SidecarStatus};
