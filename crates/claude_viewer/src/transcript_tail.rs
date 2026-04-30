//! Tails a Claude Code transcript JSONL file and emits typed events.
//!
//! The transcript at `~/.claude/projects/<encoded-cwd>/<session>.jsonl` is
//! append-only — Claude Code writes one JSON object per line as the session
//! progresses. We watch the file with `notify-debouncer-full`, replay every
//! line on startup, then emit each new line as it arrives.

use std::{
    io::SeekFrom,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result};
use async_channel::{Receiver, Sender};
use notify_debouncer_full::{
    DebounceEventResult, Debouncer, FileIdMap, new_debouncer,
    notify::{RecommendedWatcher, RecursiveMode},
};
use serde_json::Value;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, BufReader},
    sync::Notify,
};

use crate::events::TranscriptEvent;

/// Default debounce interval for the underlying file watcher. Claude Code
/// streams at ~10–60 Hz during tool use; 50ms keeps us responsive without
/// re-reading the file on every byte.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(50);

/// One decoded line plus the original JSON. The viewer renders [`event`],
/// the cards module collapses tool_use ↔ tool_result, and any future
/// "view raw JSON" affordance reads [`raw`].
#[derive(Debug, Clone)]
pub struct TranscriptLine {
    pub raw: Value,
    pub event: TranscriptEvent,
    pub byte_offset: u64,
    pub line_number: u64,
}

/// Errors emitted alongside successful lines on the same channel so callers
/// can surface read failures without a separate stream.
#[derive(Debug, thiserror::Error)]
pub enum TailError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json parse error at line {line}: {source}")]
    Json {
        line: u64,
        #[source]
        source: serde_json::Error,
    },
}

pub type TailItem = std::result::Result<TranscriptLine, TailError>;

/// A live tail of a transcript file. Drop the handle to stop watching; the
/// background task exits when its receiver is dropped.
pub struct TranscriptTail {
    rx: Receiver<TailItem>,
    /// Owning the debouncer keeps the watcher thread alive.
    _debouncer: Debouncer<RecommendedWatcher, FileIdMap>,
    /// Used by tests to wait for the initial replay before asserting.
    initial_replay_done: Arc<Notify>,
}

impl TranscriptTail {
    /// Open `path` for tailing. The first replay (existing lines on disk)
    /// arrives on the receiver before any subsequent file-change events.
    /// Replay is performed on the current Tokio runtime.
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Self::open_with_debounce(path, DEFAULT_DEBOUNCE).await
    }

    pub async fn open_with_debounce(path: impl Into<PathBuf>, debounce: Duration) -> Result<Self> {
        let path = path.into();

        let (tx, rx) = async_channel::unbounded::<TailItem>();
        let initial_replay_done = Arc::new(Notify::new());

        let (notify_tx, notify_rx) = std::sync::mpsc::channel::<DebounceEventResult>();
        let mut debouncer = new_debouncer(debounce, None, move |res| {
            let _ = notify_tx.send(res);
        })
        .context("failed to create file debouncer")?;

        // Watch the parent directory non-recursively so we still see writes
        // when the transcript file is replaced (e.g. claude resumes).
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("transcript path has no parent: {}", path.display()))?;
        debouncer
            .watch(parent, RecursiveMode::NonRecursive)
            .with_context(|| format!("failed to watch {}", parent.display()))?;

        let path_for_task = path.clone();
        let replay_done = initial_replay_done.clone();
        let line_no = Arc::new(AtomicU64::new(0));
        let next_offset = Arc::new(AtomicU64::new(0));

        // Poll the std mpsc channel from a blocking task so we don't tie up
        // a tokio worker. Each event triggers a re-read from the last byte
        // offset on the same runtime via `tokio::spawn`.
        let tx_replay = tx.clone();
        let replay_path = path_for_task.clone();
        let replay_line_no = line_no.clone();
        let replay_offset = next_offset.clone();
        tokio::spawn(async move {
            // Initial replay.
            let _ = read_new_lines(
                &replay_path,
                &tx_replay,
                replay_offset.as_ref(),
                replay_line_no.as_ref(),
            )
            .await;
            replay_done.notify_waiters();

            // Convert std mpsc → tokio channel by polling on a blocking thread.
            // We re-read on *any* event in the watched parent directory and let
            // `read_new_lines` decide whether new bytes actually arrived. This
            // avoids brittle path-equality comparisons across canonicalization
            // differences (e.g. /tmp vs /private/tmp on macOS).
            let (forward_tx, mut forward_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
            std::thread::spawn(move || {
                while let Ok(_res) = notify_rx.recv() {
                    if forward_tx.send(()).is_err() {
                        break;
                    }
                }
            });

            while forward_rx.recv().await.is_some() {
                if read_new_lines(
                    &replay_path,
                    &tx_replay,
                    replay_offset.as_ref(),
                    replay_line_no.as_ref(),
                )
                .await
                .is_err()
                {
                    // I/O errors are already pushed to `tx_replay` as TailItem::Err.
                }
            }
        });

        Ok(Self {
            rx,
            _debouncer: debouncer,
            initial_replay_done,
        })
    }

    /// Returns the next item from the tail, or `None` if the tail has been
    /// shut down (sender dropped).
    pub async fn next(&self) -> Option<TailItem> {
        self.rx.recv().await.ok()
    }

    /// Borrow the underlying receiver — useful for `select!` in callers.
    pub fn receiver(&self) -> &Receiver<TailItem> {
        &self.rx
    }

    /// Wait for the initial on-disk replay to finish. Mostly useful in tests.
    pub async fn wait_for_replay(&self) {
        self.initial_replay_done.notified().await;
    }
}

/// Read everything from `*next_offset` onwards in `path`, parse each complete
/// line, push results onto `tx`, and update `next_offset` to one past the
/// last newline byte we consumed. Partial trailing lines are left for the
/// next read.
async fn read_new_lines(
    path: &Path,
    tx: &Sender<TailItem>,
    next_offset: &AtomicU64,
    line_no: &AtomicU64,
) -> std::io::Result<()> {
    let mut file = match File::open(path).await {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            let _ = tx.send(Err(TailError::from(io_clone(&err)))).await;
            return Err(err);
        }
    };

    let metadata = file.metadata().await?;
    let len = metadata.len();
    let from = next_offset.load(Ordering::Acquire).min(len);

    // If the file shrank (rotated/truncated) restart from zero so we don't
    // skip the new content.
    let from = if from > len { 0 } else { from };

    file.seek(SeekFrom::Start(from)).await?;

    let mut reader = BufReader::new(&mut file);
    let mut consumed: u64 = from;
    let mut buf = String::new();
    loop {
        buf.clear();
        let read = reader.read_line(&mut buf).await?;
        if read == 0 {
            break;
        }
        // Only emit complete lines (i.e. ones that end with \n).
        if !buf.ends_with('\n') {
            // Leave the partial line for the next read.
            break;
        }
        let line_bytes = read as u64;
        let line_start = consumed;
        consumed += line_bytes;

        let trimmed = buf.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }
        let n = line_no.fetch_add(1, Ordering::AcqRel);
        let item = parse_line(trimmed, line_start, n);
        if tx.send(item).await.is_err() {
            // Receiver dropped; stop tailing.
            return Ok(());
        }
    }

    next_offset.store(consumed, Ordering::Release);
    drop(reader);

    // Read past the EOF position so subsequent appends are picked up — we
    // don't actually keep the file handle open between calls; we reopen on
    // each event so log rotation works.
    let _ = file.read(&mut [0u8; 0]).await;
    Ok(())
}

fn parse_line(line: &str, byte_offset: u64, line_number: u64) -> TailItem {
    let raw: Value = serde_json::from_str(line).map_err(|e| TailError::Json {
        line: line_number,
        source: e,
    })?;
    let event: TranscriptEvent =
        serde_json::from_value(raw.clone()).map_err(|e| TailError::Json {
            line: line_number,
            source: e,
        })?;
    Ok(TranscriptLine {
        raw,
        event,
        byte_offset,
        line_number,
    })
}

fn io_clone(err: &std::io::Error) -> std::io::Error {
    std::io::Error::new(err.kind(), err.to_string())
}

/// Synchronously parse every line in `path`. Useful for unit tests and for
/// the dashboard's "load whole transcript on demand" affordance.
pub async fn read_all_lines(path: impl AsRef<Path>) -> Result<Vec<TranscriptLine>> {
    let path = path.as_ref();
    let mut file = File::open(path)
        .await
        .with_context(|| format!("open {}", path.display()))?;
    let mut text = String::new();
    file.read_to_string(&mut text).await?;
    let mut out = Vec::new();
    let mut offset: u64 = 0;
    for (i, line) in text.lines().enumerate() {
        let len = line.len() as u64 + 1; // +1 for the consumed newline
        if !line.is_empty()
            && let Ok(parsed) = parse_line(line, offset, i as u64) {
                out.push(parsed);
            }
        offset += len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;
    use tokio::time::{sleep, timeout};

    async fn write_line(path: &Path, line: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(file, "{line}").unwrap();
        file.sync_all().unwrap();
    }

    #[tokio::test]
    async fn replays_existing_lines_on_open() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl");
        write_line(&path, r#"{"type":"user","sessionId":"s","message":"hi"}"#).await;
        write_line(&path, r#"{"type":"some-other","x":1}"#).await;

        let tail = TranscriptTail::open(&path).await.unwrap();
        tail.wait_for_replay().await;

        let first = timeout(Duration::from_secs(1), tail.next())
            .await
            .unwrap()
            .unwrap();
        let first = first.unwrap();
        assert!(matches!(first.event, TranscriptEvent::User(_)));

        let second = timeout(Duration::from_secs(1), tail.next())
            .await
            .unwrap()
            .unwrap();
        let second = second.unwrap();
        assert!(matches!(second.event, TranscriptEvent::Unknown));
    }

    #[tokio::test]
    async fn picks_up_appended_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl");
        write_line(
            &path,
            r#"{"type":"user","sessionId":"s","message":"first"}"#,
        )
        .await;

        let tail = TranscriptTail::open_with_debounce(&path, Duration::from_millis(10))
            .await
            .unwrap();
        tail.wait_for_replay().await;

        // Drain the initial line.
        let _ = timeout(Duration::from_secs(1), tail.next()).await;

        // Give the watcher a moment to settle, then append.
        sleep(Duration::from_millis(50)).await;
        write_line(
            &path,
            r#"{"type":"user","sessionId":"s","message":"second"}"#,
        )
        .await;

        let appended = timeout(Duration::from_secs(2), tail.next())
            .await
            .unwrap()
            .unwrap();
        let appended = appended.unwrap();
        assert!(matches!(appended.event, TranscriptEvent::User(_)));
    }

    #[tokio::test]
    async fn read_all_lines_parses_recorded_transcript() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.jsonl");
        write_line(
            &path,
            r#"{"type":"permission-mode","permissionMode":"default","sessionId":"s"}"#,
        )
        .await;
        write_line(
            &path,
            r#"{"type":"user","sessionId":"s","message":"hello"}"#,
        )
        .await;
        write_line(
            &path,
            r#"{"type":"result","sessionId":"s","stopReason":"end_turn"}"#,
        )
        .await;

        let lines = read_all_lines(&path).await.unwrap();
        assert_eq!(lines.len(), 3);
        assert!(matches!(lines[0].event, TranscriptEvent::PermissionMode(_)));
        assert!(matches!(lines[1].event, TranscriptEvent::User(_)));
        assert!(matches!(lines[2].event, TranscriptEvent::Result(_)));
    }
}
