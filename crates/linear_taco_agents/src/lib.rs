//! Linear Taco — agent state model.
//!
//! The "agent" abstraction layered on top of `cli_agent_sessions`. A
//! [`PaneId`] identifies a terminal pane. Each pane that runs the
//! `claude` CLI has an [`AgentState`] tracking its phase (idle / thinking
//! / tool / awaiting / done / error), session id, todos, file changes,
//! cost, and lineage.
//!
//! Modules:
//!
//! - [`phase`] — [`AgentPhase`] derivation from [`claude_viewer`] events
//!   plus a PTY heartbeat.
//! - [`state`] — the per-pane [`AgentState`] struct + JSON persistence.
//! - [`dispatcher`] — typed actions (Spawn / Fork / SubAgent / Headless)
//!   for the integration layer to fulfill via PTYs.
//! - [`watch`] — write-gate predicate enforcing watch mode at the IPC
//!   boundary.
//! - [`lineage`] — parent ↔ child relationships and tree rollup.
//! - [`dashboard`] — sortable, filterable view over many panes for the
//!   AgentDashboard overlay.

#![deny(rust_2018_idioms)]

pub mod dashboard;
pub mod dispatcher;
pub mod lineage;
pub mod phase;
pub mod state;
pub mod watch;

pub use dashboard::{DashboardSort, DashboardView};
pub use dispatcher::{DispatchAction, DispatchKind};
pub use lineage::Lineage;
pub use phase::{AgentPhase, PhaseDeriver};
pub use state::{AgentState, AgentStateStore, Cost, FileChange, PaneId};
pub use watch::{WatchGate, WriteGateDecision};
