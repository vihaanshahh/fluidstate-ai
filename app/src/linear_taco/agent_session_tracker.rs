//! Tracks which terminal panes have already launched a `claude`
//! interactive session under the linear-taco agent hijack.
//!
//! First submission in a pane → spawn `claude "<prompt>"` so the user
//! lands in the interactive TUI. Every subsequent submission → write
//! just `"<prompt>\r"` so the prompt enters Claude's existing input
//! buffer instead of relaunching the binary. This gives a single
//! continuous conversation per pane, which is what the user expects
//! when they keep typing in the AI input.
//!
//! The tracker is process-wide and stores [`warpui::EntityId`]s of
//! [`TerminalView`] instances. We don't try to detect when the user
//! exits Claude (Ctrl+D, `:q`) — that would require subscribing to
//! PTY events. If they exit, the next submission will type the prompt
//! as a shell command, which is a clear failure mode they can recover
//! from with one keystroke (`claude\r`). Worth iterating on.

use std::{collections::HashSet, sync::OnceLock};

use parking_lot::RwLock;
use warpui::EntityId;

fn store() -> &'static RwLock<HashSet<EntityId>> {
    static STORE: OnceLock<RwLock<HashSet<EntityId>>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Returns whether this pane has already launched `claude`. If `false`,
/// also marks the pane as launched so the next call returns `true`.
pub fn is_launched_or_mark(view_id: EntityId) -> bool {
    {
        let g = store().read();
        if g.contains(&view_id) {
            return true;
        }
    }
    let mut g = store().write();
    // Re-check under the write lock — another caller may have raced us.
    if g.contains(&view_id) {
        return true;
    }
    g.insert(view_id);
    false
}

/// Forget a pane. Called by the pane teardown so a re-spawned pane id
/// (theoretically) doesn't carry stale state. Not currently wired but
/// safe to call.
#[allow(dead_code)]
pub fn forget(view_id: EntityId) {
    store().write().remove(&view_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use warpui::EntityId;

    fn fresh_id(seed: u64) -> EntityId {
        // EntityId's actual constructor is internal; in tests we just
        // need *some* unique id. The unsafe transmute is gated to the
        // test cfg only — production code never fabricates EntityIds.
        unsafe { std::mem::transmute::<u64, EntityId>(seed) }
    }

    #[test]
    fn first_call_returns_false_then_true() {
        let id = fresh_id(0xfeed_0001);
        assert!(!is_launched_or_mark(id));
        assert!(is_launched_or_mark(id));
        assert!(is_launched_or_mark(id));
        forget(id);
        assert!(!is_launched_or_mark(id));
    }

    #[test]
    fn distinct_ids_track_independently() {
        let a = fresh_id(0xfeed_0010);
        let b = fresh_id(0xfeed_0011);
        assert!(!is_launched_or_mark(a));
        assert!(!is_launched_or_mark(b));
        assert!(is_launched_or_mark(a));
        assert!(is_launched_or_mark(b));
    }
}
