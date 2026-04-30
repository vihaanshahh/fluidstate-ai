// Task auto-detection backend. Surfaces wire in via a follow-up patch.
#![allow(dead_code)]

//! Minimal Tasks runner — VS Code parity item.
//!
//! Auto-detects runnable tasks in a workspace and exposes a typed list
//! that the future "Run Task…" palette command consumes. v1 supports:
//!
//! - **npm scripts** from `package.json` → `npm run <name>` (or pnpm /
//!   yarn / bun, sniffed from the lockfile present)
//! - **Cargo binaries** discovered from `Cargo.toml` `[[bin]]` entries
//!   plus the implicit root binary when present → `cargo run -p <pkg>`
//!
//! No Python / Make / Just for v1 — the goal is to hit the most common
//! "what does this project actually run" cases without becoming
//! another task framework.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    NpmScript,
    PnpmScript,
    YarnScript,
    BunScript,
    CargoRun,
    CargoTest,
}

impl TaskKind {
    pub fn label_prefix(&self) -> &'static str {
        match self {
            TaskKind::NpmScript => "npm",
            TaskKind::PnpmScript => "pnpm",
            TaskKind::YarnScript => "yarn",
            TaskKind::BunScript => "bun",
            TaskKind::CargoRun => "cargo run",
            TaskKind::CargoTest => "cargo test",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub kind: TaskKind,
    /// Human-readable label, e.g. `npm run dev`.
    pub label: String,
    /// Argv-style command and args, ready for `Command::new(...).args(...)`.
    pub command: String,
    pub args: Vec<String>,
    /// File the task was sourced from, for "open source" affordance.
    pub source: PathBuf,
}

/// Discover every task in the workspace at `root`. Returns an empty
/// vec for non-project directories.
pub fn discover(root: &Path) -> Vec<Task> {
    let mut out = Vec::new();
    out.extend(discover_npm(root));
    out.extend(discover_cargo(root));
    out
}

fn discover_npm(root: &Path) -> Vec<Task> {
    let pkg_path = root.join("package.json");
    let Ok(text) = fs::read_to_string(&pkg_path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let kind = sniff_npm_kind(root);
    let prefix = match kind {
        TaskKind::PnpmScript => ("pnpm", vec!["run".to_string()]),
        TaskKind::YarnScript => ("yarn", vec![]),
        TaskKind::BunScript => ("bun", vec!["run".to_string()]),
        _ => ("npm", vec!["run".to_string()]),
    };

    scripts
        .iter()
        .filter_map(|(name, _)| {
            if name.is_empty() {
                return None;
            }
            let mut args = prefix.1.clone();
            args.push(name.clone());
            Some(Task {
                kind,
                label: format!("{} {}", prefix.0, args.join(" ")),
                command: prefix.0.to_string(),
                args,
                source: pkg_path.clone(),
            })
        })
        .collect()
}

fn sniff_npm_kind(root: &Path) -> TaskKind {
    if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
        return TaskKind::BunScript;
    }
    if root.join("pnpm-lock.yaml").exists() {
        return TaskKind::PnpmScript;
    }
    if root.join("yarn.lock").exists() {
        return TaskKind::YarnScript;
    }
    TaskKind::NpmScript
}

fn discover_cargo(root: &Path) -> Vec<Task> {
    let cargo_path = root.join("Cargo.toml");
    let Ok(text) = fs::read_to_string(&cargo_path) else {
        return Vec::new();
    };

    let mut out = Vec::new();

    // Bin auto-discovery — for v1 we just expose `cargo run` and
    // `cargo test` at the workspace root. Per-bin discovery is a
    // future enhancement; the user can pass --bin in the prompt.
    out.push(Task {
        kind: TaskKind::CargoRun,
        label: "cargo run".to_string(),
        command: "cargo".to_string(),
        args: vec!["run".to_string()],
        source: cargo_path.clone(),
    });
    out.push(Task {
        kind: TaskKind::CargoTest,
        label: "cargo test".to_string(),
        command: "cargo".to_string(),
        args: vec!["test".to_string()],
        source: cargo_path.clone(),
    });

    // Discover individual `[[bin]]` entries by simple line scanning —
    // we don't pull in `toml` here since this module is best-effort
    // and the format is stable enough.
    let mut current_bin_name: Option<String> = None;
    let mut in_bin_table = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[[bin]]" {
            if let Some(name) = current_bin_name.take() {
                out.push(make_cargo_bin_task(&name, &cargo_path));
            }
            in_bin_table = true;
            continue;
        }
        if in_bin_table {
            if trimmed.starts_with('[') && trimmed != "[[bin]]" {
                if let Some(name) = current_bin_name.take() {
                    out.push(make_cargo_bin_task(&name, &cargo_path));
                }
                in_bin_table = false;
                continue;
            }
            if let Some(name) = trimmed.strip_prefix("name") {
                let name = name
                    .trim_start_matches([' ', '=', '"'])
                    .trim_end_matches('"')
                    .trim();
                if !name.is_empty() {
                    current_bin_name = Some(name.to_string());
                }
            }
        }
    }
    if let Some(name) = current_bin_name.take() {
        out.push(make_cargo_bin_task(&name, &cargo_path));
    }

    out
}

fn make_cargo_bin_task(name: &str, source: &Path) -> Task {
    Task {
        kind: TaskKind::CargoRun,
        label: format!("cargo run --bin {name}"),
        command: "cargo".to_string(),
        args: vec!["run".to_string(), "--bin".to_string(), name.to_string()],
        source: source.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn discovers_npm_scripts_with_pnpm_when_lockfile_present() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"dev":"vite","build":"vite build"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
        let tasks = discover(dir.path());
        let labels: Vec<_> = tasks.iter().map(|t| t.label.as_str()).collect();
        assert!(labels.iter().any(|l| *l == "pnpm run dev"));
        assert!(labels.iter().any(|l| *l == "pnpm run build"));
    }

    #[test]
    fn discovers_cargo_run_and_named_bins() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            r#"
[package]
name = "demo"
version = "0.1.0"

[[bin]]
name = "alpha"
path = "src/bin/alpha.rs"

[[bin]]
name = "beta"
path = "src/bin/beta.rs"
"#,
        )
        .unwrap();
        let tasks = discover(dir.path());
        let labels: Vec<_> = tasks.iter().map(|t| t.label.clone()).collect();
        assert!(labels.contains(&"cargo run".to_string()));
        assert!(labels.contains(&"cargo run --bin alpha".to_string()));
        assert!(labels.contains(&"cargo run --bin beta".to_string()));
    }

    #[test]
    fn empty_dir_returns_no_tasks() {
        let dir = TempDir::new().unwrap();
        assert!(discover(dir.path()).is_empty());
    }

    #[test]
    fn yarn_lock_picks_yarn_kind() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"start":"node ."}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("yarn.lock"), "").unwrap();
        let tasks = discover(dir.path());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].kind, TaskKind::YarnScript);
        assert_eq!(tasks[0].command, "yarn");
        assert_eq!(tasks[0].args, vec!["start".to_string()]);
    }
}
