use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as CEvent, EventStream, KeyEvent};
use futures::StreamExt;
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::model::WorktreeStatus;

pub type RepoId = usize;
pub type WorktreeId = (usize, usize);

pub enum Event {
    Input(KeyEvent),
    RepoDirty(RepoId),
    StatusReady(WorktreeId, WorktreeStatus),
}

/// Opaque handle that keeps a repo's FS watcher alive for the caller's
/// lifetime.  Dropping this stops emitting `RepoDirty` events.
pub struct RepoWatcher {
    _debouncer: Debouncer<RecommendedWatcher>,
}

pub type EventSender = UnboundedSender<Event>;
pub type EventReceiver = UnboundedReceiver<Event>;

pub fn channel() -> (EventSender, EventReceiver) {
    unbounded_channel()
}

/// Spawn a task that forwards terminal key events into the event channel.
///
/// Any other crossterm event (resize, focus) is dropped for now; resize handling
/// arrives with v1.x polish.
pub fn spawn_terminal_reader(tx: EventSender) {
    tokio::spawn(async move {
        let mut stream = EventStream::new();
        while let Some(evt) = stream.next().await {
            let Ok(evt) = evt else { continue };
            if let CEvent::Key(key) = evt {
                if tx.send(Event::Input(key)).is_err() {
                    break;
                }
            }
        }
    });
}

/// Spawn a blocking task that runs `git::compute_status` for one worktree and
/// forwards the result.  If the query errors (permissions, disappeared path),
/// the event is silently dropped — the UI keeps the last-known status.
pub fn spawn_status_refresh(id: WorktreeId, path: PathBuf, tx: EventSender) {
    tokio::task::spawn_blocking(move || {
        if let Ok(status) = crate::git::compute_status(&path) {
            let _ = tx.send(Event::StatusReady(id, status));
        }
    });
}

/// Start watching `<repo_root>/.git/` and emit `Event::RepoDirty(repo_idx)`
/// when it changes (debounced at 150 ms).  Returns an opaque handle that must
/// be kept alive for the duration of the watch — dropping it stops events.
pub fn spawn_repo_watcher(
    repo_idx: RepoId,
    repo_root: PathBuf,
    tx: EventSender,
) -> Result<RepoWatcher> {
    let dot_git = repo_root.join(".git");
    let (fs_tx, fs_rx) = std::sync::mpsc::channel::<DebounceEventResult>();
    let handler = move |res: DebounceEventResult| {
        let _ = fs_tx.send(res);
    };
    let mut debouncer = new_debouncer(Duration::from_millis(150), handler)
        .context("creating fs debouncer")?;
    debouncer
        .watcher()
        .watch(&dot_git, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", dot_git.display()))?;

    // Bridge std::mpsc (blocking) to the async tokio channel via a background
    // thread.  Cheaper than spawn_blocking because the thread is permanent.
    std::thread::spawn(move || {
        while let Ok(result) = fs_rx.recv() {
            if result.is_err() {
                continue;
            }
            if tx.send(Event::RepoDirty(repo_idx)).is_err() {
                break;
            }
        }
    });

    Ok(RepoWatcher {
        _debouncer: debouncer,
    })
}
