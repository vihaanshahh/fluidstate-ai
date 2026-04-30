// The functions in this module are the SCM backend that the future
// source-control sidebar will call. They're individually wired in as
// the view layer lands, but until then they're a callable, tested
// surface — keep the warnings off so the rest of the build stays
// noise-free.
#![allow(dead_code)]

//! SCM operations for the Linear Taco source-control sidebar.
//!
//! Wraps `git2` (already a dep of the app crate via `vendored-libgit2`)
//! with the surface the SCM sidebar will eventually call:
//!
//! - [`status`] — three-way status grouped into Merge / Staged / Changes
//! - [`stage`] / [`unstage`] — file-level
//! - [`commit`] — message-based commit with author resolution
//! - [`branches`] / [`current_branch`] / [`switch_branch`]
//! - [`recent_commits`] — for the history graph
//! - [`stage_hunk`] — synthesizes a unified diff and applies via
//!   `git apply --cached --unidiff-zero`, the same approach VS Code uses
//!
//! Everything takes a workspace root path; we open a fresh `Repository`
//! per call. libgit2 caches enough internally that this is fine for the
//! frequencies the SCM panel queries at (debounced ~250ms).

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use git2::{BranchType, ErrorCode, Repository, RepositoryOpenFlags, Sort, Status, StatusOptions};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ScmError {
    #[error("not a git repository: {0}")]
    NotARepo(PathBuf),

    #[error("git error: {0}")]
    Git(#[from] git2::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("`git` CLI returned non-zero (exit {0:?}): {1}")]
    GitCli(Option<i32>, String),

    #[error("invalid argument: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Untracked,
    Modified,
    Added,
    Deleted,
    Renamed,
    Conflicted,
    Ignored,
    Unmodified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEntry {
    pub path: PathBuf,
    pub index_status: FileStatus,
    pub worktree_status: FileStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusGroups {
    pub merge: Vec<StatusEntry>,
    pub staged: Vec<StatusEntry>,
    pub changes: Vec<StatusEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_remote: bool,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub oid: String,
    pub short_oid: String,
    pub summary: String,
    pub author_name: String,
    pub author_email: String,
    pub time_unix: i64,
}

fn open_repo(workspace_root: &Path) -> Result<Repository, ScmError> {
    Repository::open_ext(
        workspace_root,
        RepositoryOpenFlags::empty(),
        std::iter::empty::<&Path>(),
    )
    .map_err(|e| {
        if e.code() == ErrorCode::NotFound {
            ScmError::NotARepo(workspace_root.to_path_buf())
        } else {
            ScmError::Git(e)
        }
    })
}

/// Group `git status --porcelain` output into the three sections the
/// SCM sidebar renders. Files in conflict appear in `merge`, anything
/// with an index change in `staged`, anything with a worktree change in
/// `changes`. A file may appear in both `staged` and `changes` if it
/// has changes in both stages — that mirrors VS Code's UI.
pub fn status(workspace_root: &Path) -> Result<StatusGroups, ScmError> {
    let repo = open_repo(workspace_root)?;
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true);

    let statuses = repo.statuses(Some(&mut opts))?;
    let mut groups = StatusGroups::default();

    for entry in statuses.iter() {
        let path = match entry.path() {
            Some(p) => PathBuf::from(p),
            None => continue,
        };
        let s = entry.status();
        let index_status = index_status_from(s);
        let worktree_status = worktree_status_from(s);

        if s.contains(Status::CONFLICTED) {
            groups.merge.push(StatusEntry {
                path: path.clone(),
                index_status: FileStatus::Conflicted,
                worktree_status: FileStatus::Conflicted,
            });
            continue;
        }
        if s.intersects(
            Status::INDEX_NEW
                | Status::INDEX_MODIFIED
                | Status::INDEX_DELETED
                | Status::INDEX_RENAMED
                | Status::INDEX_TYPECHANGE,
        ) {
            groups.staged.push(StatusEntry {
                path: path.clone(),
                index_status,
                worktree_status,
            });
        }
        if s.intersects(
            Status::WT_NEW
                | Status::WT_MODIFIED
                | Status::WT_DELETED
                | Status::WT_RENAMED
                | Status::WT_TYPECHANGE,
        ) {
            groups.changes.push(StatusEntry {
                path,
                index_status,
                worktree_status,
            });
        }
    }
    Ok(groups)
}

fn index_status_from(s: Status) -> FileStatus {
    if s.contains(Status::INDEX_NEW) {
        FileStatus::Added
    } else if s.contains(Status::INDEX_MODIFIED) {
        FileStatus::Modified
    } else if s.contains(Status::INDEX_DELETED) {
        FileStatus::Deleted
    } else if s.contains(Status::INDEX_RENAMED) {
        FileStatus::Renamed
    } else if s.is_ignored() {
        FileStatus::Ignored
    } else {
        FileStatus::Unmodified
    }
}

fn worktree_status_from(s: Status) -> FileStatus {
    if s.contains(Status::WT_NEW) {
        FileStatus::Untracked
    } else if s.contains(Status::WT_MODIFIED) {
        FileStatus::Modified
    } else if s.contains(Status::WT_DELETED) {
        FileStatus::Deleted
    } else if s.contains(Status::WT_RENAMED) {
        FileStatus::Renamed
    } else {
        FileStatus::Unmodified
    }
}

/// Stage one or more workspace-relative paths.
pub fn stage(workspace_root: &Path, paths: &[&Path]) -> Result<(), ScmError> {
    let repo = open_repo(workspace_root)?;
    let mut index = repo.index()?;
    for p in paths {
        // libgit2 wants "add_all" semantics for both modified and untracked.
        index.add_all([*p].iter(), git2::IndexAddOption::DEFAULT, None)?;
    }
    index.write()?;
    Ok(())
}

/// Unstage paths back to HEAD. Equivalent to `git reset HEAD <paths>`.
pub fn unstage(workspace_root: &Path, paths: &[&Path]) -> Result<(), ScmError> {
    let repo = open_repo(workspace_root)?;
    // Use `repo.reset_default` which resets paths in the index to their
    // HEAD versions without touching the worktree.
    let head_obj = repo
        .head()
        .ok()
        .and_then(|h| h.peel(git2::ObjectType::Commit).ok());
    repo.reset_default(head_obj.as_ref(), paths.iter().map(|p| p.as_os_str()))?;
    Ok(())
}

/// Stage a single hunk by applying a synthesized unified diff to the
/// index. Mirrors VS Code's per-hunk staging — we shell out to the
/// system `git` rather than fight libgit2's apply API, which is awkward
/// for partial patches.
pub fn stage_hunk(workspace_root: &Path, unified_diff: &str) -> Result<(), ScmError> {
    git_apply_cached(workspace_root, unified_diff, false)
}

/// Unstage a single hunk by reverse-applying the same diff to the
/// index.
pub fn unstage_hunk(workspace_root: &Path, unified_diff: &str) -> Result<(), ScmError> {
    git_apply_cached(workspace_root, unified_diff, true)
}

fn git_apply_cached(
    workspace_root: &Path,
    unified_diff: &str,
    reverse: bool,
) -> Result<(), ScmError> {
    use std::io::Write;
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(workspace_root);
    cmd.args(["apply", "--cached", "--unidiff-zero"]);
    if reverse {
        cmd.arg("--reverse");
    }
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(unified_diff.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return Err(ScmError::GitCli(output.status.code(), stderr));
    }
    Ok(())
}

/// Commit the index with `message`. Author is resolved from the repo's
/// configured `user.name` + `user.email`. Returns the new commit's oid.
pub fn commit(workspace_root: &Path, message: &str) -> Result<String, ScmError> {
    if message.trim().is_empty() {
        return Err(ScmError::Invalid("commit message is empty".into()));
    }
    let repo = open_repo(workspace_root)?;
    let signature = repo.signature()?;

    let mut index = repo.index()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let parents: Vec<git2::Commit<'_>> = match repo.head() {
        Ok(head) => head
            .peel(git2::ObjectType::Commit)
            .ok()
            .and_then(|o| o.into_commit().ok())
            .map(|c| vec![c])
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let parent_refs: Vec<&git2::Commit<'_>> = parents.iter().collect();

    let oid = repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        message,
        &tree,
        &parent_refs,
    )?;
    Ok(oid.to_string())
}

pub fn current_branch(workspace_root: &Path) -> Result<Option<String>, ScmError> {
    let repo = open_repo(workspace_root)?;
    let head = match repo.head() {
        Ok(h) => h,
        Err(e) if e.code() == ErrorCode::UnbornBranch || e.code() == ErrorCode::NotFound => {
            return Ok(None)
        }
        Err(e) => return Err(e.into()),
    };
    Ok(head.shorthand().map(String::from))
}

pub fn branches(workspace_root: &Path) -> Result<Vec<BranchInfo>, ScmError> {
    let repo = open_repo(workspace_root)?;
    let current = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().map(String::from));

    let mut out = Vec::new();
    for entry in repo.branches(None)? {
        let (branch, kind) = entry?;
        let Some(name) = branch.name()?.map(String::from) else {
            continue;
        };
        let is_remote = matches!(kind, BranchType::Remote);
        let is_current = current.as_deref() == Some(&name);
        let mut upstream = None;
        let mut ahead = 0usize;
        let mut behind = 0usize;
        if !is_remote {
            if let Ok(up) = branch.upstream() {
                upstream = up.name()?.map(String::from);
                if let (Some(local_oid), Some(up_oid)) = (branch.get().target(), up.get().target())
                {
                    if let Ok((a, b)) = repo.graph_ahead_behind(local_oid, up_oid) {
                        ahead = a;
                        behind = b;
                    }
                }
            }
        }
        out.push(BranchInfo {
            name,
            is_remote,
            is_current,
            upstream,
            ahead,
            behind,
        });
    }
    Ok(out)
}

/// Switch to an existing local branch. Caller is responsible for
/// dirty-tree confirmation.
pub fn switch_branch(workspace_root: &Path, branch_name: &str) -> Result<(), ScmError> {
    let repo = open_repo(workspace_root)?;
    let (object, reference) = repo.revparse_ext(branch_name)?;
    repo.checkout_tree(&object, None)?;
    if let Some(r) = reference {
        repo.set_head(r.name().unwrap_or(branch_name))?;
    } else {
        repo.set_head_detached(object.id())?;
    }
    Ok(())
}

/// Last `limit` commits on the current branch, newest-first. Powers
/// the HistoryGraph view.
pub fn recent_commits(workspace_root: &Path, limit: usize) -> Result<Vec<CommitInfo>, ScmError> {
    let repo = open_repo(workspace_root)?;
    let mut walk = repo.revwalk()?;
    walk.set_sorting(Sort::TIME | Sort::TOPOLOGICAL)?;
    if walk.push_head().is_err() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(limit);
    for oid in walk.take(limit) {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let author = commit.author();
        out.push(CommitInfo {
            oid: oid.to_string(),
            short_oid: oid.to_string().chars().take(7).collect(),
            summary: commit.summary().unwrap_or("").to_string(),
            author_name: author.name().unwrap_or("").to_string(),
            author_email: author.email().unwrap_or("").to_string(),
            time_unix: commit.time().seconds(),
        });
    }
    Ok(out)
}

/// Whether the working tree has any uncommitted changes. Used to gate
/// the dirty-tree warning when switching branches.
pub fn is_dirty(workspace_root: &Path) -> Result<bool, ScmError> {
    let groups = status(workspace_root)?;
    Ok(!groups.merge.is_empty() || !groups.staged.is_empty() || !groups.changes.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_repo(dir: &Path) {
        Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "linear-taco@example.com"])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Linear Taco Test"])
            .current_dir(dir)
            .status()
            .unwrap();
    }

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn status_classifies_untracked_and_staged() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        write(dir.path(), "a.txt", "alpha");
        let groups = status(dir.path()).unwrap();
        assert_eq!(groups.changes.len(), 1);
        assert_eq!(groups.staged.len(), 0);

        stage(dir.path(), &[Path::new("a.txt")]).unwrap();
        let groups = status(dir.path()).unwrap();
        assert_eq!(groups.staged.len(), 1);
        assert_eq!(groups.staged[0].index_status, FileStatus::Added);
    }

    #[test]
    fn commit_returns_oid_and_clears_status() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        write(dir.path(), "hello.txt", "hi");
        stage(dir.path(), &[Path::new("hello.txt")]).unwrap();
        let oid = commit(dir.path(), "initial").unwrap();
        assert_eq!(oid.len(), 40);
        let groups = status(dir.path()).unwrap();
        assert!(groups.changes.is_empty() && groups.staged.is_empty());
    }

    #[test]
    fn current_branch_and_recent_commits() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        write(dir.path(), "x.txt", "x");
        stage(dir.path(), &[Path::new("x.txt")]).unwrap();
        commit(dir.path(), "x").unwrap();

        assert_eq!(current_branch(dir.path()).unwrap().as_deref(), Some("main"));

        let commits = recent_commits(dir.path(), 5).unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].summary, "x");
        assert_eq!(commits[0].author_email, "linear-taco@example.com");
    }

    #[test]
    fn is_dirty_reflects_status() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        assert!(!is_dirty(dir.path()).unwrap());
        write(dir.path(), "drift.txt", "drift");
        assert!(is_dirty(dir.path()).unwrap());
    }

    #[test]
    fn unstage_returns_file_to_changes() {
        let dir = TempDir::new().unwrap();
        init_repo(dir.path());
        write(dir.path(), "y.txt", "y");
        stage(dir.path(), &[Path::new("y.txt")]).unwrap();
        commit(dir.path(), "y0").unwrap();
        // Modify, stage, then unstage.
        write(dir.path(), "y.txt", "y2");
        stage(dir.path(), &[Path::new("y.txt")]).unwrap();
        let staged_before = status(dir.path()).unwrap().staged.len();
        assert_eq!(staged_before, 1);
        unstage(dir.path(), &[Path::new("y.txt")]).unwrap();
        let groups = status(dir.path()).unwrap();
        assert_eq!(groups.staged.len(), 0);
        assert_eq!(groups.changes.len(), 1);
    }

    #[test]
    fn empty_dir_is_not_a_repo() {
        let dir = TempDir::new().unwrap();
        match status(dir.path()) {
            Err(ScmError::NotARepo(_)) => {}
            other => panic!("expected NotARepo, got {other:?}"),
        }
    }
}
