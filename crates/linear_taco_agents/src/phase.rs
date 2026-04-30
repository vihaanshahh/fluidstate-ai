//! Derive [`AgentPhase`] from a stream of transcript events plus a
//! PTY-quiet heartbeat.
//!
//! The mapping (matches the plan):
//!
//! | Phase     | Trigger                                                   |
//! |-----------|-----------------------------------------------------------|
//! | `Idle`    | session created, no input yet, OR finished with prompt    |
//! | `Thinking`| last event = user msg, no assistant token yet, OR streaming text |
//! | `Tool`    | last event = `tool_use`, no matching `tool_result` yet    |
//! | `Awaiting`| permission prompt detected (`mcp__permissions` tool_use)  |
//! | `Done`    | run completed (`type: "result"`)                          |
//! | `Error`   | non-zero exit / `result.is_error` / stderr error pattern  |
//!
//! The `PhaseDeriver` is event-driven and pure — feed it events in order
//! and read [`PhaseDeriver::phase`] after each. The Warp wire-in adds the
//! 500 ms PTY-quiet heartbeat externally to break ties between Tool and
//! Idle when no transcript events have arrived for a while.

use std::collections::HashSet;

use claude_viewer::events::{ContentBlock, TranscriptEvent};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum AgentPhase {
    #[default]
    Idle,
    Thinking,
    Tool,
    Awaiting,
    Done,
    Error,
}


/// Stateful phase deriver. Track a single agent: feed events as they
/// arrive (typically from a `claude_viewer::TranscriptTail`), call
/// [`PhaseDeriver::on_pty_quiet`] when the PTY has been idle for the
/// heartbeat window, and read [`PhaseDeriver::phase`] anytime.
#[derive(Debug, Clone)]
pub struct PhaseDeriver {
    phase: AgentPhase,
    /// Outstanding tool_use ids whose result hasn't arrived yet.
    pending_tools: HashSet<String>,
    last_user_unanswered: bool,
    /// True after a `result` event with `is_error == true`.
    last_run_errored: bool,
    /// True after a `result` event regardless of outcome.
    has_completed_run: bool,
}

impl PhaseDeriver {
    pub fn new() -> Self {
        Self {
            phase: AgentPhase::Idle,
            pending_tools: HashSet::new(),
            last_user_unanswered: false,
            last_run_errored: false,
            has_completed_run: false,
        }
    }

    pub fn phase(&self) -> AgentPhase {
        self.phase
    }

    /// Feed one transcript event. Updates internal state and recomputes
    /// the phase. Returns the new phase.
    pub fn on_event(&mut self, event: &TranscriptEvent) -> AgentPhase {
        match event {
            TranscriptEvent::User(_) => {
                // A user message means we're back in flight — Claude
                // hasn't responded yet.
                self.last_user_unanswered = true;
            }
            TranscriptEvent::Assistant(a) => {
                if let Some(message) = &a.message {
                    for block in &message.content {
                        match block {
                            ContentBlock::ToolUse { id, name, .. } => {
                                self.pending_tools.insert(id.clone());
                                if is_permission_request(name) {
                                    // Awaiting is sticky until a result.
                                    self.phase = AgentPhase::Awaiting;
                                    return self.phase;
                                }
                            }
                            ContentBlock::Text { .. } => {}
                            ContentBlock::ToolResult { tool_use_id, .. } => {
                                self.pending_tools.remove(tool_use_id);
                            }
                            ContentBlock::Other => {}
                        }
                    }
                }
            }
            TranscriptEvent::ToolResult(tr) => {
                if let Some(id) = tr.tool_use_id.as_ref() {
                    self.pending_tools.remove(id);
                }
                if tr.is_error == Some(true) {
                    self.last_run_errored = true;
                }
            }
            TranscriptEvent::Result(r) => {
                self.has_completed_run = true;
                self.last_run_errored = r.is_error.unwrap_or(false);
                self.last_user_unanswered = false;
                self.pending_tools.clear();
            }
            TranscriptEvent::Attachment(_)
            | TranscriptEvent::PermissionMode(_)
            | TranscriptEvent::FileHistorySnapshot(_)
            | TranscriptEvent::Unknown => {}
        }

        self.phase = self.compute();
        self.phase
    }

    /// Heartbeat: the PTY has been quiet for the configured window and
    /// no new transcript events have arrived. Used to settle from `Tool`
    /// to `Idle` when claude-code is genuinely waiting on the user.
    pub fn on_pty_quiet(&mut self) -> AgentPhase {
        if self.pending_tools.is_empty()
            && !self.last_user_unanswered
            && self.has_completed_run
            && !matches!(self.phase, AgentPhase::Awaiting | AgentPhase::Error)
        {
            self.phase = AgentPhase::Idle;
        }
        self.phase
    }

    fn compute(&self) -> AgentPhase {
        if self.last_run_errored {
            return AgentPhase::Error;
        }
        if matches!(self.phase, AgentPhase::Awaiting) && !self.pending_tools.is_empty() {
            return AgentPhase::Awaiting;
        }
        if !self.pending_tools.is_empty() {
            return AgentPhase::Tool;
        }
        if self.last_user_unanswered {
            return AgentPhase::Thinking;
        }
        if self.has_completed_run {
            return AgentPhase::Done;
        }
        AgentPhase::Idle
    }
}

impl Default for PhaseDeriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Permission prompts come through as a tool_use call to one of Warp's
/// or Claude Code's permission-request MCP tools. We treat any tool name
/// containing "permission" as a permission gate.
fn is_permission_request(name: &str) -> bool {
    name.to_ascii_lowercase().contains("permission")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(line: &str) -> TranscriptEvent {
        serde_json::from_str(line).unwrap()
    }

    #[test]
    fn user_message_then_tool_use_settles_to_tool() {
        let mut d = PhaseDeriver::new();
        d.on_event(&ev(r#"{"type":"user","sessionId":"s","message":"hi"}"#));
        assert_eq!(d.phase(), AgentPhase::Thinking);
        d.on_event(&ev(
            r#"{"type":"assistant","sessionId":"s","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        ));
        assert_eq!(d.phase(), AgentPhase::Tool);
    }

    #[test]
    fn tool_result_settles_to_thinking_when_user_pending() {
        let mut d = PhaseDeriver::new();
        d.on_event(&ev(r#"{"type":"user","sessionId":"s","message":"hi"}"#));
        d.on_event(&ev(
            r#"{"type":"assistant","sessionId":"s","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
        ));
        d.on_event(&ev(
            r#"{"type":"tool-result","toolUseId":"t1","content":"ok"}"#,
        ));
        // user is still unanswered (no Result event yet) → Thinking.
        assert_eq!(d.phase(), AgentPhase::Thinking);
    }

    #[test]
    fn permission_tool_use_marks_awaiting() {
        let mut d = PhaseDeriver::new();
        d.on_event(&ev(
            r#"{"type":"assistant","sessionId":"s","message":{"content":[{"type":"tool_use","id":"p1","name":"mcp__permissions__request","input":{}}]}}"#,
        ));
        assert_eq!(d.phase(), AgentPhase::Awaiting);
    }

    #[test]
    fn result_event_marks_done_then_idle_after_quiet() {
        let mut d = PhaseDeriver::new();
        d.on_event(&ev(r#"{"type":"user","sessionId":"s","message":"hi"}"#));
        d.on_event(&ev(
            r#"{"type":"assistant","sessionId":"s","message":{"content":[{"type":"text","text":"done"}]}}"#,
        ));
        d.on_event(&ev(
            r#"{"type":"result","sessionId":"s","stopReason":"end_turn"}"#,
        ));
        assert_eq!(d.phase(), AgentPhase::Done);
        d.on_pty_quiet();
        assert_eq!(d.phase(), AgentPhase::Idle);
    }

    #[test]
    fn errored_result_settles_to_error_and_persists_through_quiet() {
        let mut d = PhaseDeriver::new();
        d.on_event(&ev(r#"{"type":"result","sessionId":"s","isError":true}"#));
        assert_eq!(d.phase(), AgentPhase::Error);
        d.on_pty_quiet();
        assert_eq!(d.phase(), AgentPhase::Error);
    }
}
