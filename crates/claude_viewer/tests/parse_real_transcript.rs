//! Integration test that parses a real transcript on the developer's
//! machine if one exists. Skipped in CI.

use claude_viewer::{TranscriptEvent, cards_from_events, read_all_lines};
use std::path::PathBuf;

#[tokio::test]
async fn parses_real_local_transcript_if_available() {
    let Some(path) = first_local_transcript() else {
        eprintln!("no local transcript found; skipping");
        return;
    };
    eprintln!("parsing {}", path.display());

    let lines = read_all_lines(&path).await.expect("read_all_lines");
    assert!(!lines.is_empty(), "transcript should have events");

    let mut decoded = 0usize;
    let mut unknown = 0usize;
    for line in &lines {
        match line.event {
            TranscriptEvent::Unknown => unknown += 1,
            _ => decoded += 1,
        }
    }
    eprintln!(
        "decoded {decoded} lines, {unknown} unknown ({:.1}% recognized)",
        100.0 * decoded as f64 / lines.len() as f64
    );

    let events: Vec<&TranscriptEvent> = lines.iter().map(|l| &l.event).collect();
    let cards = cards_from_events(events.into_iter());
    eprintln!("produced {} cards", cards.len());

    // We don't assert on counts — different sessions look very different —
    // but we verify nothing panicked.
    let _ = cards;
}

fn first_local_transcript() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let projects = PathBuf::from(home).join(".claude").join("projects");
    let dirs = std::fs::read_dir(&projects).ok()?;
    for project_dir in dirs.flatten() {
        let Ok(mut entries) = std::fs::read_dir(project_dir.path()) else {
            continue;
        };
        while let Some(Ok(file)) = entries.next() {
            if file.path().extension().and_then(|s| s.to_str()) == Some("jsonl") {
                return Some(file.path());
            }
        }
    }
    None
}
