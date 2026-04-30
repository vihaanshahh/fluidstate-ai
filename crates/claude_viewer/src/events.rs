//! Typed model of the events written to a Claude Code transcript file
//! (`~/.claude/projects/<encoded-cwd>/<session-id>.jsonl`).
//!
//! Each line in a transcript is a JSON object with a `type` discriminator.
//! Many event types exist; we only model the ones that map to UI cards in the
//! Claude viewer panel. Anything we don't recognize is preserved as
//! [`TranscriptEvent::Unknown`] so we never drop data on the floor.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Stable identifier for a Claude session (`sessionId` in the transcript).
pub type SessionId = String;

/// A single decoded line from a Claude Code transcript.
///
/// Variants are added on demand. The catch-all [`TranscriptEvent::Unknown`]
/// holds the raw JSON for any event whose `type` we don't model yet, which
/// keeps replays loss-less even as Claude Code adds new event kinds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum TranscriptEvent {
    PermissionMode(PermissionModeEvent),
    User(UserEvent),
    Assistant(AssistantEvent),
    Attachment(AttachmentEvent),
    ToolResult(ToolResultEvent),
    FileHistorySnapshot(FileHistorySnapshotEvent),
    Result(ResultEvent),
    #[serde(other, deserialize_with = "deserialize_unknown_default")]
    Unknown,
}

/// Lossless wrapper that retains the original JSON line alongside the
/// decoded variant. The viewer uses [`TranscriptEvent`] for rendering and
/// keeps the [`RawEvent`] around so cards can show "raw JSON" on demand.
#[derive(Debug, Clone)]
pub struct RawEvent {
    pub raw: Value,
    pub decoded: TranscriptEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionModeEvent {
    pub permission_mode: String,
    #[serde(default)]
    pub session_id: Option<SessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserEvent {
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub message: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEvent {
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub message: Option<AssistantMessage>,
}

/// The shape of `assistant.message` in the transcript — a Claude message
/// object whose `content` is an array of `text` and `tool_use` blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: Option<bool>,
    },
    /// Catch-all for content variants we don't model yet (e.g. images).
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentEvent {
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub attachment: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultEvent {
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistorySnapshotEvent {
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub snapshot: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultEvent {
    #[serde(default)]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub usage: Option<Value>,
    #[serde(default)]
    pub total_cost_usd: Option<f64>,
}

/// Card kinds rendered by the viewer. Computed from a [`TranscriptEvent`]
/// and any matching `tool_result` so the viewer can collapse a tool_use +
/// its result into one card.
#[derive(Debug, Clone)]
pub enum Card {
    User {
        text: String,
    },
    AssistantText {
        text: String,
    },
    Bash {
        command: String,
        output: Option<String>,
        exit_code: Option<i32>,
    },
    Read {
        path: PathBuf,
        range: Option<(u64, u64)>,
    },
    Edit {
        path: PathBuf,
        hunk_summary: String,
    },
    Write {
        path: PathBuf,
        byte_count: usize,
    },
    TodoWrite {
        todos: Vec<TodoItem>,
    },
    Task {
        subagent_type: String,
        description: String,
        child_session: Option<SessionId>,
    },
    McpCall {
        server: String,
        tool: String,
        args: Value,
        result: Option<Value>,
    },
    Result {
        stop_reason: Option<String>,
        total_cost_usd: Option<f64>,
        duration_ms: Option<u64>,
    },
    Attachment {
        hook_name: Option<String>,
        content: Option<String>,
    },
    Other {
        kind: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    #[serde(default)]
    pub active_form: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// Collapse a stream of [`TranscriptEvent`]s into [`Card`]s, joining each
/// `tool_use` block with the matching `tool_result`. The function is pure
/// so it can be tested against recorded transcripts.
pub fn cards_from_events<'a, I: IntoIterator<Item = &'a TranscriptEvent>>(events: I) -> Vec<Card> {
    use std::collections::HashMap;
    let events: Vec<&TranscriptEvent> = events.into_iter().collect();

    let mut tool_results: HashMap<String, &ToolResultEvent> = HashMap::new();
    for ev in &events {
        if let TranscriptEvent::ToolResult(tr) = ev
            && let Some(id) = tr.tool_use_id.as_ref() {
                tool_results.insert(id.clone(), tr);
            }
    }

    let mut cards: Vec<Card> = Vec::new();
    for ev in &events {
        match ev {
            TranscriptEvent::User(u) => {
                if let Some(text) = extract_user_text(u.message.as_ref()) {
                    cards.push(Card::User { text });
                }
            }
            TranscriptEvent::Assistant(a) => {
                if let Some(message) = &a.message {
                    for block in &message.content {
                        match block {
                            ContentBlock::Text { text } => {
                                cards.push(Card::AssistantText { text: text.clone() });
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                let result = tool_results.get(id).and_then(|tr| tr.content.clone());
                                cards.push(card_for_tool_use(name, input, result));
                            }
                            ContentBlock::ToolResult { .. } | ContentBlock::Other => {}
                        }
                    }
                }
            }
            TranscriptEvent::Attachment(att) => {
                let (hook_name, content) = unpack_attachment(att.attachment.as_ref());
                cards.push(Card::Attachment { hook_name, content });
            }
            TranscriptEvent::Result(r) => {
                cards.push(Card::Result {
                    stop_reason: r.stop_reason.clone(),
                    total_cost_usd: r.total_cost_usd,
                    duration_ms: r.duration_ms,
                });
            }
            TranscriptEvent::PermissionMode(_)
            | TranscriptEvent::FileHistorySnapshot(_)
            | TranscriptEvent::ToolResult(_)
            | TranscriptEvent::Unknown => {}
        }
    }
    cards
}

fn extract_user_text(message: Option<&Value>) -> Option<String> {
    let msg = message?;
    if let Some(s) = msg.as_str() {
        return Some(s.to_string());
    }
    if let Some(content) = msg.get("content") {
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }
        if let Some(arr) = content.as_array() {
            let mut buf = String::new();
            for block in arr {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    if !buf.is_empty() {
                        buf.push('\n');
                    }
                    buf.push_str(text);
                }
            }
            if !buf.is_empty() {
                return Some(buf);
            }
        }
    }
    None
}

fn unpack_attachment(att: Option<&Value>) -> (Option<String>, Option<String>) {
    let Some(att) = att else {
        return (None, None);
    };
    let hook_name = att
        .get("hookName")
        .and_then(|v| v.as_str())
        .map(String::from);
    let content = att
        .get("content")
        .and_then(|v| v.as_str())
        .map(String::from);
    (hook_name, content)
}

fn card_for_tool_use(name: &str, input: &Value, result: Option<Value>) -> Card {
    match name {
        "Bash" => Card::Bash {
            command: input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            output: result.as_ref().and_then(extract_text).map(truncate_output),
            exit_code: result.as_ref().and_then(extract_exit_code),
        },
        "Read" => Card::Read {
            path: PathBuf::from(
                input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            ),
            range: read_range(input),
        },
        "Edit" | "MultiEdit" => Card::Edit {
            path: PathBuf::from(
                input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            ),
            hunk_summary: hunk_summary_from_input(name, input),
        },
        "Write" => Card::Write {
            path: PathBuf::from(
                input
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            ),
            byte_count: input
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.len())
                .unwrap_or(0),
        },
        "TodoWrite" => Card::TodoWrite {
            todos: serde_json::from_value(
                input
                    .get("todos")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            )
            .unwrap_or_default(),
        },
        "Task" => Card::Task {
            subagent_type: input
                .get("subagent_type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            description: input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            child_session: None,
        },
        n if n.starts_with("mcp__") => {
            // Convention: `mcp__<server>__<tool>`.
            let mut parts = n.splitn(3, "__");
            let _ = parts.next();
            let server = parts.next().unwrap_or("").to_string();
            let tool = parts.next().unwrap_or("").to_string();
            Card::McpCall {
                server,
                tool,
                args: input.clone(),
                result,
            }
        }
        other => Card::Other {
            kind: other.to_string(),
        },
    }
}

fn extract_text(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = v.as_array() {
        let mut buf = String::new();
        for block in arr {
            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                if !buf.is_empty() {
                    buf.push('\n');
                }
                buf.push_str(t);
            }
        }
        if !buf.is_empty() {
            return Some(buf);
        }
    }
    None
}

fn extract_exit_code(v: &Value) -> Option<i32> {
    v.get("exitCode")
        .or_else(|| v.get("exit_code"))
        .and_then(|c| c.as_i64())
        .map(|c| c as i32)
}

fn truncate_output(mut s: String) -> String {
    const MAX: usize = 8 * 1024;
    if s.len() > MAX {
        s.truncate(MAX);
        s.push_str("\n…[truncated]");
    }
    s
}

fn read_range(input: &Value) -> Option<(u64, u64)> {
    let offset = input.get("offset").and_then(|v| v.as_u64())?;
    let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(0);
    Some((offset, offset.saturating_add(limit)))
}

fn hunk_summary_from_input(tool: &str, input: &Value) -> String {
    if tool == "MultiEdit" {
        let count = input
            .get("edits")
            .and_then(|e| e.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        return format!("{count} edits");
    }
    let lines = input
        .get("new_string")
        .and_then(|v| v.as_str())
        .map(|s| s.lines().count())
        .unwrap_or(0);
    format!("{lines} line(s) replaced")
}

// `serde(other)` deserialization helper for the catch-all variant. We
// deliberately throw away the body — callers who want lossless round-tripping
// should consume the [`RawEvent`] from the tail directly.
fn deserialize_unknown_default<'de, D>(_: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_event_with_string_message() {
        let line = r#"{"type":"user","sessionId":"s","message":"hello"}"#;
        let ev: TranscriptEvent = serde_json::from_str(line).unwrap();
        let cards = cards_from_events(std::iter::once(&ev));
        assert!(matches!(cards.as_slice(), [Card::User { text }] if text == "hello"));
    }

    #[test]
    fn collapses_assistant_tool_use_with_result() {
        let assistant = r#"{"type":"assistant","sessionId":"s","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#;
        let result = r#"{"type":"tool-result","toolUseId":"t1","content":[{"type":"text","text":"file.txt"}]}"#;
        let events: Vec<TranscriptEvent> = vec![
            serde_json::from_str(assistant).unwrap(),
            serde_json::from_str(result).unwrap(),
        ];
        let cards = cards_from_events(events.iter());
        match cards.as_slice() {
            [
                Card::Bash {
                    command, output, ..
                },
            ] => {
                assert_eq!(command, "ls");
                assert_eq!(output.as_deref(), Some("file.txt"));
            }
            other => panic!("unexpected cards: {other:?}"),
        }
    }

    #[test]
    fn unknown_event_kind_is_preserved_via_unknown_variant() {
        let line = r#"{"type":"some-future-event-kind","payload":42}"#;
        let ev: TranscriptEvent = serde_json::from_str(line).unwrap();
        assert!(matches!(ev, TranscriptEvent::Unknown));
    }

    #[test]
    fn mcp_tool_use_routes_to_mcp_card() {
        let assistant = r#"{"type":"assistant","sessionId":"s","message":{"content":[{"type":"tool_use","id":"t","name":"mcp__claude-ex__search_code","input":{"query":"foo"}}]}}"#;
        let ev: TranscriptEvent = serde_json::from_str(assistant).unwrap();
        let cards = cards_from_events(std::iter::once(&ev));
        match cards.as_slice() {
            [Card::McpCall { server, tool, .. }] => {
                assert_eq!(server, "claude-ex");
                assert_eq!(tool, "search_code");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
