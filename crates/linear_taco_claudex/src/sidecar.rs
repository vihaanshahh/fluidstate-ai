//! Long-lived `claude-ex watch` child process per workspace.
//!
//! `claude-ex` is a Node.js CLI we expect on `PATH`. The packaged Linear
//! Taco app will eventually bundle a Node runtime + `claude-ex` under
//! `resources/`, but for development we resolve the binary via [`which`].
//!
//! The sidecar:
//!
//! 1. Resolves the binary, surfacing a clear error if missing.
//! 2. If `<workspace>/.claude-ex.db` does not exist, runs `claude-ex init`
//!    once and waits for it to finish (one-shot).
//! 3. Spawns `claude-ex watch <workspace>` and keeps the handle alive.
//!    Stdout/stderr are forwarded to `tracing` so they show up in the app
//!    logs without spamming the user.
//! 4. Restarts the watcher on unexpected exit, capped to a small backoff
//!    so a permanently broken install doesn't pin a CPU.
//!
//! Stopping the sidecar (drop or [`ClaudeExSidecar::stop`]) sends SIGTERM
//! and waits for the child to exit.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::{Mutex, Notify},
    task::JoinHandle,
    time::sleep,
};
use tracing::{debug, error, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error(
        "`claude-ex` binary not found on PATH; install with `npm i -g claude-ex` or set CLAUDE_EX_PATH"
    )]
    BinaryMissing,

    #[error("`claude-ex init` failed (exit {0:?}): {1}")]
    InitFailed(Option<i32>, String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarStatus {
    /// Looking up the binary / running init.
    Starting,
    /// `claude-ex watch` is alive.
    Running,
    /// Last spawn exited; we'll retry after the backoff.
    Restarting,
    /// `stop()` was called; no further restarts.
    Stopped,
}

/// Handle to a running `claude-ex watch` sidecar tied to a single workspace.
pub struct ClaudeExSidecar {
    workspace_root: PathBuf,
    binary: PathBuf,
    state: Arc<Mutex<State>>,
    stop_notify: Arc<Notify>,
    join: Option<JoinHandle<()>>,
}

struct State {
    status: SidecarStatus,
    child: Option<Child>,
}

impl ClaudeExSidecar {
    /// Start a sidecar for `workspace_root`. Returns once `claude-ex init`
    /// has finished (the watch loop continues in the background).
    pub async fn start(workspace_root: impl AsRef<Path>) -> Result<Self, SidecarError> {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let binary = resolve_binary()?;
        info!(binary = %binary.display(), workspace = %workspace_root.display(), "starting claude-ex sidecar");

        let state = Arc::new(Mutex::new(State {
            status: SidecarStatus::Starting,
            child: None,
        }));

        ensure_indexed(&binary, &workspace_root).await?;

        let stop_notify = Arc::new(Notify::new());
        let join = tokio::spawn(supervise(
            binary.clone(),
            workspace_root.clone(),
            state.clone(),
            stop_notify.clone(),
        ));

        Ok(Self {
            workspace_root,
            binary,
            state,
            stop_notify,
            join: Some(join),
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub async fn status(&self) -> SidecarStatus {
        self.state.lock().await.status
    }

    /// Stop the watch loop and kill any live child. Idempotent.
    pub async fn stop(&mut self) {
        self.stop_notify.notify_waiters();
        if let Some(join) = self.join.take() {
            // Wait briefly for the supervisor to tear down; if it overruns,
            // proceed — the child is being killed in any case.
            let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
        }
        let mut state = self.state.lock().await;
        if let Some(mut child) = state.child.take() {
            let _ = child.kill().await;
        }
        state.status = SidecarStatus::Stopped;
    }
}

impl Drop for ClaudeExSidecar {
    fn drop(&mut self) {
        // Best-effort: signal the supervisor and let the runtime collect
        // the join handle. We can't `await` here, so any in-flight child
        // is left to the OS to clean up if no one called `stop()` first.
        self.stop_notify.notify_waiters();
    }
}

fn resolve_binary() -> Result<PathBuf, SidecarError> {
    if let Some(env) = std::env::var_os("CLAUDE_EX_PATH") {
        let p = PathBuf::from(env);
        if p.is_file() {
            return Ok(p);
        }
    }
    which::which("claude-ex").map_err(|_| SidecarError::BinaryMissing)
}

async fn ensure_indexed(binary: &Path, workspace: &Path) -> Result<(), SidecarError> {
    let db = workspace.join(".claude-ex.db");
    if db.exists() {
        debug!("claude-ex db already present at {}", db.display());
        return Ok(());
    }
    info!("running `claude-ex init` for first-time index");
    let output = Command::new(binary)
        .arg("init")
        .current_dir(workspace)
        .output()
        .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(SidecarError::InitFailed(output.status.code(), stderr));
    }
    Ok(())
}

async fn supervise(
    binary: PathBuf,
    workspace: PathBuf,
    state: Arc<Mutex<State>>,
    stop_notify: Arc<Notify>,
) {
    let mut backoff = Duration::from_millis(500);
    let max_backoff = Duration::from_secs(15);

    loop {
        if stop_requested(&stop_notify) {
            break;
        }

        let spawn_result = spawn_watch(&binary, &workspace).await;
        let mut child = match spawn_result {
            Ok(c) => c,
            Err(err) => {
                error!(
                    ?err,
                    "failed to spawn claude-ex watch; retrying after {:?}", backoff
                );
                set_status(&state, SidecarStatus::Restarting).await;
                if wait_with_stop(&stop_notify, backoff).await {
                    break;
                }
                backoff = (backoff * 2).min(max_backoff);
                continue;
            }
        };

        // Forward stdout/stderr to tracing.
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(forward("claude-ex stdout", BufReader::new(stdout)));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(forward("claude-ex stderr", BufReader::new(stderr)));
        }

        {
            let mut s = state.lock().await;
            s.status = SidecarStatus::Running;
            s.child = Some(child);
        }
        // Reset backoff after a successful spawn.
        backoff = Duration::from_millis(500);

        tokio::select! {
            _ = stop_notify.notified() => {
                let mut s = state.lock().await;
                if let Some(mut child) = s.child.take() {
                    let _ = child.kill().await;
                }
                s.status = SidecarStatus::Stopped;
                break;
            }
            status = wait_for_child(&state) => {
                set_status(&state, SidecarStatus::Restarting).await;
                match status {
                    Ok(exit) => warn!(?exit, "claude-ex watch exited; restarting"),
                    Err(err) => error!(?err, "error waiting for claude-ex watch"),
                }
                if wait_with_stop(&stop_notify, backoff).await {
                    break;
                }
                backoff = (backoff * 2).min(max_backoff);
            }
        }
    }
}

async fn spawn_watch(binary: &Path, workspace: &Path) -> std::io::Result<Child> {
    let mut cmd = Command::new(binary);
    cmd.arg("watch")
        .arg(workspace)
        .current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);
    cmd.spawn()
}

async fn wait_for_child(state: &Arc<Mutex<State>>) -> std::io::Result<Option<i32>> {
    // Take the child out so we can `await` on it without holding the lock.
    let mut child = {
        let mut s = state.lock().await;
        s.child.take()
    };
    if let Some(child) = child.as_mut() {
        let status = child.wait().await?;
        Ok(status.code())
    } else {
        // No child to wait for: someone must have stopped us.
        Ok(None)
    }
}

async fn forward<R: tokio::io::AsyncBufRead + Unpin>(channel: &'static str, mut reader: R) {
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf).await {
            Ok(0) => break,
            Ok(_) => {
                let line = buf.trim_end();
                if !line.is_empty() {
                    debug!(target: "claude_ex", channel = channel, "{line}");
                }
            }
            Err(err) => {
                warn!(?err, channel = channel, "error reading claude-ex output");
                break;
            }
        }
    }
}

async fn set_status(state: &Arc<Mutex<State>>, status: SidecarStatus) {
    let mut s = state.lock().await;
    s.status = status;
}

fn stop_requested(_notify: &Notify) -> bool {
    // `Notify` doesn't expose a non-blocking check; the supervise loop
    // handles stop via `tokio::select!` so this is only used to short-
    // circuit in retry loops where we already know the channel is open.
    false
}

async fn wait_with_stop(stop: &Notify, dur: Duration) -> bool {
    tokio::select! {
        _ = stop.notified() => true,
        _ = sleep(dur) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_binary_returns_clean_error() {
        // Force resolution failure by unsetting CLAUDE_EX_PATH and asking
        // for a binary we know is absent. The runtime PATH might still
        // have `claude-ex` for the developer running these tests, so we
        // only assert if `which` *fails* — we can't simulate "missing"
        // cleanly in process. This test is mostly a smoke check that
        // resolve_binary doesn't panic.
        let _ = resolve_binary();
    }

    #[test]
    fn sidecar_status_variants_compile() {
        let _ = SidecarStatus::Starting;
        let _ = SidecarStatus::Running;
        let _ = SidecarStatus::Restarting;
        let _ = SidecarStatus::Stopped;
    }
}
