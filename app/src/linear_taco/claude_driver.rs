// Driver wires up in Phase C alongside the workspace mount.
#![allow(dead_code)]

//! Drives one Claude Code session per pane.
//!
//! v1 strategy: each user submission runs `claude -p <prompt>
//! --output-format stream-json` (headless, one-shot). The first
//! invocation creates a new session and we capture its id from the
//! stream-json header; subsequent invocations append to the same
//! session via `--resume <session_id>`. This avoids fighting the
//! interactive `claude` TUI's stdin reader and gives us clean
//! line-delimited NDJSON to forward to the [`agent_panel::ClaudeViewerPane`].
//!
//! Why not pipe to a long-lived `claude` process: the TUI manages its
//! own readline, alt-screen, etc. Pasting structured input into stdin
//! is fragile. With `--print`/`--resume` each turn is a fresh process
//! which is much easier to reason about; conversation continuity is
//! provided by claude's session-id mechanism, not by us.
//!
//! All transcript events also land in the canonical JSONL under
//! `~/.claude/projects/<cwd>/<session>.jsonl`, so the
//! [`ClaudeViewerPane`] reads the same file the user's other panes do.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use claude_viewer::events::TranscriptEvent;
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tracing::{debug, warn};

#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    #[error(
        "`claude` binary not found on PATH; install Claude Code from \
         https://docs.anthropic.com/en/docs/claude-code or set CLAUDE_PATH"
    )]
    BinaryMissing,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("claude exited with status {0:?}: {1}")]
    NonZero(Option<i32>, String),
}

/// Stream events emitted while a claude-p invocation runs. Mirrors what
/// gets written to the transcript JSONL but lets the panel surface the
/// run before the file tail catches up.
#[derive(Debug, Clone)]
pub enum DriverEvent {
    /// Driver has decided which session this run targets. Emitted once
    /// per `submit_prompt` call.
    SessionResolved(String),

    /// One decoded transcript event. Same shape as the JSONL tail emits.
    Transcript(TranscriptEvent),

    /// The `claude -p` process exited.
    Done { exit_code: Option<i32> },

    /// Anything we couldn't interpret. Surfaces a short error string the
    /// panel can render as a faint badge.
    Error(String),
}

/// One driver per `claude` ClaudeViewer pane. Holds the session id once
/// learned and lets the panel submit follow-up prompts via `--resume`.
pub struct ClaudeDriver {
    workspace_root: PathBuf,
    binary: PathBuf,
    /// `None` until the first submission has been answered. Set once we
    /// see a `permission-mode` or `attachment` event with a sessionId.
    session_id: Option<String>,
}

impl ClaudeDriver {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Result<Self, DriverError> {
        let binary = resolve_binary()?;
        Ok(Self {
            workspace_root: workspace_root.into(),
            binary,
            session_id: None,
        })
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn set_session_id(&mut self, id: impl Into<String>) {
        self.session_id = Some(id.into());
    }

    /// Submit a user prompt. Returns a receiver of [`DriverEvent`]s
    /// streamed live while `claude -p` runs. The receiver closes when
    /// the process exits.
    ///
    /// Idempotent across submissions: if `session_id` is already set,
    /// the run uses `--resume`; otherwise it starts a fresh session and
    /// we capture the new session id from the first event we see.
    pub async fn submit_prompt(
        &mut self,
        prompt: &str,
    ) -> Result<mpsc::UnboundedReceiver<DriverEvent>, DriverError> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-p")
            .arg(prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .current_dir(&self.workspace_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(session) = &self.session_id {
            cmd.arg("--resume").arg(session);
        }

        let mut child = cmd.spawn()?;
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");

        let (tx, rx) = mpsc::unbounded_channel();
        let tx_for_stderr = tx.clone();
        let known_session = self.session_id.clone();
        let session_slot = self.session_slot_handle();

        // stdout reader — parse stream-json lines.
        let tx_stdout = tx.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(value) => {
                        // Capture session id on first sight if we don't have one yet.
                        if known_session.is_none() {
                            if let Some(id) =
                                value.get("sessionId").and_then(|v| v.as_str())
                            {
                                let _ = tx_stdout.send(DriverEvent::SessionResolved(id.to_string()));
                                if let Some(slot) = &session_slot {
                                    let _ = slot.send(id.to_string());
                                }
                            }
                        }
                        match serde_json::from_value::<TranscriptEvent>(value.clone()) {
                            Ok(event) => {
                                let _ = tx_stdout.send(DriverEvent::Transcript(event));
                            }
                            Err(err) => {
                                debug!(?err, "failed to decode stream-json line; forwarding raw");
                                let _ = tx_stdout.send(DriverEvent::Error(format!(
                                    "decode: {err}"
                                )));
                            }
                        }
                    }
                    Err(err) => {
                        warn!(?err, line = %line.chars().take(120).collect::<String>(),
                              "claude stream-json line was not valid JSON");
                    }
                }
            }
        });

        // stderr reader — surface as Error events so the panel can show them.
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let _ = tx_for_stderr.send(DriverEvent::Error(trimmed.to_string()));
                }
            }
        });

        // Waiter — emits Done when the child exits.
        let tx_done = tx;
        tokio::spawn(async move {
            let exit = match child.wait().await {
                Ok(status) => status.code(),
                Err(err) => {
                    let _ = tx_done.send(DriverEvent::Error(format!("wait: {err}")));
                    None
                }
            };
            let _ = tx_done.send(DriverEvent::Done { exit_code: exit });
        });

        Ok(rx)
    }

    /// One-shot oneshot channel that the stdout-reader uses to ping us
    /// back with the discovered session id. We thread the sender into
    /// the reader task.
    fn session_slot_handle(&self) -> Option<mpsc::UnboundedSender<String>> {
        // For v1 we don't actually need bi-directional ownership; the
        // session id is also emitted via `DriverEvent::SessionResolved`
        // which the panel picks up and feeds back via `set_session_id`.
        // Returning `None` keeps the API simple.
        None
    }
}

fn resolve_binary() -> Result<PathBuf, DriverError> {
    if let Some(env) = std::env::var_os("CLAUDE_PATH") {
        let p = PathBuf::from(env);
        if p.is_file() {
            return Ok(p);
        }
    }
    which::which("claude").map_err(|_| DriverError::BinaryMissing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_session_id_is_none() {
        // `new` will fail if `claude` isn't on PATH on this machine;
        // skip cleanly in that case so the test suite stays green for
        // contributors without Claude Code installed.
        let Ok(driver) = ClaudeDriver::new("/tmp") else {
            return;
        };
        assert!(driver.session_id().is_none());
    }

    #[test]
    fn set_session_id_persists() {
        let Ok(mut driver) = ClaudeDriver::new("/tmp") else {
            return;
        };
        driver.set_session_id("abc-123");
        assert_eq!(driver.session_id(), Some("abc-123"));
    }
}
