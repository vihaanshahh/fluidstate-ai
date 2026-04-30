//! Parent ↔ child pane relationships.
//!
//! Sub-agents spawned via the `Task` tool inherit a parent pane id. The
//! [`Lineage`] view computes the resulting forest from a flat
//! [`AgentStateStore`], used by the AgentDashboard's Tree sort and by the
//! workspace layout to render connecting hairlines between parent and
//! child panes.

use std::collections::{HashMap, HashSet};

use crate::state::{AgentState, AgentStateStore, PaneId};

/// Tree-shaped projection of `AgentStateStore`. Holds borrows back into
/// the store for cheap dashboard rendering.
#[derive(Debug, Clone)]
pub struct Lineage<'a> {
    pub roots: Vec<PaneId>,
    pub children: HashMap<PaneId, Vec<PaneId>>,
    pub by_id: HashMap<PaneId, &'a AgentState>,
}

impl<'a> Lineage<'a> {
    pub fn build(store: &'a AgentStateStore) -> Self {
        let by_id: HashMap<PaneId, &AgentState> =
            store.iter().map(|s| (s.pane_id.clone(), s)).collect();

        let mut children: HashMap<PaneId, Vec<PaneId>> = HashMap::new();
        let mut roots: Vec<PaneId> = Vec::new();
        let known_ids: HashSet<&PaneId> = by_id.keys().collect();

        for state in store.iter() {
            match &state.parent_pane_id {
                Some(parent) if known_ids.contains(parent) => {
                    children
                        .entry(parent.clone())
                        .or_default()
                        .push(state.pane_id.clone());
                }
                _ => roots.push(state.pane_id.clone()),
            }
        }

        // Stable order per parent: by start time, oldest first.
        for kids in children.values_mut() {
            kids.sort_by_key(|id| by_id[id].started_at_unix_ms);
        }
        roots.sort_by_key(|id| by_id[id].started_at_unix_ms);

        Self {
            roots,
            children,
            by_id,
        }
    }

    /// Pre-order walk (parent before children) used by the dashboard's
    /// Tree sort. Calls `visit` with `(state, depth)`.
    pub fn walk_preorder(&self, mut visit: impl FnMut(&AgentState, usize)) {
        let roots = self.roots.clone();
        for root in roots {
            self.walk_from(&root, 0, &mut visit);
        }
    }

    fn walk_from(&self, id: &PaneId, depth: usize, visit: &mut impl FnMut(&AgentState, usize)) {
        if let Some(state) = self.by_id.get(id) {
            visit(state, depth);
            if let Some(kids) = self.children.get(id) {
                for kid in kids {
                    self.walk_from(kid, depth + 1, visit);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn agent(pane: &str, parent: Option<&str>, started: u64) -> AgentState {
        let mut s = AgentState::new(PaneId::new(pane), PathBuf::from("/"), pane);
        s.parent_pane_id = parent.map(|p| PaneId::new(p));
        s.started_at_unix_ms = started;
        s
    }

    #[test]
    fn flat_panes_are_all_roots() {
        let mut store = AgentStateStore::new();
        store.upsert(agent("a", None, 1));
        store.upsert(agent("b", None, 2));
        let lineage = Lineage::build(&store);
        assert_eq!(lineage.roots.len(), 2);
        assert!(lineage.children.is_empty());
    }

    #[test]
    fn child_nests_under_parent() {
        let mut store = AgentStateStore::new();
        store.upsert(agent("p", None, 1));
        store.upsert(agent("c1", Some("p"), 2));
        store.upsert(agent("c2", Some("p"), 3));
        let lineage = Lineage::build(&store);
        assert_eq!(lineage.roots, vec![PaneId::new("p")]);
        assert_eq!(
            lineage.children[&PaneId::new("p")],
            vec![PaneId::new("c1"), PaneId::new("c2")]
        );
    }

    #[test]
    fn child_with_unknown_parent_is_treated_as_root() {
        let mut store = AgentStateStore::new();
        store.upsert(agent("orphan", Some("ghost"), 1));
        let lineage = Lineage::build(&store);
        assert_eq!(lineage.roots, vec![PaneId::new("orphan")]);
    }

    #[test]
    fn preorder_visits_parents_before_children() {
        let mut store = AgentStateStore::new();
        store.upsert(agent("p", None, 1));
        store.upsert(agent("c", Some("p"), 2));
        let lineage = Lineage::build(&store);
        let mut order: Vec<(String, usize)> = Vec::new();
        lineage.walk_preorder(|state, depth| order.push((state.pane_id.0.clone(), depth)));
        assert_eq!(order, vec![("p".to_string(), 0), ("c".to_string(), 1)]);
    }
}
