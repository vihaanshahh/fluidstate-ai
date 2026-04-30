// View-side wiring lands when the workspace mounts the dashboard
// modal. Until then we suppress dead-code warnings on the helper
// types that the future modal code will consume.
#![allow(dead_code)]

//! AgentDashboard — Cmd+Shift+A overlay listing every Claude pane
//! across every open workspace.
//!
//! The data layer (`linear_taco_agents::DashboardView`) already
//! computes typed rows + sorts + bulk-action predicates. This view
//! just renders those rows as warpui Elements and routes row-click
//! events back to the workspace.

use linear_taco_agents::{AgentPhase, AgentStateStore, DashboardSort, DashboardView};
use warpui::{
    AppContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View, ViewContext,
    elements::{Container, Flex, ParentElement as _, Text},
    fonts::FamilyId,
};

use crate::appearance::Appearance;

/// Events the dashboard emits — workspace listens and acts.
#[derive(Debug, Clone)]
pub enum AgentDashboardEvent {
    /// Open the pane represented by this row in its workspace.
    Focus { pane_id: String },

    /// Bulk-action: close panes matching this predicate.
    CloseAllIdle,
    CloseAllDone,
    RestartAllErrored,

    /// Modal closed.
    Dismissed,
}

pub struct AgentDashboardView {
    store: AgentStateStore,
    sort: DashboardSort,
}

impl AgentDashboardView {
    pub fn new(store: AgentStateStore, _ctx: &mut ViewContext<Self>) -> Self {
        Self {
            store,
            sort: DashboardSort::Phase,
        }
    }

    pub fn set_store(&mut self, store: AgentStateStore, ctx: &mut ViewContext<Self>) {
        self.store = store;
        ctx.notify();
    }

    pub fn set_sort(&mut self, sort: DashboardSort, ctx: &mut ViewContext<Self>) {
        if self.sort != sort {
            self.sort = sort;
            ctx.notify();
        }
    }

    pub fn sort(&self) -> DashboardSort {
        self.sort
    }

    /// Return the rendered rows for the current sort. Public for
    /// testing — view code calls this internally too.
    pub fn rows(&self) -> Vec<linear_taco_agents::dashboard::DashboardRow> {
        DashboardView::new(&self.store).rows(self.sort)
    }

    fn render_row(
        &self,
        row: &linear_taco_agents::dashboard::DashboardRow,
        family: FamilyId,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let body = appearance.theme().active_ui_detail();
        let muted = appearance.theme().nonactive_ui_detail();

        let phase_label = phase_short(row.phase);
        let cost_str = match row.cost_usd {
            Some(c) => format!("${c:.4}"),
            None => "—".to_string(),
        };
        let indent = "  ".repeat(row.depth);
        let title = format!("{indent}{} · {}", phase_label, row.title);
        let subtitle = format!(
            "{}  ·  {}{}",
            cost_str,
            if row.watch { "watching" } else { "" },
            if row.pinned { "  (pinned)" } else { "" },
        );

        let title_el = Text::new(title, family, 13.0).with_color(body.into()).finish();
        let subtitle_el = Text::new(subtitle, family, 11.0).with_color(muted.into()).finish();
        let column = Flex::column()
            .with_spacing(2.0)
            .with_child(title_el)
            .with_child(subtitle_el)
            .finish();
        Container::new(column)
            .with_padding_left(14.0)
            .with_padding_right(14.0)
            .with_padding_top(8.0)
            .with_padding_bottom(8.0)
            .finish()
    }

    fn render_header(&self, family: FamilyId, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let muted = appearance.theme().nonactive_ui_detail();
        let title = format!(
            "Agents · sort: {}",
            match self.sort {
                DashboardSort::Phase => "phase",
                DashboardSort::LastActivity => "last activity",
                DashboardSort::Cost => "cost",
                DashboardSort::Tree => "tree",
            }
        );
        let text = Text::new(title, family, 12.0).with_color(muted.into()).finish();
        Container::new(text)
            .with_padding_left(14.0)
            .with_padding_right(14.0)
            .with_padding_top(10.0)
            .with_padding_bottom(10.0)
            .finish()
    }
}

impl Entity for AgentDashboardView {
    type Event = AgentDashboardEvent;
}

impl TypedActionView for AgentDashboardView {
    type Action = ();
}

impl View for AgentDashboardView {
    fn ui_name() -> &'static str {
        "AgentDashboardView"
    }

    fn on_focus(&mut self, _focus_ctx: &FocusContext, _ctx: &mut ViewContext<Self>) {}

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let family = appearance.ui_builder().ui_font_family();

        let mut col = Flex::column()
            .with_spacing(0.0)
            .with_child(self.render_header(family, app));

        let rows = self.rows();
        if rows.is_empty() {
            let muted = appearance.theme().nonactive_ui_detail();
            col = col.with_child(
                Container::new(
                    Text::new(
                        "No agents yet. Open a workspace and run `claude` in a terminal pane.",
                        family,
                        13.0,
                    )
                    .with_color(muted.into())
                    .finish(),
                )
                .with_padding_left(16.0)
                .with_padding_right(16.0)
                .with_padding_top(16.0)
                .with_padding_bottom(16.0)
                .finish(),
            );
        } else {
            for row in &rows {
                col = col.with_child(self.render_row(row, family, app));
            }
        }
        col.finish()
    }
}

fn phase_short(p: AgentPhase) -> &'static str {
    match p {
        AgentPhase::Idle => "idle",
        AgentPhase::Thinking => "think",
        AgentPhase::Tool => "tool",
        AgentPhase::Awaiting => "wait",
        AgentPhase::Done => "done",
        AgentPhase::Error => "err",
    }
}

/// Helper that derives the visible Cards for a given pane from the
/// dashboard's view of the world. Used by the AgentDashboard's "expand
/// row" action when implemented in Phase D — for now exposed so tests
/// can lock in the contract.
#[cfg(test)]
pub fn _link_to_card_filter() -> CardFilter {
    CardFilter::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use linear_taco_agents::{AgentState, PaneId};
    use std::path::PathBuf;

    #[test]
    fn empty_store_renders_no_rows() {
        let store = AgentStateStore::new();
        let dash_view = DashboardView::new(&store);
        assert!(dash_view.rows(DashboardSort::Phase).is_empty());
    }

    #[test]
    fn rows_change_with_sort() {
        let mut store = AgentStateStore::new();
        let mut a = AgentState::new(PaneId::new("a"), PathBuf::from("/"), "alpha");
        a.phase = AgentPhase::Idle;
        a.last_activity_unix_ms = 1;
        let mut b = AgentState::new(PaneId::new("b"), PathBuf::from("/"), "beta");
        b.phase = AgentPhase::Awaiting;
        b.last_activity_unix_ms = 2;
        store.upsert(a);
        store.upsert(b);

        let by_phase: Vec<_> = DashboardView::new(&store)
            .rows(DashboardSort::Phase)
            .into_iter()
            .map(|r| r.pane_id.0)
            .collect();
        assert_eq!(by_phase, vec!["b", "a"]);

        let by_activity: Vec<_> = DashboardView::new(&store)
            .rows(DashboardSort::LastActivity)
            .into_iter()
            .map(|r| r.pane_id.0)
            .collect();
        assert_eq!(by_activity, vec!["b", "a"]);
    }

    #[test]
    fn phase_short_covers_every_variant() {
        for phase in [
            AgentPhase::Idle,
            AgentPhase::Thinking,
            AgentPhase::Tool,
            AgentPhase::Awaiting,
            AgentPhase::Done,
            AgentPhase::Error,
        ] {
            assert!(!phase_short(phase).is_empty());
        }
    }
}
