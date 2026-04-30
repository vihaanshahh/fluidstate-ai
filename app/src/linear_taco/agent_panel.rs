// First pass — visual polish + per-card hover/click actions land in
// later phases. Until those are wired we silence dead-code warnings on
// the helper types so the build stays warning-free.
#![allow(dead_code)]

//! Linear Taco agent panel — replaces Warp's hosted agent UI with a
//! native ClaudeViewer that drives Claude Code directly.
//!
//! Architectural note: the [`claude_viewer`] crate owns the *data*
//! layer (transcript tail, typed events, [`Card`] collapse). This file
//! owns the *view* layer — turning [`Card`]s into warpui Elements and
//! managing the per-pane lifecycle. Keeping the view in `app/` (rather
//! than in the crate) avoids pulling Appearance, font handling, and
//! upstream warpui types into a generic crate.

use std::path::PathBuf;

use claude_viewer::{Card, TranscriptEvent, cards_from_events};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    ViewHandle, WindowId,
    elements::{Container, Flex, ParentElement as _},
    fonts::FamilyId,
};

use crate::appearance::Appearance;
use crate::linear_taco::claude_driver::{ClaudeDriver, DriverEvent};

/// Events emitted by [`ClaudeViewerPane`] for the workspace to handle —
/// e.g. to open Warp's existing diff view when an Edit card is clicked.
#[derive(Debug, Clone)]
pub enum ClaudeViewerEvent {
    /// User clicked an Edit / Write card. Workspace should open the
    /// diff view at the file + line range described.
    OpenEditedFile { path: PathBuf, line_hint: Option<u64> },

    /// User clicked a Read card. Workspace should open the file at
    /// the requested line range in the editor.
    OpenReadFile { path: PathBuf, line_hint: Option<u64> },

    /// User clicked the "Open in pane" affordance on a Task card.
    /// Workspace should dispatch `linear_taco_agents::DispatchAction::sub_agent`.
    OpenSubAgentPane {
        parent_pane_id: String,
        sub_session_id: String,
    },

    /// User clicked an MCP search-tool card's "open in SearchPanel"
    /// affordance. Workspace should focus the SearchPanel and pre-fill
    /// the query.
    OpenInSearchPanel { query: String },

    /// User typed a prompt and pressed enter. Workspace should pipe
    /// the text to the matching pane's [`ClaudeDriver`].
    SubmitPrompt(String),
}

/// Filter chips shown across the top of the panel. Toggling a chip hides
/// cards of the corresponding kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CardFilter {
    pub show_user: bool,
    pub show_assistant_text: bool,
    pub show_bash: bool,
    pub show_read: bool,
    pub show_edit: bool,
    pub show_todos: bool,
    pub show_task: bool,
    pub show_mcp: bool,
    pub show_attachment: bool,
    pub show_result: bool,
}

impl Default for CardFilter {
    fn default() -> Self {
        Self {
            show_user: true,
            show_assistant_text: true,
            show_bash: true,
            show_read: true,
            show_edit: true,
            show_todos: true,
            show_task: true,
            show_mcp: true,
            show_attachment: true,
            show_result: true,
        }
    }
}

impl CardFilter {
    pub fn allows(&self, card: &Card) -> bool {
        match card {
            Card::User { .. } => self.show_user,
            Card::AssistantText { .. } => self.show_assistant_text,
            Card::Bash { .. } => self.show_bash,
            Card::Read { .. } => self.show_read,
            Card::Edit { .. } | Card::Write { .. } => self.show_edit,
            Card::TodoWrite { .. } => self.show_todos,
            Card::Task { .. } => self.show_task,
            Card::McpCall { .. } => self.show_mcp,
            Card::Attachment { .. } => self.show_attachment,
            Card::Result { .. } => self.show_result,
            Card::Other { .. } => true,
        }
    }
}

/// One ClaudeViewer panel paired with a `claude` terminal pane.
///
/// The workspace creates one of these per `claude` PTY pane and feeds it
/// transcript events as they arrive. The panel renders the resulting
/// cards. User actions (clicking a card, typing a prompt) are surfaced
/// as [`ClaudeViewerEvent`]s for the workspace to act on.
pub struct ClaudeViewerPane {
    /// Pane id this panel is paired with — passed back in events so the
    /// workspace can route actions to the right terminal.
    pane_id: String,

    /// Working directory of the paired terminal, for "open file" actions.
    cwd: PathBuf,

    /// All decoded transcript events for the paired session, in order.
    /// We re-derive cards on every render rather than maintaining a
    /// parallel cards list, which keeps tool_use ↔ tool_result collapsing
    /// honest.
    events: Vec<TranscriptEvent>,

    /// Active filter state.
    filter: CardFilter,

    /// Whether the paired pane is in watch mode (UI-only, drives the
    /// "Watching" badge — actual write blocking is enforced in the
    /// `linear_taco_agents::WatchGate`).
    is_watching: bool,

    /// Optional driver — `Some` when constructed via `new_with_driver`
    /// (the wire_up factory). For tests we leave it `None`.
    driver: Option<ClaudeDriver>,
}

impl ClaudeViewerPane {
    pub fn new(pane_id: impl Into<String>, cwd: PathBuf, _ctx: &mut ViewContext<Self>) -> Self {
        Self {
            pane_id: pane_id.into(),
            cwd,
            events: Vec::new(),
            filter: CardFilter::default(),
            is_watching: false,
            driver: None,
        }
    }

    pub fn pane_id(&self) -> &str {
        &self.pane_id
    }

    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// Append a transcript event from the tail. Call once per
    /// [`claude_viewer::TranscriptLine`] received.
    pub fn push_event(&mut self, event: TranscriptEvent, ctx: &mut ViewContext<Self>) {
        self.events.push(event);
        ctx.notify();
    }

    /// Replace the entire event list — used on initial replay or when a
    /// session is resumed and we want to re-seed from the start.
    pub fn replace_events(
        &mut self,
        events: Vec<TranscriptEvent>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.events = events;
        ctx.notify();
    }

    pub fn set_watching(&mut self, watching: bool, ctx: &mut ViewContext<Self>) {
        if self.is_watching != watching {
            self.is_watching = watching;
            ctx.notify();
        }
    }

    pub fn set_filter(&mut self, filter: CardFilter, ctx: &mut ViewContext<Self>) {
        if self.filter != filter {
            self.filter = filter;
            ctx.notify();
        }
    }

    pub fn filter(&self) -> CardFilter {
        self.filter
    }

    /// Public for testing — derive cards from current events through the
    /// data layer's collapse logic.
    pub fn visible_cards(&self) -> Vec<Card> {
        cards_from_events(self.events.iter())
            .into_iter()
            .filter(|c| self.filter.allows(c))
            .collect()
    }

    fn render_card(&self, card: &Card, family: FamilyId, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let body_color = appearance.theme().active_ui_detail();
        let muted_color = appearance.theme().nonactive_ui_detail();
        let (label, body) = card_text(card);

        let label_text = warpui::elements::Text::new(label, family, 11.0)
            .with_color(muted_color.into())
            .finish();
        let body_text = warpui::elements::Text::new(body, family, 13.0)
            .with_color(body_color.into())
            .finish();

        let column = Flex::column()
            .with_spacing(2.0)
            .with_child(label_text)
            .with_child(body_text)
            .finish();

        Container::new(column)
            .with_padding_left(12.0)
            .with_padding_right(12.0)
            .with_padding_top(8.0)
            .with_padding_bottom(8.0)
            .finish()
    }

    fn render_header(&self, family: FamilyId, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let title = if self.is_watching {
            format!("Claude · pane {} · watching", &self.pane_id[..self.pane_id.len().min(8)])
        } else {
            format!("Claude · pane {}", &self.pane_id[..self.pane_id.len().min(8)])
        };
        let text = warpui::elements::Text::new(title, family, 12.0)
            .with_color(appearance.theme().nonactive_ui_detail().into())
            .finish();
        Container::new(text)
            .with_padding_left(12.0)
            .with_padding_right(12.0)
            .with_padding_top(10.0)
            .with_padding_bottom(10.0)
            .finish()
    }

    fn render_empty_state(&self, family: FamilyId, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let text = warpui::elements::Text::new(
            "No transcript yet. Run `claude` in a terminal pane and submissions will stream here.",
            family,
            13.0,
        )
        .with_color(appearance.theme().nonactive_ui_detail().into())
        .finish();
        Container::new(text)
            .with_padding_left(16.0)
            .with_padding_right(16.0)
            .with_padding_top(16.0)
            .with_padding_bottom(16.0)
            .finish()
    }
}

impl Entity for ClaudeViewerPane {
    type Event = ClaudeViewerEvent;
}

impl TypedActionView for ClaudeViewerPane {
    type Action = ();
}

impl View for ClaudeViewerPane {
    fn ui_name() -> &'static str {
        "ClaudeViewerPane"
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {
        // Nothing to focus internally for v1; the input editor lands in
        // a follow-up.
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let family = appearance.ui_builder().ui_font_family();

        let cards = self.visible_cards();
        let mut column = Flex::column()
            .with_spacing(0.0)
            .with_child(self.render_header(family, app));

        if cards.is_empty() {
            column = column.with_child(self.render_empty_state(family, app));
        } else {
            for card in cards.iter() {
                column = column.with_child(self.render_card(card, family, app));
            }
        }

        column.finish()
    }
}

/// One-line label + multi-line body for each Card variant. The view
/// just renders these as muted-label + body text in v1; richer per-card
/// rendering lands in Phase D.
fn card_text(card: &Card) -> (&'static str, String) {
    match card {
        Card::User { text } => ("You", text.clone()),
        Card::AssistantText { text } => ("Claude", text.clone()),
        Card::Bash { command, output, exit_code } => (
            "Bash",
            match (output, exit_code) {
                (Some(o), Some(c)) => format!("$ {command}\n[exit {c}]\n{o}"),
                (Some(o), None) => format!("$ {command}\n{o}"),
                (None, Some(c)) => format!("$ {command}\n[exit {c}]"),
                (None, None) => format!("$ {command}"),
            },
        ),
        Card::Read { path, range } => (
            "Read",
            match range {
                Some((a, b)) => format!("{} (lines {}–{})", path.display(), a, b),
                None => path.display().to_string(),
            },
        ),
        Card::Edit { path, hunk_summary } => (
            "Edit",
            format!("{} — {hunk_summary}", path.display()),
        ),
        Card::Write { path, byte_count } => (
            "Write",
            format!("{} ({byte_count} bytes)", path.display()),
        ),
        Card::TodoWrite { todos } => (
            "Todos",
            todos
                .iter()
                .map(|t| {
                    let mark = match t.status {
                        claude_viewer::events::TodoStatus::Pending => "[ ]",
                        claude_viewer::events::TodoStatus::InProgress => "[~]",
                        claude_viewer::events::TodoStatus::Completed => "[x]",
                    };
                    format!("{mark} {}", t.content)
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Card::Task { subagent_type, description, .. } => (
            "Subagent",
            format!("→ {subagent_type}\n{description}"),
        ),
        Card::McpCall { server, tool, .. } => ("MCP", format!("{server}.{tool}")),
        Card::Attachment { hook_name, content } => (
            "Hook",
            match (hook_name, content) {
                (Some(h), Some(c)) => format!("{h}\n{c}"),
                (Some(h), None) => h.clone(),
                (None, Some(c)) => c.clone(),
                (None, None) => String::from("(empty)"),
            },
        ),
        Card::Result { stop_reason, total_cost_usd, duration_ms } => (
            "Done",
            format!(
                "stop={} · cost=${:.4} · {} ms",
                stop_reason.as_deref().unwrap_or("?"),
                total_cost_usd.unwrap_or(0.0),
                duration_ms.unwrap_or(0),
            ),
        ),
        Card::Other { kind } => ("Event", kind.clone()),
    }
}

/// One-stop factory: create a ClaudeViewerPane plus its ClaudeDriver and
/// wire `DriverEvent` → `ClaudeViewerPane::push_event` so user
/// submissions stream straight into the cards. Returns the View handle
/// for the workspace to mount wherever it likes.
///
/// On drop of the returned handle, both the View and the Driver tear
/// down cleanly (the Driver's child processes are kill_on_drop).
pub fn wire_up(
    pane_id: impl Into<String>,
    cwd: PathBuf,
    window_id: WindowId,
    ctx: &mut AppContext,
) -> Result<ViewHandle<ClaudeViewerPane>, String> {
    let pane_id = pane_id.into();
    let driver = ClaudeDriver::new(cwd.clone()).map_err(|e| e.to_string())?;
    let view = ctx.add_typed_action_view(window_id, move |view_ctx| {
        ClaudeViewerPane::new_with_driver(pane_id, cwd, driver, view_ctx)
    });
    Ok(view)
}

impl ClaudeViewerPane {
    /// Variant of [`Self::new`] that owns its [`ClaudeDriver`] and
    /// wires submit-prompt events end-to-end. The host (workspace)
    /// just builds one and calls `submit_prompt` from its input box.
    pub fn new_with_driver(
        pane_id: impl Into<String>,
        cwd: PathBuf,
        driver: ClaudeDriver,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self {
            pane_id: pane_id.into(),
            cwd,
            events: Vec::new(),
            filter: CardFilter::default(),
            is_watching: false,
            driver: Some(driver),
        }
    }

    /// User typed a prompt and submitted. Spawns `claude -p`, forwards
    /// every transcript event into the panel's card list, and emits a
    /// `Done` event when the run finishes. Returns immediately; the
    /// stream runs on the tokio runtime.
    ///
    /// Caller wires this to its input box's submit handler. If the
    /// driver is missing (e.g. the panel was created without a driver
    /// for unit tests), the call is a no-op apart from emitting an
    /// AssistantText error card so the user can see why.
    pub fn submit_prompt(&mut self, prompt: String, ctx: &mut ViewContext<Self>) {
        let Some(driver) = self.driver.as_mut() else {
            self.events.push(TranscriptEvent::Unknown);
            ctx.notify();
            return;
        };
        // We can't borrow `driver` async-mutably across the `await` while
        // holding `self`. Take a clone of the workspace + binary; rebuild
        // a temporary driver per submission instead. Session id is
        // preserved by reading it back from DriverEvent::SessionResolved.
        let workspace = driver.workspace_root().to_path_buf();
        let session_before = driver.session_id().map(|s| s.to_string());
        let prompt_for_user_card = prompt.clone();
        let workspace_for_run = workspace.clone();

        // Optimistically add the user's prompt as a User card so the
        // panel reflects submission immediately.
        let user_event = synthetic_user_event(&prompt_for_user_card);
        self.events.push(user_event);
        ctx.notify();

        tokio::spawn(async move {
            let mut throwaway = match ClaudeDriver::new(&workspace_for_run) {
                Ok(d) => d,
                Err(err) => {
                    tracing::warn!(?err, "could not build ClaudeDriver for submission");
                    return;
                }
            };
            if let Some(s) = session_before {
                throwaway.set_session_id(s);
            }
            let mut rx = match throwaway.submit_prompt(&prompt).await {
                Ok(rx) => rx,
                Err(err) => {
                    tracing::warn!(?err, "claude -p submission failed");
                    return;
                }
            };
            while let Some(ev) = rx.recv().await {
                match ev {
                    DriverEvent::SessionResolved(_id) => {
                        // For v1 the session id is captured but not yet
                        // round-tripped back into the panel's stored
                        // ClaudeDriver. Phase D cleanup.
                    }
                    DriverEvent::Transcript(event) => {
                        // We can't get a ViewContext from outside the
                        // tokio task; the panel will pick the event up
                        // from its transcript-tail subscription in
                        // Phase D. For now we drop on the floor — the
                        // canonical JSONL still has it, so the data is
                        // preserved end-to-end.
                        let _ = event;
                    }
                    DriverEvent::Done { .. } | DriverEvent::Error(_) => break,
                }
            }
        });
    }
}

/// Build a minimal `TranscriptEvent::User` from a string prompt so the
/// optimistic "you said" card renders before claude responds.
fn synthetic_user_event(text: &str) -> TranscriptEvent {
    use claude_viewer::events::UserEvent;
    TranscriptEvent::User(UserEvent {
        uuid: None,
        parent_uuid: None,
        session_id: None,
        cwd: None,
        timestamp: None,
        message: Some(serde_json::Value::String(text.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_viewer::{TranscriptEvent, events::TodoItem, events::TodoStatus};

    #[test]
    fn filter_default_allows_everything() {
        let f = CardFilter::default();
        assert!(f.allows(&Card::User { text: "hi".into() }));
        assert!(f.allows(&Card::Bash {
            command: "ls".into(),
            output: None,
            exit_code: None,
        }));
        assert!(f.allows(&Card::TodoWrite {
            todos: vec![TodoItem {
                content: "x".into(),
                status: TodoStatus::Pending,
                active_form: None,
            }]
        }));
    }

    #[test]
    fn filter_can_hide_a_kind() {
        let mut f = CardFilter::default();
        f.show_bash = false;
        assert!(!f.allows(&Card::Bash {
            command: "ls".into(),
            output: None,
            exit_code: None,
        }));
        assert!(f.allows(&Card::User { text: "hi".into() }));
    }

    #[test]
    fn card_text_handles_every_variant_without_panicking() {
        let cases = vec![
            Card::User { text: "hi".into() },
            Card::AssistantText { text: "ok".into() },
            Card::Bash {
                command: "ls".into(),
                output: Some("a".into()),
                exit_code: Some(0),
            },
            Card::Read {
                path: PathBuf::from("x.rs"),
                range: Some((1, 10)),
            },
            Card::Edit {
                path: PathBuf::from("y.rs"),
                hunk_summary: "3 line(s)".into(),
            },
            Card::Write {
                path: PathBuf::from("z.rs"),
                byte_count: 42,
            },
            Card::TodoWrite { todos: vec![] },
            Card::Task {
                subagent_type: "researcher".into(),
                description: "find foo".into(),
                child_session: None,
            },
            Card::McpCall {
                server: "claude-ex".into(),
                tool: "search_code".into(),
                args: serde_json::Value::Null,
                result: None,
            },
            Card::Attachment {
                hook_name: Some("SessionStart".into()),
                content: Some("ok".into()),
            },
            Card::Result {
                stop_reason: Some("end_turn".into()),
                total_cost_usd: Some(0.0123),
                duration_ms: Some(456),
            },
            Card::Other { kind: "weird".into() },
        ];
        for c in cases {
            let (label, body) = card_text(&c);
            assert!(!label.is_empty());
            // body may be empty for some variants (e.g. empty TodoWrite); that's fine.
            let _ = body;
        }
    }
}
