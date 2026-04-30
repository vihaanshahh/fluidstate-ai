//! Sortable, filterable view over the agent state store for the
//! AgentDashboard overlay (Cmd+Shift+A).
//!
//! The dashboard is data-only here — the renderer maps these rows to
//! warpui_core widgets in the integration layer. Keeping the projection
//! pure means we can unit-test sorting and bulk actions without a UI.

use serde::{Deserialize, Serialize};

use crate::{
    lineage::Lineage,
    phase::AgentPhase,
    state::{AgentState, AgentStateStore, PaneId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DashboardSort {
    /// Awaiting first, then Tool, then Thinking, then Done, then Error,
    /// then Idle. Within a group, oldest activity first.
    Phase,
    /// Newest activity first.
    LastActivity,
    /// Highest cost first; null cost last.
    Cost,
    /// Pre-order parent → children walk.
    Tree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardRow {
    pub pane_id: PaneId,
    pub title: String,
    pub phase: AgentPhase,
    pub last_activity_unix_ms: u64,
    pub cost_usd: Option<f64>,
    /// Display indent (only non-zero in [`DashboardSort::Tree`]).
    pub depth: usize,
    pub watch: bool,
    pub pinned: bool,
}

impl DashboardRow {
    fn from(state: &AgentState, depth: usize) -> Self {
        Self {
            pane_id: state.pane_id.clone(),
            title: state.title.clone(),
            phase: state.phase,
            last_activity_unix_ms: state.last_activity_unix_ms,
            cost_usd: state.cost.as_ref().map(|c| c.usd),
            depth,
            watch: state.watch,
            pinned: state.pinned,
        }
    }
}

/// Public projection over an [`AgentStateStore`] for the dashboard.
pub struct DashboardView<'a> {
    store: &'a AgentStateStore,
}

impl<'a> DashboardView<'a> {
    pub fn new(store: &'a AgentStateStore) -> Self {
        Self { store }
    }

    pub fn rows(&self, sort: DashboardSort) -> Vec<DashboardRow> {
        match sort {
            DashboardSort::Tree => self.rows_tree(),
            other => self.rows_flat(other),
        }
    }

    fn rows_flat(&self, sort: DashboardSort) -> Vec<DashboardRow> {
        let mut rows: Vec<DashboardRow> = self
            .store
            .iter()
            .map(|s| DashboardRow::from(s, 0))
            .collect();
        match sort {
            DashboardSort::Phase => {
                rows.sort_by(|a, b| {
                    let pa = phase_order(a.phase);
                    let pb = phase_order(b.phase);
                    pa.cmp(&pb)
                        .then(a.last_activity_unix_ms.cmp(&b.last_activity_unix_ms))
                });
            }
            DashboardSort::LastActivity => {
                rows.sort_by(|a, b| b.last_activity_unix_ms.cmp(&a.last_activity_unix_ms));
            }
            DashboardSort::Cost => {
                rows.sort_by(|a, b| match (a.cost_usd, b.cost_usd) {
                    (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                });
            }
            DashboardSort::Tree => unreachable!("handled above"),
        }
        rows
    }

    fn rows_tree(&self) -> Vec<DashboardRow> {
        let lineage = Lineage::build(self.store);
        let mut out = Vec::new();
        lineage.walk_preorder(|state, depth| {
            out.push(DashboardRow::from(state, depth));
        });
        out
    }

    /// Pane ids matching a bulk-action predicate. Use this with the host
    /// integration to enact "Close all idle", "Close all done",
    /// "Restart all errored", etc.
    pub fn ids_matching(&self, predicate: impl Fn(&AgentState) -> bool) -> Vec<PaneId> {
        self.store
            .iter()
            .filter(|s| predicate(s) && !s.pinned)
            .map(|s| s.pane_id.clone())
            .collect()
    }
}

/// Lower number = higher priority in the Phase sort.
fn phase_order(p: AgentPhase) -> u8 {
    match p {
        AgentPhase::Awaiting => 0,
        AgentPhase::Tool => 1,
        AgentPhase::Thinking => 2,
        AgentPhase::Done => 3,
        AgentPhase::Error => 4,
        AgentPhase::Idle => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make(pane: &str, phase: AgentPhase, last: u64, cost: Option<f64>) -> AgentState {
        let mut s = AgentState::new(PaneId::new(pane), PathBuf::from("/"), pane);
        s.phase = phase;
        s.last_activity_unix_ms = last;
        if let Some(usd) = cost {
            s.cost = Some(crate::state::Cost {
                input_tokens: 0,
                output_tokens: 0,
                usd,
            });
        }
        s
    }

    #[test]
    fn phase_sort_puts_awaiting_first_idle_last() {
        let mut store = AgentStateStore::new();
        store.upsert(make("idle", AgentPhase::Idle, 1, None));
        store.upsert(make("awaiting", AgentPhase::Awaiting, 2, None));
        store.upsert(make("tool", AgentPhase::Tool, 3, None));
        let view = DashboardView::new(&store);
        let order: Vec<_> = view
            .rows(DashboardSort::Phase)
            .into_iter()
            .map(|r| r.pane_id.0)
            .collect();
        assert_eq!(order, vec!["awaiting", "tool", "idle"]);
    }

    #[test]
    fn cost_sort_descending_with_nulls_last() {
        let mut store = AgentStateStore::new();
        store.upsert(make("a", AgentPhase::Done, 1, Some(0.10)));
        store.upsert(make("b", AgentPhase::Done, 2, Some(1.50)));
        store.upsert(make("c", AgentPhase::Done, 3, None));
        let view = DashboardView::new(&store);
        let order: Vec<_> = view
            .rows(DashboardSort::Cost)
            .into_iter()
            .map(|r| r.pane_id.0)
            .collect();
        assert_eq!(order, vec!["b", "a", "c"]);
    }

    #[test]
    fn ids_matching_excludes_pinned_panes() {
        let mut store = AgentStateStore::new();
        let mut idle = make("idle1", AgentPhase::Idle, 1, None);
        let mut idle_pinned = make("idle_pin", AgentPhase::Idle, 2, None);
        idle_pinned.pinned = true;
        store.upsert(idle.clone());
        store.upsert(idle_pinned);
        idle.pane_id = PaneId::new("idle2");
        store.upsert(idle);

        let view = DashboardView::new(&store);
        let ids = view.ids_matching(|s| s.phase == AgentPhase::Idle);
        let names: Vec<_> = ids.into_iter().map(|i| i.0).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"idle1".to_string()));
        assert!(names.contains(&"idle2".to_string()));
        assert!(!names.contains(&"idle_pin".to_string()));
    }

    #[test]
    fn tree_sort_uses_pre_order_with_depth() {
        let mut store = AgentStateStore::new();
        store.upsert(make("p", AgentPhase::Idle, 1, None));
        let mut child = make("c", AgentPhase::Idle, 2, None);
        child.parent_pane_id = Some(PaneId::new("p"));
        store.upsert(child);
        let view = DashboardView::new(&store);
        let rows = view.rows(DashboardSort::Tree);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pane_id.0, "p");
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].pane_id.0, "c");
        assert_eq!(rows[1].depth, 1);
    }
}
