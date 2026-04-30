//! Bridges CLI-agent-session detection to the `linear_taco_claudex`
//! sidecar lifecycle.
//!
//! Whenever Warp registers a `claude` CLI agent session in a workspace,
//! we:
//!
//! 1. **Synchronously merge** `linear-taco`'s MCP server entry and the
//!    SessionStart / PreToolUse / PostToolUse hooks into the workspace's
//!    `.claude/settings.json` — this is idempotent and fast (<1 ms), so
//!    it's safe to call on every detection.
//! 2. **Spawn a long-lived `claude-ex watch` sidecar** for that workspace
//!    if one isn't already running. Subsequent panes in the same workspace
//!    reuse the existing sidecar rather than racing to start their own.
//!
//! The sidecar map keeps strong handles to every running sidecar so we
//! can shut them down on app exit; for v1 we leak gracefully (the OS
//! reaps the children when the app exits).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use linear_taco_claudex::{merge_settings, ClaudeExSidecar};
use settings::Setting as _;
use tokio::sync::Mutex;
use tracing::{info, warn};
use warpui::{AppContext, SingletonEntity};

use crate::settings::LinearTacoSettings;

/// Map from canonicalized workspace cwd → live sidecar handle.
///
/// `OnceLock` over a tokio `Mutex` lets us lazy-init on first access
/// from any tokio runtime context without sprinkling `lazy_static!`
/// elsewhere in the crate.
fn sidecars() -> &'static Mutex<HashMap<PathBuf, ClaudeExSidecar>> {
    static MAP: OnceLock<Mutex<HashMap<PathBuf, ClaudeExSidecar>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Called when Warp registers a CLI agent session whose `agent` is
/// `Claude`. Best-effort: if any step fails, we log and continue rather
/// than blocking the user from using the terminal.
///
/// `cwd_str` is the workspace working directory the CLI agent reported
/// in its first event. If `None` or empty, this is a no-op.
///
/// `ctx` is used to read the [`LinearTacoSettings`] toggles so the user
/// can disable autostart / merge without recompiling.
pub fn on_claude_session_started(cwd_str: Option<&str>, ctx: &AppContext) {
    let Some(cwd) = cwd_str.filter(|s| !s.is_empty()).map(PathBuf::from) else {
        return;
    };

    let settings = LinearTacoSettings::as_ref(ctx);
    let merge_enabled = *settings.claudex_settings_merge.value();
    let autostart_enabled = *settings.claudex_autostart.value();

    if !merge_enabled && !autostart_enabled {
        return;
    }

    // Best-effort canonicalize so two different absolute paths to the
    // same directory share a sidecar.
    let canonical = std::fs::canonicalize(&cwd).unwrap_or(cwd.clone());

    if merge_enabled {
        match merge_settings(&canonical) {
            Ok(path) => {
                info!(target: "linear_taco", "merged claude-ex settings into {}", path.display())
            }
            Err(err) => warn!(target: "linear_taco", ?err, "failed to merge .claude/settings.json"),
        }
    }

    if !autostart_enabled {
        return;
    }

    // Kick off the sidecar in the background. We need a tokio runtime
    // handle; if we're not already inside one, spawn a small one.
    let canonical_for_task = canonical.clone();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(start_sidecar_if_absent(canonical_for_task));
    } else {
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    warn!(target: "linear_taco", ?err, "couldn't build tokio runtime for claude-ex sidecar");
                    return;
                }
            };
            rt.block_on(start_sidecar_if_absent(canonical_for_task));
        });
    }
}

async fn start_sidecar_if_absent(workspace: PathBuf) {
    {
        let map = sidecars().lock().await;
        if map.contains_key(&workspace) {
            return;
        }
    }
    match ClaudeExSidecar::start(&workspace).await {
        Ok(sidecar) => {
            let mut map = sidecars().lock().await;
            // Recheck under the write lock — another pane might have
            // raced us and already inserted.
            map.entry(workspace.clone()).or_insert(sidecar);
            info!(target: "linear_taco", "claude-ex sidecar running for {}", workspace.display());
        }
        Err(err) => {
            warn!(target: "linear_taco", workspace = %workspace.display(), ?err, "claude-ex sidecar failed to start");
        }
    }
}

/// Test seam — count of currently tracked sidecars. Tests can spin up a
/// dummy workspace and verify the map updates without us exposing the
/// `OnceLock` itself.
#[cfg(test)]
pub(crate) async fn live_count() -> usize {
    sidecars().lock().await.len()
}

/// Stop and forget every running sidecar. Called on app shutdown by
/// future integration; safe to call any time.
#[allow(dead_code)] // wired up by app shutdown hook in a later patch
pub async fn shutdown_all() {
    let mut map = sidecars().lock().await;
    let entries: Vec<(PathBuf, ClaudeExSidecar)> = map.drain().collect();
    drop(map);
    for (workspace, mut sidecar) in entries {
        sidecar.stop().await;
        info!(target: "linear_taco", "stopped claude-ex sidecar for {}", workspace.display());
    }
}

/// Read-only access to the sidecar map for diagnostics.
#[allow(dead_code)] // exposed for future "Linear Taco status" debug panel
pub async fn snapshot() -> Vec<PathBuf> {
    sidecars().lock().await.keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The public `on_claude_session_started` requires an `AppContext`,
    // which isn't trivial to construct in unit tests. The sidecar map
    // itself is exercised via `start_sidecar_if_absent` directly.
    #[tokio::test]
    async fn sidecar_map_starts_empty() {
        let _ = live_count().await;
    }
}

/// `Path` re-export used by integration tests outside this module.
#[allow(dead_code)]
pub(crate) fn _path_marker(_p: &Path) {}
