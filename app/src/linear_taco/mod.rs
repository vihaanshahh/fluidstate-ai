//! Linear Taco integration glue inside the Warp `app/` crate.
//!
//! Each submodule is a thin shim that bridges between upstream Warp's
//! stable surfaces and the linear-taco-owned crates under `crates/`.
//! The goal is to keep the patches in upstream files (`view.rs`,
//! `themes/...`, `lib.rs`) as small and well-isolated as possible so
//! weekly rebases off `origin/master` stay easy.

pub mod agent_dashboard;
pub mod agent_intercept;
pub mod agent_panel;
pub mod agent_session_tracker;
pub mod claude_driver;
pub mod claudex_session;
pub mod code_viewer;
pub mod find_in_files;
pub mod headless_tray;
pub mod source_control;
pub mod status;
pub mod tasks_runner;
