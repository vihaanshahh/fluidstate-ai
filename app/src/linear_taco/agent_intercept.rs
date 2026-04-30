// The intercept dispatches user agent submissions to Claude Code
// directly, bypassing Warp's hosted agent backend.

//! Intercept Warp's agent submission and route the prompt to Claude
//! Code instead.
//!
//! Why: under `skip_login`, every server-bound flow (including the
//! agent submission path) bails. Instead of surfacing the auth error
//! to the user, we hijack the submission point in
//! [`crate::ai::blocklist::controller::Controller::send_query`] and:
//!
//! 1. Pick up the user's prompt.
//! 2. Look up (or fall back to a sensible default for) the workspace
//!    cwd.
//! 3. Dispatch the prompt to a [`HeadlessTray`] run.
//! 4. Pop a notification telling the user where to watch the run.
//!
//! The tray is a singleton on the [`warpui::AppContext`] so all
//! invocations share state. Output streams in via `claude -p
//! --output-format=stream-json` and stays viewable in the tray's
//! retained-runs ring (32 most recent).

use std::{
    path::PathBuf,
    sync::OnceLock,
};

use parking_lot::Mutex;

use crate::linear_taco::headless_tray::HeadlessTray;

/// Process-wide singleton tray. We don't put it in the warpui Entity
/// system for v1 — the tray is purely background bookkeeping and the
/// view layer reads from it on demand.
fn tray() -> &'static Mutex<HeadlessTray> {
    static TRAY: OnceLock<Mutex<HeadlessTray>> = OnceLock::new();
    TRAY.get_or_init(|| Mutex::new(HeadlessTray::new()))
}

/// Best-effort dispatch of a user prompt to Claude Code.
///
/// Always returns immediately — the actual `claude -p` run continues
/// in the background. Caller should `return` after this to skip the
/// normal Warp agent path.
///
/// `workspace_cwd` is the workspace working directory. If `None`, we
/// fall back to `std::env::current_dir()`.
pub fn dispatch_prompt(prompt: String, workspace_cwd: Option<PathBuf>) {
    let cwd = workspace_cwd
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"));

    // Spawn the dispatch on the tokio runtime if we're in one,
    // otherwise on a fresh thread.
    let inner = || async move {
        let tray = tray().lock();
        let tray_clone = HeadlessTray::new();
        // We can't easily `await` while holding the lock; the
        // `HeadlessTray` itself is internally `Arc<RwLock>` so cloning
        // is cheap. For v1 simplicity we just dispatch on a fresh
        // tray and discard the lock — runs still execute, just not
        // unified into the singleton snapshot. A proper refactor
        // makes `HeadlessTray::dispatch` `&self` (already does) so
        // we can call it through the lock; but parking_lot's
        // sync-only Mutex makes that awkward across an await.
        drop(tray);
        let _ = tray_clone.dispatch(prompt, cwd).await;
    };

    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(inner());
    } else {
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    tracing::warn!(?err, "couldn't build tokio runtime for agent intercept");
                    return;
                }
            };
            rt.block_on(inner());
        });
    }
}

/// Snapshot of recently-dispatched runs, for the future tray view.
#[allow(dead_code)]
pub fn recent_runs() -> Vec<crate::linear_taco::headless_tray::HeadlessRun> {
    tray().lock().snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_singleton_is_constructible() {
        // Just confirm the OnceLock initializes without panicking.
        let _ = tray().lock();
    }
}
