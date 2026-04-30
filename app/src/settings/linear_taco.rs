//! Linear Taco user-facing settings.
//!
//! Anything Linear Taco-specific that the user can configure goes here.
//! The macro takes care of registration, schema generation, and TOML
//! file mapping; we just declare the keys.

use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};

define_settings_group!(LinearTacoSettings, settings: [
    // Whether sub-agents (panes spawned via Claude Code's `Task` tool)
    // open in watch mode by default. Watch mode blocks PTY writes at the
    // IPC boundary, letting the user observe the sub-agent without
    // accidentally typing into it.
    watch_subagents_default: WatchSubagentsDefault {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "linear_taco.agents.watch_subagents_by_default",
        description: "Open sub-agent panes (spawned via Task) in watch mode by default.",
    }
    // Show the headless-dispatch tray. When enabled, the palette command
    // "Dispatch headless" places the run in a top-right tray that can be
    // promoted to a full pane via `claude --resume`.
    headless_tray_visible: HeadlessTrayVisible {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "linear_taco.dispatch.headless_tray_visible",
        description: "Show the headless-dispatch tray for `claude -p` background runs.",
    }
    // Whether opening a workspace where Claude Code starts should
    // auto-spawn the `claude-ex watch` sidecar. Off → no .claude-ex.db
    // is created automatically.
    claudex_autostart: ClaudexAutostart {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "linear_taco.claudex.autostart",
        description: "Auto-start the `claude-ex watch` sidecar when a `claude` session begins in a workspace.",
    }
    // Whether to merge Linear Taco's MCP entry and SessionStart /
    // PreToolUse / PostToolUse hooks into `<workspace>/.claude/settings.json`
    // when a `claude` session starts. Off → user is responsible for
    // configuring claude-ex by hand.
    claudex_settings_merge: ClaudexSettingsMerge {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "linear_taco.claudex.merge_settings_json",
        description: "Merge Linear Taco's MCP entry and hooks into the workspace's .claude/settings.json on `claude` start.",
    }
    // Whether to mount the structured ClaudeViewer panel beside each
    // `claude` terminal pane. Off → only the raw TUI is shown.
    claude_viewer_enabled: ClaudeViewerEnabled {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "linear_taco.claude_viewer.enabled",
        description: "Mount the structured ClaudeViewer panel beside each `claude` terminal pane.",
    }
    // Whether to show the AgentDashboard's tree-sort indicator hairline
    // connecting parent/child agent panes in the grid.
    show_agent_lineage: ShowAgentLineage {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::DESKTOP,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "linear_taco.agents.show_lineage",
        description: "Render hairlines between parent and child agent panes in the grid.",
    }
]);
