//! Merges Linear Taco's MCP server entry and Claude Code hooks into a
//! workspace's `.claude/settings.json`, preserving any existing keys.
//!
//! The exact shape we want to land in `.claude/settings.json`:
//!
//! ```jsonc
//! {
//!   "mcpServers": {
//!     "claude-ex": { "command": "claude-ex", "args": ["mcp"] }
//!   },
//!   "hooks": {
//!     "SessionStart": [
//!       { "matcher": "startup", "hooks": [
//!           { "type": "command", "command": "claude-ex brief" }
//!       ] }
//!     ],
//!     "PreToolUse":  [
//!       { "matcher": "Write|Edit|Read", "hooks": [
//!           { "type": "command", "command": "claude-ex pre-edit" }
//!       ] }
//!     ],
//!     "PostToolUse": [
//!       { "matcher": "Write|Edit|MultiEdit", "hooks": [
//!           { "type": "command", "command": "claude-ex post-edit" }
//!       ] }
//!     ]
//!   }
//! }
//! ```
//!
//! Anything else already in the file (themes, tool permissions, other MCP
//! servers, custom hook entries) is left alone. Re-running the merge is
//! idempotent.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Map, Value, json};

#[derive(Debug, thiserror::Error)]
pub enum SettingsWriteError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("settings.json is not a JSON object")]
    NotAnObject,

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Merge Linear Taco's MCP + hook entries into the file at
/// `<workspace>/.claude/settings.json`, creating it if absent. Returns the
/// final path written.
pub fn merge_settings(workspace_root: &Path) -> Result<PathBuf, SettingsWriteError> {
    let settings_dir = workspace_root.join(".claude");
    std::fs::create_dir_all(&settings_dir)
        .with_context(|| format!("create_dir_all {}", settings_dir.display()))?;
    let settings_path = settings_dir.join("settings.json");

    let mut current: Value = match std::fs::read_to_string(&settings_path) {
        Ok(text) if text.trim().is_empty() => json!({}),
        Ok(text) => serde_json::from_str(&text)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => return Err(err.into()),
    };

    apply_merge(&mut current)?;

    let pretty = serde_json::to_string_pretty(&current)?;
    std::fs::write(&settings_path, pretty)?;
    Ok(settings_path)
}

fn apply_merge(value: &mut Value) -> Result<(), SettingsWriteError> {
    let obj = value
        .as_object_mut()
        .ok_or(SettingsWriteError::NotAnObject)?;
    upsert_mcp_server(obj);
    upsert_hooks(obj);
    Ok(())
}

fn upsert_mcp_server(obj: &mut Map<String, Value>) {
    let servers = obj
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(map) = servers.as_object_mut() else {
        // A non-object `mcpServers` is unexpected; replace with our entry.
        *servers = json!({
            "claude-ex": { "command": "claude-ex", "args": ["mcp"] }
        });
        return;
    };
    map.insert(
        "claude-ex".to_string(),
        json!({ "command": "claude-ex", "args": ["mcp"] }),
    );
}

fn upsert_hooks(obj: &mut Map<String, Value>) {
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(hooks_map) = hooks.as_object_mut() else {
        *hooks = default_hooks();
        return;
    };
    upsert_hook_entry(hooks_map, "SessionStart", "startup", "claude-ex brief");
    upsert_hook_entry(
        hooks_map,
        "PreToolUse",
        "Write|Edit|Read",
        "claude-ex pre-edit",
    );
    upsert_hook_entry(
        hooks_map,
        "PostToolUse",
        "Write|Edit|MultiEdit",
        "claude-ex post-edit",
    );
}

fn upsert_hook_entry(
    hooks_map: &mut Map<String, Value>,
    event_name: &str,
    matcher: &str,
    command: &str,
) {
    let entries = hooks_map
        .entry(event_name.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(arr) = entries.as_array_mut() else {
        *entries = json!([{
            "matcher": matcher,
            "hooks": [{ "type": "command", "command": command }]
        }]);
        return;
    };

    // Find an existing entry with this matcher; if it has our command,
    // leave it. Otherwise append our hook to its `hooks` list.
    for existing in arr.iter_mut() {
        let Some(existing_obj) = existing.as_object_mut() else {
            continue;
        };
        if existing_obj.get("matcher").and_then(|v| v.as_str()) != Some(matcher) {
            continue;
        }
        let inner = existing_obj
            .entry("hooks".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(inner_arr) = inner.as_array_mut() else {
            *inner = json!([{ "type": "command", "command": command }]);
            return;
        };
        let already_present = inner_arr
            .iter()
            .any(|h| h.get("command").and_then(|v| v.as_str()) == Some(command));
        if !already_present {
            inner_arr.push(json!({ "type": "command", "command": command }));
        }
        return;
    }

    arr.push(json!({
        "matcher": matcher,
        "hooks": [{ "type": "command", "command": command }]
    }));
}

fn default_hooks() -> Value {
    json!({
        "SessionStart": [{
            "matcher": "startup",
            "hooks": [{ "type": "command", "command": "claude-ex brief" }]
        }],
        "PreToolUse": [{
            "matcher": "Write|Edit|Read",
            "hooks": [{ "type": "command", "command": "claude-ex pre-edit" }]
        }],
        "PostToolUse": [{
            "matcher": "Write|Edit|MultiEdit",
            "hooks": [{ "type": "command", "command": "claude-ex post-edit" }]
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn creates_settings_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = merge_settings(dir.path()).unwrap();
        assert!(path.exists());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            v["mcpServers"]["claude-ex"]["command"].as_str(),
            Some("claude-ex")
        );
        assert!(
            v["hooks"]["SessionStart"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("claude-ex brief")
        );
    }

    #[test]
    fn preserves_unrelated_keys() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir(&claude_dir).unwrap();
        let pre_existing = json!({
            "theme": "warp-classic",
            "permissions": { "allow": ["bash"] },
            "mcpServers": {
                "puppeteer": { "command": "puppeteer-mcp" }
            }
        });
        std::fs::write(
            claude_dir.join("settings.json"),
            serde_json::to_string_pretty(&pre_existing).unwrap(),
        )
        .unwrap();

        merge_settings(dir.path()).unwrap();

        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(claude_dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(v["theme"].as_str(), Some("warp-classic"));
        assert_eq!(v["permissions"]["allow"][0].as_str(), Some("bash"));
        // Other server preserved.
        assert_eq!(
            v["mcpServers"]["puppeteer"]["command"].as_str(),
            Some("puppeteer-mcp")
        );
        // Our server added alongside.
        assert_eq!(
            v["mcpServers"]["claude-ex"]["command"].as_str(),
            Some("claude-ex")
        );
    }

    #[test]
    fn merge_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let path = merge_settings(dir.path()).unwrap();
        let after_first = std::fs::read_to_string(&path).unwrap();
        let _ = merge_settings(dir.path()).unwrap();
        let after_second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after_first, after_second, "second merge should be a no-op");
    }

    #[test]
    fn does_not_duplicate_hook_with_same_matcher_and_command() {
        let dir = TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir(&claude_dir).unwrap();
        // User has already wired up the SessionStart entry by hand.
        let pre = json!({
            "hooks": {
                "SessionStart": [{
                    "matcher": "startup",
                    "hooks": [{ "type": "command", "command": "claude-ex brief" }]
                }]
            }
        });
        std::fs::write(
            claude_dir.join("settings.json"),
            serde_json::to_string_pretty(&pre).unwrap(),
        )
        .unwrap();

        merge_settings(dir.path()).unwrap();
        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(claude_dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        let inner = v["hooks"]["SessionStart"][0]["hooks"].as_array().unwrap();
        let count = inner
            .iter()
            .filter(|h| h["command"].as_str() == Some("claude-ex brief"))
            .count();
        assert_eq!(count, 1, "hook should not be duplicated");
    }
}
