// Backend for the FindInFiles panel — view wires up later.
#![allow(dead_code)]

//! Workspace-wide find / replace backed by ripgrep.
//!
//! Warp already ships `crates/warp_ripgrep/` for in-block search but
//! that's scoped to terminal output. The IDE-level "find in files"
//! panel needs cross-file results with line/column context plus a
//! replace-all preview, which is closer to what `rg --json` gives.
//!
//! We shell out to the system `rg` rather than statically linking. The
//! binary is universally available as a Vercel-style dev dep, and
//! shelling out keeps the streaming model simple — line-by-line stdout
//! with `--json`.

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};

#[derive(Debug, thiserror::Error)]
pub enum FindError {
    #[error("ripgrep not found on PATH; install via `brew install ripgrep`")]
    RgMissing,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("rg exited with status {0:?}")]
    NonZero(Option<i32>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindMatch {
    pub path: PathBuf,
    pub line_number: u64,
    pub column_start: u64,
    pub column_end: u64,
    /// The full matched line (with trailing newline stripped).
    pub line: String,
    /// Just the substring that matched the query, useful for highlighting.
    pub matched_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindOptions {
    pub case_insensitive: bool,
    pub regex: bool,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub max_matches: Option<usize>,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            case_insensitive: false,
            regex: false,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            max_matches: Some(2_000),
        }
    }
}

/// Run a workspace-wide search. Streams matches via `tokio::process` so
/// huge result sets stay responsive. Returns when ripgrep exits or
/// `max_matches` is hit.
pub async fn find_in_files(
    workspace_root: &Path,
    query: &str,
    options: &FindOptions,
) -> Result<Vec<FindMatch>, FindError> {
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let rg = which_rg().ok_or(FindError::RgMissing)?;

    let mut cmd = Command::new(rg);
    cmd.arg("--json")
        .arg("--no-config")
        .arg("--hidden")
        .arg("--glob")
        .arg("!.git")
        .arg("--glob")
        .arg("!node_modules")
        .arg("--glob")
        .arg("!target")
        .current_dir(workspace_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if options.case_insensitive {
        cmd.arg("--ignore-case");
    }
    if !options.regex {
        cmd.arg("--fixed-strings");
    }
    for glob in &options.include_globs {
        cmd.arg("--glob").arg(glob);
    }
    for glob in &options.exclude_globs {
        cmd.arg("--glob").arg(format!("!{glob}"));
    }
    cmd.arg("--").arg(query);

    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("piped");
    let mut reader = BufReader::new(stdout).lines();
    let limit = options.max_matches.unwrap_or(usize::MAX);

    let mut out: Vec<FindMatch> = Vec::new();
    while let Some(line) = reader.next_line().await? {
        if out.len() >= limit {
            // Reached the cap — kill the child so we don't keep
            // streaming.
            let _ = child.start_kill();
            break;
        }
        if let Some(m) = parse_rg_json_line(&line) {
            out.push(m);
        }
    }
    let status = child.wait().await?;
    // ripgrep returns 1 when there are no matches; that's not an error.
    if !status.success() && status.code() != Some(1) && out.is_empty() {
        return Err(FindError::NonZero(status.code()));
    }
    Ok(out)
}

fn which_rg() -> Option<PathBuf> {
    if let Some(env) = std::env::var_os("RG_PATH") {
        let p = PathBuf::from(env);
        if p.is_file() {
            return Some(p);
        }
    }
    which::which("rg").ok()
}

/// Parse one JSON line emitted by `rg --json`. We only care about
/// `match` events; everything else (`begin`, `end`, `summary`) is
/// ignored.
fn parse_rg_json_line(line: &str) -> Option<FindMatch> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("match") {
        return None;
    }
    let data = v.get("data")?;
    let path = data
        .get("path")
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())?;
    let lines_text = data
        .get("lines")
        .and_then(|l| l.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let line_number = data
        .get("line_number")
        .and_then(|n| n.as_u64())
        .unwrap_or(0);
    let submatches = data.get("submatches").and_then(|s| s.as_array())?;
    let first = submatches.first()?;
    let col_start = first.get("start").and_then(|n| n.as_u64()).unwrap_or(0);
    let col_end = first
        .get("end")
        .and_then(|n| n.as_u64())
        .unwrap_or(col_start);
    let matched_text = first
        .get("match")
        .and_then(|m| m.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    Some(FindMatch {
        path: PathBuf::from(path),
        line_number,
        column_start: col_start,
        column_end: col_end,
        line: lines_text.trim_end_matches('\n').to_string(),
        matched_text,
    })
}

/// Generate a replacement preview without writing anything. Returns the
/// new content for each match grouped by file. Apply with
/// [`apply_replace`] once the user confirms.
pub fn preview_replace(matches: &[FindMatch], replacement: &str) -> Vec<ReplacePreview> {
    use std::collections::BTreeMap;
    let mut by_path: BTreeMap<PathBuf, Vec<&FindMatch>> = BTreeMap::new();
    for m in matches {
        by_path.entry(m.path.clone()).or_default().push(m);
    }
    by_path
        .into_iter()
        .map(|(path, ms)| ReplacePreview {
            replacements: ms
                .iter()
                .map(|m| LineReplacement {
                    line_number: m.line_number,
                    before: m.line.clone(),
                    after: m
                        .line
                        .chars()
                        .enumerate()
                        .fold(String::new(), |mut acc, (i, c)| {
                            let i = i as u64;
                            if i >= m.column_start && i < m.column_end {
                                if i == m.column_start {
                                    acc.push_str(replacement);
                                }
                                // Skip chars inside the match window.
                            } else {
                                acc.push(c);
                            }
                            acc
                        }),
                })
                .collect(),
            path,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplacePreview {
    pub path: PathBuf,
    pub replacements: Vec<LineReplacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineReplacement {
    pub line_number: u64,
    pub before: String,
    pub after: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rg_json_line_extracts_match_fields() {
        let line = r#"{"type":"match","data":{"path":{"text":"src/lib.rs"},"lines":{"text":"fn foo() {\n"},"line_number":42,"submatches":[{"match":{"text":"foo"},"start":3,"end":6}]}}"#;
        let m = parse_rg_json_line(line).unwrap();
        assert_eq!(m.path, PathBuf::from("src/lib.rs"));
        assert_eq!(m.line_number, 42);
        assert_eq!(m.column_start, 3);
        assert_eq!(m.column_end, 6);
        assert_eq!(m.matched_text, "foo");
        assert_eq!(m.line, "fn foo() {");
    }

    #[test]
    fn parse_rg_json_line_ignores_non_match() {
        let line = r#"{"type":"begin","data":{"path":{"text":"foo"}}}"#;
        assert!(parse_rg_json_line(line).is_none());
    }

    #[test]
    fn preview_replace_groups_by_path() {
        let m1 = FindMatch {
            path: PathBuf::from("a.rs"),
            line_number: 1,
            column_start: 0,
            column_end: 3,
            line: "foo bar".to_string(),
            matched_text: "foo".to_string(),
        };
        let m2 = FindMatch {
            path: PathBuf::from("b.rs"),
            line_number: 5,
            column_start: 4,
            column_end: 7,
            line: "baz foo".to_string(),
            matched_text: "foo".to_string(),
        };
        let previews = preview_replace(&[m1, m2], "qux");
        assert_eq!(previews.len(), 2);
        let a = previews
            .iter()
            .find(|p| p.path == PathBuf::from("a.rs"))
            .unwrap();
        assert_eq!(a.replacements[0].after, "qux bar");
        let b = previews
            .iter()
            .find(|p| p.path == PathBuf::from("b.rs"))
            .unwrap();
        assert_eq!(b.replacements[0].after, "baz qux");
    }
}
