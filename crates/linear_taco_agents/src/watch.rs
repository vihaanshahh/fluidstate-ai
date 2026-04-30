//! Watch-mode write gate.
//!
//! When `AgentState.watch == true`, every `pty:write` for that pane must
//! be silently dropped at the IPC boundary — defense-in-depth so a
//! UI bug, an accidental focus, or a paste can't interfere with a
//! sub-agent that the user is observing.
//!
//! The gate itself is intentionally trivial; the value of this module is
//! in keeping it as the single chokepoint that everything funnels through,
//! plus a small audit-trail counter for debugging.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::state::AgentState;

#[derive(Debug, Default)]
pub struct WatchGate {
    blocked: AtomicU64,
    allowed: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteGateDecision {
    Allow,
    BlockedByWatchMode,
}

impl WatchGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide whether a write to this pane should proceed.
    pub fn decide(&self, state: &AgentState) -> WriteGateDecision {
        if state.watch {
            self.blocked.fetch_add(1, Ordering::Relaxed);
            WriteGateDecision::BlockedByWatchMode
        } else {
            self.allowed.fetch_add(1, Ordering::Relaxed);
            WriteGateDecision::Allow
        }
    }

    pub fn blocked_count(&self) -> u64 {
        self.blocked.load(Ordering::Relaxed)
    }

    pub fn allowed_count(&self) -> u64 {
        self.allowed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PaneId;
    use std::path::PathBuf;

    #[test]
    fn watching_pane_blocks_writes() {
        let mut state = AgentState::new(PaneId::new("p"), PathBuf::from("/"), "t");
        state.watch = true;
        let gate = WatchGate::new();
        assert_eq!(gate.decide(&state), WriteGateDecision::BlockedByWatchMode);
        assert_eq!(gate.blocked_count(), 1);
        assert_eq!(gate.allowed_count(), 0);
    }

    #[test]
    fn unwatching_pane_allows_writes() {
        let state = AgentState::new(PaneId::new("p"), PathBuf::from("/"), "t");
        let gate = WatchGate::new();
        assert_eq!(gate.decide(&state), WriteGateDecision::Allow);
        assert_eq!(gate.allowed_count(), 1);
        assert_eq!(gate.blocked_count(), 0);
    }

    #[test]
    fn toggling_watch_flips_subsequent_decisions() {
        let mut state = AgentState::new(PaneId::new("p"), PathBuf::from("/"), "t");
        let gate = WatchGate::new();
        assert_eq!(gate.decide(&state), WriteGateDecision::Allow);
        state.watch = true;
        assert_eq!(gate.decide(&state), WriteGateDecision::BlockedByWatchMode);
        state.watch = false;
        assert_eq!(gate.decide(&state), WriteGateDecision::Allow);
    }
}
