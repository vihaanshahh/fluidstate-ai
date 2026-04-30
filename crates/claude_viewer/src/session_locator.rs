//! Resolves which transcript file belongs to a given workspace pane.
//!
//! Claude Code stores transcripts under
//! `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` where `encoded-cwd`
//! is the workspace path with `/` replaced by `-`. A workspace can have many
//! sessions; we pick the one whose mtime is most recent at-or-after the
//! moment the pane was spawned. That handles fresh `claude` runs and
//! `claude --resume` equally — both write to the same file.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result};

use crate::events::SessionId;

/// Encode a workspace cwd the same way Claude Code does: replace each path
/// separator with `-` and strip a leading separator. Matches the directory
/// layout we observed under `~/.claude/projects/`.
pub fn encode_cwd(cwd: &Path) -> String {
    let mut s = cwd.to_string_lossy().replace('/', "-");
    if s.starts_with('-') {
        s.remove(0);
    }
    // Prepend a single "-" so an absolute path (`/Users/...`) becomes
    // `-Users-...` to match what Claude Code writes.
    format!("-{s}")
}

/// Default project directory for Claude Code transcripts.
pub fn default_projects_root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME env var is not set")?;
    Ok(home.join(".claude").join("projects"))
}

/// Locator that picks the transcript file for a (cwd, pane spawn time) pair.
#[derive(Debug, Clone)]
pub struct SessionLocator {
    projects_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LocatedSession {
    pub session_id: SessionId,
    pub transcript_path: PathBuf,
    pub modified: SystemTime,
}

impl SessionLocator {
    pub fn new(projects_root: impl Into<PathBuf>) -> Self {
        Self {
            projects_root: projects_root.into(),
        }
    }

    pub fn from_home() -> Result<Self> {
        Ok(Self::new(default_projects_root()?))
    }

    /// Path to the workspace-specific transcript directory for `cwd`.
    /// May not exist yet (no Claude session started there).
    pub fn workspace_dir(&self, cwd: &Path) -> PathBuf {
        self.projects_root.join(encode_cwd(cwd))
    }

    /// List all transcripts for `cwd`, sorted newest-first by mtime.
    pub fn list(&self, cwd: &Path) -> Result<Vec<LocatedSession>> {
        let dir = self.workspace_dir(cwd);
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(err) => return Err(err).with_context(|| format!("read_dir {}", dir.display())),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            out.push(LocatedSession {
                session_id: stem.to_string(),
                transcript_path: path,
                modified,
            });
        }
        out.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(out)
    }

    /// Return the freshest transcript whose mtime is ≥ `pane_spawned_at`,
    /// minus a small grace window to absorb clock skew between the pane
    /// spawn and Claude Code's first write. Returns `None` if no transcript
    /// has been written since the pane started.
    pub fn freshest_after(
        &self,
        cwd: &Path,
        pane_spawned_at: SystemTime,
        grace: Duration,
    ) -> Result<Option<LocatedSession>> {
        let cutoff = pane_spawned_at
            .checked_sub(grace)
            .unwrap_or(pane_spawned_at);
        let mut sessions = self.list(cwd)?;
        sessions.retain(|s| s.modified >= cutoff);
        Ok(sessions.into_iter().next())
    }

    /// Look up a transcript by session id without requiring a pane context.
    pub fn by_session_id(&self, cwd: &Path, session_id: &str) -> PathBuf {
        self.workspace_dir(cwd).join(format!("{session_id}.jsonl"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, time::Duration};
    use tempfile::TempDir;

    fn touch_jsonl(dir: &Path, name: &str) {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"{}\n").unwrap();
    }

    #[test]
    fn encode_cwd_matches_observed_layout() {
        assert_eq!(
            encode_cwd(Path::new("/Users/vihaan/Documents/GitHub/estate")),
            "-Users-vihaan-Documents-GitHub-estate"
        );
    }

    #[test]
    fn list_sorts_newest_first() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let cwd = Path::new("/foo/bar");
        let proj = root.join(encode_cwd(cwd));
        std::fs::create_dir_all(&proj).unwrap();
        touch_jsonl(&proj, "older.jsonl");
        std::thread::sleep(Duration::from_millis(20));
        touch_jsonl(&proj, "newer.jsonl");

        let locator = SessionLocator::new(&root);
        let sessions = locator.list(cwd).unwrap();
        assert_eq!(sessions[0].session_id, "newer");
        assert_eq!(sessions[1].session_id, "older");
    }

    #[test]
    fn freshest_after_filters_by_pane_spawn() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let cwd = Path::new("/foo/bar");
        let proj = root.join(encode_cwd(cwd));
        std::fs::create_dir_all(&proj).unwrap();
        touch_jsonl(&proj, "before.jsonl");

        let cutoff = SystemTime::now() + Duration::from_millis(100);
        let locator = SessionLocator::new(&root);
        let result = locator
            .freshest_after(cwd, cutoff, Duration::from_millis(0))
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn missing_workspace_dir_is_empty_not_error() {
        let dir = TempDir::new().unwrap();
        let locator = SessionLocator::new(dir.path());
        let sessions = locator.list(Path::new("/no/such/cwd")).unwrap();
        assert!(sessions.is_empty());
    }
}
