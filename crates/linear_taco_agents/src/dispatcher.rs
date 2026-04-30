//! Dispatcher actions — typed requests for the integration layer.
//!
//! The dispatcher itself doesn't spawn anything. It's a state machine that
//! emits [`DispatchAction`]s; the Warp `app/` layer turns them into
//! `node-pty`-style PTY spawns. Keeping the dispatcher data-only lets us
//! unit-test the dispatch logic without a UI.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::state::PaneId;

/// What kind of pane the dispatcher wants the host to create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchKind {
    /// Fresh `claude` session in `cwd`.
    Spawn,
    /// `claude --resume <sessionId>` in a new pane (forks the conversation).
    Fork { session_id: String },
    /// Sub-agent surfaced from a `Task` tool_use in the parent pane.
    /// The host opens a new pane in **watch mode** by default.
    SubAgent {
        parent: PaneId,
        sub_session_id: String,
    },
    /// `claude -p "<prompt>" --output-format=stream-json` with no PTY —
    /// shows up in the headless tray and can be promoted to a pane later.
    Headless { prompt: String },
    /// Plain `$SHELL` pane.
    Shell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchAction {
    pub pane_id: PaneId,
    pub cwd: PathBuf,
    pub kind: DispatchKind,
    /// Should the new pane open in watch mode? Default for SubAgent.
    pub watch: bool,
    /// Optional title. If absent, the host derives one from the first
    /// user prompt or the action kind.
    pub title: Option<String>,
}

impl DispatchAction {
    pub fn spawn(cwd: PathBuf) -> Self {
        Self {
            pane_id: PaneId::fresh(),
            cwd,
            kind: DispatchKind::Spawn,
            watch: false,
            title: None,
        }
    }

    pub fn fork(cwd: PathBuf, session_id: impl Into<String>) -> Self {
        Self {
            pane_id: PaneId::fresh(),
            cwd,
            kind: DispatchKind::Fork {
                session_id: session_id.into(),
            },
            watch: false,
            title: None,
        }
    }

    pub fn sub_agent(parent: PaneId, cwd: PathBuf, sub_session_id: impl Into<String>) -> Self {
        Self {
            pane_id: PaneId::fresh(),
            cwd,
            kind: DispatchKind::SubAgent {
                parent,
                sub_session_id: sub_session_id.into(),
            },
            watch: true,
            title: None,
        }
    }

    pub fn headless(cwd: PathBuf, prompt: impl Into<String>) -> Self {
        Self {
            pane_id: PaneId::fresh(),
            cwd,
            kind: DispatchKind::Headless {
                prompt: prompt.into(),
            },
            watch: true,
            title: None,
        }
    }

    pub fn shell(cwd: PathBuf) -> Self {
        Self {
            pane_id: PaneId::fresh(),
            cwd,
            kind: DispatchKind::Shell,
            watch: false,
            title: None,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_watch(mut self, watch: bool) -> Self {
        self.watch = watch;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_agent_defaults_to_watch_mode() {
        let action =
            DispatchAction::sub_agent(PaneId::new("parent"), PathBuf::from("/work"), "sub-session");
        assert!(action.watch);
        match action.kind {
            DispatchKind::SubAgent {
                parent,
                sub_session_id,
            } => {
                assert_eq!(parent, PaneId::new("parent"));
                assert_eq!(sub_session_id, "sub-session");
            }
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn fork_round_trips_session_id() {
        let action = DispatchAction::fork(PathBuf::from("/work"), "abc-123");
        match action.kind {
            DispatchKind::Fork { session_id } => assert_eq!(session_id, "abc-123"),
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn pane_ids_are_unique_per_dispatch() {
        let a = DispatchAction::spawn(PathBuf::from("/work"));
        let b = DispatchAction::spawn(PathBuf::from("/work"));
        assert_ne!(a.pane_id, b.pane_id);
    }
}
