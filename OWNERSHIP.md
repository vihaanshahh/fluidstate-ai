# Linear Taco — Ownership

This repository is a fork of [warpdotdev/warp](https://github.com/warpdotdev/warp) carrying additions for **Linear Taco** — a Claude Code IDE built on the Warp client.

Linear Taco is licensed under **AGPL-3.0** (inherited from Warp's predominant license). Files added by Linear Taco that depend exclusively on the MIT-licensed `warpui_core` / `warpui` crates may carry an MIT notice.

## Branch convention

- `master` — clean upstream mirror; rebased weekly off `origin/master`.
- `linear-taco` — primary working branch carrying all Linear Taco additions.
- Feature branches off `linear-taco` follow the upstream `<author>/<topic>` convention.

## Linear Taco-owned paths

These directories and files are owned by Linear Taco. Treat their contents as additions to upstream Warp, not modifications.

```
crates/claude_viewer/            # JSONL transcript tail + structured Claude UI
crates/linear_taco_agents/       # AgentPhase model, dispatcher, watch gate, dashboard
crates/linear_taco_claudex/      # claude-ex sidecar + RO sqlite + SearchPanel
crates/theme_realtylens/         # RealtyLens palette, Inter font, easings
resources/claude-ex/             # bundled claude-ex npm package + node runtime
OWNERSHIP.md                     # this file
```

## Upstream files we patch

These upstream Warp files carry small Linear Taco modifications. Keep diffs minimal and well-isolated to ease weekly rebases.

| Path | Reason |
|---|---|
| `Cargo.toml` (workspace) | Add the four new crates as members + workspace deps. |
| `app/src/lib.rs` | Register Linear Taco views and workspace-open hook. |
| `app/src/workspace/view.rs` *(or equivalent)* | Mount SearchPanel, AgentDashboard route, ClaudeViewer slot. |
| `app/src/terminal/cli_agent_sessions/` | Detect `claude` argv → mount ClaudeViewer side panel. |
| `app/src/themes/`, `app/src/themes/palette.rs` | Register `realtylens_light` + `realtylens_dark`; default to light. |
| `app/src/code_review/`, `app/src/code_review/git_dialog/` | Extend into a full SCM sidebar. |
| `crates/warp_features/` | Promote `file_tree`, `command_palette_file_search` to default-on. |
| `crates/ai/mcp/` | Workspace-scoped registration of claude-ex as an MCP server. |
| `crates/settings/` | Add `linear_taco.*` keys. |
| `crates/onboarding/` | Replace flow with one-screen "Open a workspace." |

## Upstream sync policy

- Rebase `linear-taco` onto `origin/master` weekly; resolve conflicts in the table above first.
- Our four new crates are self-contained — they should never produce merge conflicts.
- `theme_realtylens` deliberately depends only on the **MIT** `warpui_core`, so it could be relicensed if ever extracted.

## Naming

Internal binary name remains `warp` for v1. The Linear Taco display name lives in the `WarpDisplayName` setting and the About box.
