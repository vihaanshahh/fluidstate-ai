//! Linear Taco — Claude transcript viewer.
//!
//! This crate is the "data" half of the Claude viewer panel: it tails the
//! transcript JSONL that Claude Code writes for each session, decodes events
//! into a typed model, and collapses tool_use ↔ tool_result pairs into
//! [`Card`]s suitable for rendering. The view layer (in the `app/` crate)
//! subscribes to the tail and turns cards into UI.
//!
//! Why JSONL? Claude Code already writes a complete record of each session
//! to `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` (the same file
//! `/resume` reads from). Tailing this file is loss-less, doesn't require a
//! second `claude --print` process, and keeps the in-pane terminal as the
//! input surface.

#![deny(rust_2018_idioms)]

pub mod events;
pub mod session_locator;
pub mod transcript_tail;

pub use events::{Card, RawEvent, SessionId, TranscriptEvent, cards_from_events};
pub use session_locator::{SessionLocator, encode_cwd};
pub use transcript_tail::{TailError, TailItem, TranscriptLine, TranscriptTail, read_all_lines};
