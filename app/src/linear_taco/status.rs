// The status snapshot is consumed by the future "Linear Taco · Status"
// command palette entry. Until that's wired up, suppress warnings.
#![allow(dead_code)]

//! Aggregate Linear Taco diagnostic snapshot.
//!
//! Returns a single struct that covers everything a user might want to
//! see in a "Linear Taco status" command palette entry or
//! `linear-taco --status` CLI subcommand:
//!
//! - which workspaces have a live `claude-ex watch` sidecar running
//! - per-workspace claude-ex db detection + table list (proves the
//!   schema we're querying matches what claude-ex actually wrote)
//! - feature flag truth-table for the IDE flags we promoted
//! - resolved RealtyLens theme tokens, so a user can confirm they got
//!   the theme they expect
//!
//! The data is built lazily per-call; callers shouldn't hold onto a
//! snapshot, just render it and drop it.

use std::path::PathBuf;

use linear_taco_claudex::ClaudeExDb;
use serde::Serialize;
use theme_realtylens::{Palette, ThemeMode};
use warp_core::features::FeatureFlag;

#[derive(Debug, Clone, Serialize)]
pub struct LinearTacoStatus {
    pub feature_flags: FeatureFlagStatus,
    pub claudex_workspaces: Vec<WorkspaceStatus>,
    pub theme_tokens: ThemeTokens,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureFlagStatus {
    pub file_tree: bool,
    pub command_palette_file_search: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceStatus {
    pub root: PathBuf,
    pub db_present: bool,
    pub db_tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeTokens {
    pub mode: &'static str,
    pub bg: String,
    pub text_primary: String,
    pub accent: String,
}

/// Build a snapshot. `workspace_roots` is the list to probe — usually
/// the cwds of the claudex sidecar map plus the open workspace tabs.
pub async fn snapshot() -> LinearTacoStatus {
    let workspace_roots = super::claudex_session::snapshot().await;
    let mut workspaces = Vec::with_capacity(workspace_roots.len());
    for root in workspace_roots {
        workspaces.push(probe_workspace(&root));
    }

    LinearTacoStatus {
        feature_flags: FeatureFlagStatus {
            file_tree: FeatureFlag::FileTree.is_enabled(),
            command_palette_file_search: FeatureFlag::CommandPaletteFileSearch.is_enabled(),
        },
        claudex_workspaces: workspaces,
        theme_tokens: theme_tokens_for(ThemeMode::Light),
    }
}

fn probe_workspace(root: &PathBuf) -> WorkspaceStatus {
    let db_path = root.join(".claude-ex.db");
    let db_present = db_path.exists();
    let db_tables = if db_present {
        ClaudeExDb::open(root)
            .ok()
            .and_then(|db| db.detected_tables().ok())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    WorkspaceStatus {
        root: root.clone(),
        db_present,
        db_tables,
    }
}

fn theme_tokens_for(mode: ThemeMode) -> ThemeTokens {
    let p = Palette::for_mode(mode);
    ThemeTokens {
        mode: match mode {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        },
        bg: p.bg.to_hex_string(),
        text_primary: p.text_primary.to_hex_string(),
        accent: p.accent.to_hex_string(),
    }
}

/// Render the snapshot as a human-readable string. Used as the body
/// of the "Linear Taco · Status" notification card.
pub fn render(status: &LinearTacoStatus) -> String {
    let mut out = String::new();
    out.push_str("Linear Taco status\n\n");
    out.push_str("• Feature flags:\n");
    out.push_str(&format!(
        "    FileTree                  {}\n",
        on_off(status.feature_flags.file_tree)
    ));
    out.push_str(&format!(
        "    CommandPaletteFileSearch  {}\n",
        on_off(status.feature_flags.command_palette_file_search)
    ));
    out.push_str(&format!(
        "\n• claude-ex sidecars ({}):\n",
        status.claudex_workspaces.len()
    ));
    if status.claudex_workspaces.is_empty() {
        out.push_str("    (none — start a `claude` session in a workspace)\n");
    } else {
        for w in &status.claudex_workspaces {
            out.push_str(&format!(
                "    {} — db {}, {} tables\n",
                w.root.display(),
                if w.db_present { "present" } else { "missing" },
                w.db_tables.len()
            ));
        }
    }
    out.push_str("\n• Theme (light defaults):\n");
    out.push_str(&format!("    bg            {}\n", status.theme_tokens.bg));
    out.push_str(&format!(
        "    text-primary  {}\n",
        status.theme_tokens.text_primary
    ));
    out.push_str(&format!(
        "    accent        {}\n",
        status.theme_tokens.accent
    ));
    out
}

fn on_off(b: bool) -> &'static str {
    if b {
        "ON"
    } else {
        "off"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_tokens_match_realtylens_constants() {
        let t = theme_tokens_for(ThemeMode::Light);
        assert_eq!(t.bg, "#FEFFFC");
        assert_eq!(t.accent, "#171717");
    }

    #[test]
    fn render_handles_empty_workspaces() {
        let status = LinearTacoStatus {
            feature_flags: FeatureFlagStatus {
                file_tree: true,
                command_palette_file_search: true,
            },
            claudex_workspaces: Vec::new(),
            theme_tokens: theme_tokens_for(ThemeMode::Light),
        };
        let s = render(&status);
        assert!(s.contains("FileTree"));
        assert!(s.contains("(none"));
        assert!(s.contains("#FEFFFC"));
    }
}
