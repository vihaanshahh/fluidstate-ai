// Tray UI lands when integrated into the workspace chrome. Until
// then we suppress dead-code warnings on the runtime data types.
#![allow(dead_code)]

//! Headless dispatch tray — runs `claude -p "<prompt>"` jobs without
//! creating a PTY and shows the running list in a small top-right tray.
//!
//! Data layer: a singleton-ish [`HeadlessTray`] holds all in-flight
//! and recently-completed runs, indexed by id. Each [`HeadlessRun`]
//! exposes its prompt, status, and accumulated assistant text.
//!
//! The view layer (a future `app/src/linear_taco/headless_tray_view.rs`)
//! reads this struct and renders rows. For now the runtime + tests are
//! the deliverable — Phase F's UI mount pairs with the eventual
//! workspace shell update.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use claude_viewer::{Card, TranscriptEvent, cards_from_events};
use parking_lot::RwLock;
use tokio::sync::mpsc;

use crate::linear_taco::claude_driver::{ClaudeDriver, DriverError, DriverEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessRunStatus {
    Running,
    Done,
    Errored,
}

#[derive(Debug, Clone)]
pub struct HeadlessRun {
    pub id: u64,
    pub prompt: String,
    pub workspace: PathBuf,
    pub started_at_unix_ms: u64,
    pub status: HeadlessRunStatus,
    /// Last assistant text we observed — what the user sees in the
    /// tray's collapsed row preview.
    pub last_text: Option<String>,
    /// Transcript events accumulated for the "expand to view" panel.
    pub events: Vec<TranscriptEvent>,
}

impl HeadlessRun {
    pub fn cards(&self) -> Vec<Card> {
        cards_from_events(self.events.iter())
    }
}

#[derive(Default)]
pub struct HeadlessTray {
    inner: Arc<RwLock<Inner>>,
    next_id: AtomicU64,
}

#[derive(Default)]
struct Inner {
    runs: HashMap<u64, HeadlessRun>,
    /// Runs in display order — newest first.
    order: Vec<u64>,
    /// Cap on retained completed runs so the tray doesn't grow without
    /// bound; runs older than this fall off the bottom.
    max_retained: usize,
}

impl HeadlessTray {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                runs: HashMap::new(),
                order: Vec::new(),
                max_retained: 32,
            })),
            next_id: AtomicU64::new(1),
        }
    }

    /// Dispatch a headless run. Returns the run id; events stream into
    /// the tray's stored events list as they arrive.
    pub async fn dispatch(
        &self,
        prompt: String,
        workspace: PathBuf,
    ) -> Result<u64, DriverError> {
        let id = self.next_id.fetch_add(1, Ordering::AcqRel);
        let mut driver = ClaudeDriver::new(&workspace)?;

        {
            let mut inner = self.inner.write();
            let run = HeadlessRun {
                id,
                prompt: prompt.clone(),
                workspace: workspace.clone(),
                started_at_unix_ms: unix_ms_now(),
                status: HeadlessRunStatus::Running,
                last_text: None,
                events: Vec::new(),
            };
            inner.runs.insert(id, run);
            inner.order.insert(0, id);
            // Trim to max_retained by dropping oldest completed runs.
            if inner.order.len() > inner.max_retained {
                let trim_from = inner.max_retained;
                let to_remove: Vec<u64> = inner.order[trim_from..].to_vec();
                for rid in to_remove.iter() {
                    inner.runs.remove(rid);
                }
                inner.order.truncate(trim_from);
            }
        }

        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut rx: mpsc::UnboundedReceiver<DriverEvent> =
                match driver.submit_prompt(&prompt).await {
                    Ok(rx) => rx,
                    Err(err) => {
                        let mut g = inner.write();
                        if let Some(run) = g.runs.get_mut(&id) {
                            run.status = HeadlessRunStatus::Errored;
                            run.last_text = Some(format!("dispatch failed: {err}"));
                        }
                        return;
                    }
                };

            while let Some(ev) = rx.recv().await {
                let mut g = inner.write();
                let Some(run) = g.runs.get_mut(&id) else { return };
                match ev {
                    DriverEvent::SessionResolved(_) => {}
                    DriverEvent::Transcript(event) => {
                        if let TranscriptEvent::Assistant(a) = &event {
                            if let Some(message) = &a.message {
                                if let Some(text) = first_text_block(&message.content) {
                                    run.last_text = Some(text);
                                }
                            }
                        }
                        run.events.push(event);
                    }
                    DriverEvent::Done { exit_code } => {
                        run.status = if exit_code.unwrap_or(0) == 0 {
                            HeadlessRunStatus::Done
                        } else {
                            HeadlessRunStatus::Errored
                        };
                    }
                    DriverEvent::Error(msg) => {
                        run.last_text = Some(msg);
                    }
                }
            }
        });

        Ok(id)
    }

    pub fn snapshot(&self) -> Vec<HeadlessRun> {
        let g = self.inner.read();
        g.order
            .iter()
            .filter_map(|id| g.runs.get(id).cloned())
            .collect()
    }

    pub fn get(&self, id: u64) -> Option<HeadlessRun> {
        self.inner.read().runs.get(&id).cloned()
    }

    pub fn forget(&self, id: u64) {
        let mut g = self.inner.write();
        g.runs.remove(&id);
        g.order.retain(|x| *x != id);
    }
}

fn first_text_block(blocks: &[claude_viewer::events::ContentBlock]) -> Option<String> {
    for block in blocks {
        if let claude_viewer::events::ContentBlock::Text { text } = block {
            return Some(text.clone());
        }
    }
    None
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tray_snapshot() {
        let tray = HeadlessTray::new();
        assert!(tray.snapshot().is_empty());
    }

    #[test]
    fn forget_removes_only_target() {
        let tray = HeadlessTray::new();
        let mut g = tray.inner.write();
        g.runs.insert(
            1,
            HeadlessRun {
                id: 1,
                prompt: "a".into(),
                workspace: PathBuf::from("/"),
                started_at_unix_ms: 0,
                status: HeadlessRunStatus::Done,
                last_text: None,
                events: Vec::new(),
            },
        );
        g.runs.insert(
            2,
            HeadlessRun {
                id: 2,
                prompt: "b".into(),
                workspace: PathBuf::from("/"),
                started_at_unix_ms: 0,
                status: HeadlessRunStatus::Done,
                last_text: None,
                events: Vec::new(),
            },
        );
        g.order = vec![2, 1];
        drop(g);

        tray.forget(1);
        let snap = tray.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].id, 2);
    }
}
