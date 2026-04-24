use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{Event as CEvent, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;
use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::activity::ActivityId;
use crate::model::WorktreeStatus;

pub type RepoId = usize;
pub type WorktreeId = (usize, usize);

pub enum Event {
    Input(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    RepoDirty(RepoId),
    StatusReady(WorktreeId, WorktreeStatus),
    DiffReady(WorktreeId, Vec<crate::git::DiffFile>),
    PrStatusReady(WorktreeId, crate::model::PrStatus),
    /// A background fetch finished.  `ok` is true on success; on
    /// failure the activity is still deregistered but `last_fetched_at`
    /// stays pinned to its previous value so the scheduler retries at
    /// the next cadence tick.
    FetchFinished(RepoId, bool),
    /// Generic activity-deregistration signal.  Any task that obtained
    /// an `ActivityId` via `ActivityState::start` must emit this when
    /// it finishes so the sidebar footer stops showing it.
    ActivityFinished(ActivityId),
    /// A `grove <dir>` scan finished; `paths` is the sorted list of
    /// repos discovered under the scan root.
    ScanCompleted(Vec<PathBuf>),
    /// A terminal's reader thread advanced its vt100 parser state; trigger a
    /// repaint.  The active-worktree's parser is the one we render, so we
    /// don't need to know which terminal emitted the event.
    TerminalOutput,
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
            match evt {
                CEvent::Key(key) if tx.send(Event::Input(key)).is_err() => {
                    break;
                }
                CEvent::Mouse(mouse) if tx.send(Event::Mouse(mouse)).is_err() => {
                    break;
                }
                CEvent::Paste(text) if tx.send(Event::Paste(text.clone())).is_err() => {
                    break;
                }
                _ => {}
            }
        }
    });
}

/// Spawn a blocking task that runs `git::compute_status` for one worktree and
/// forwards the result.  If the query errors (permissions, disappeared path),
/// the event is silently dropped — the UI keeps the last-known status.
///
/// If `activity_id` is given, an `ActivityFinished` event fires when the task
/// returns so the sidebar footer can deregister it.
pub fn spawn_status_refresh(
    id: WorktreeId,
    path: PathBuf,
    tx: EventSender,
    activity_id: Option<ActivityId>,
) {
    tokio::task::spawn_blocking(move || {
        if let Ok(status) = crate::git::compute_status(&path) {
            let _ = tx.send(Event::StatusReady(id, status));
        }
        if let Some(a) = activity_id {
            let _ = tx.send(Event::ActivityFinished(a));
        }
    });
}

pub fn spawn_diff_refresh(
    id: WorktreeId,
    path: PathBuf,
    mode: crate::app::DiffMode,
    base_branch: String,
    tx: EventSender,
    activity_id: Option<ActivityId>,
) {
    tokio::task::spawn_blocking(move || {
        let files = match mode {
            crate::app::DiffMode::Local => crate::git::compute_local_diff(&path),
            crate::app::DiffMode::Branch => crate::git::compute_branch_diff(&path, &base_branch),
        }
        .unwrap_or_default();
        let _ = tx.send(Event::DiffReady(id, files));
        if let Some(a) = activity_id {
            let _ = tx.send(Event::ActivityFinished(a));
        }
    });
}

/// Spawn a blocking task that runs `git fetch --all --prune` for one repo
/// and forwards the outcome.  The caller is expected to have already
/// registered an `Activity` for this fetch (so the footer shows it); the
/// same `activity_id` is deregistered when the task finishes.
pub fn spawn_fetch(repo_idx: RepoId, repo_root: PathBuf, tx: EventSender, activity_id: ActivityId) {
    tokio::task::spawn_blocking(move || {
        let ok = crate::git::fetch_remote(&repo_root).is_ok();
        let _ = tx.send(Event::FetchFinished(repo_idx, ok));
        let _ = tx.send(Event::ActivityFinished(activity_id));
    });
}

/// Spawn a blocking task that walks `root` up to `depth` levels looking
/// for git repositories, then sends the sorted result back.
pub fn spawn_scan(root: PathBuf, depth: u8, tx: EventSender) {
    tokio::task::spawn_blocking(move || {
        let paths = crate::git::discover_repos(&root, depth);
        let _ = tx.send(Event::ScanCompleted(paths));
    });
}

/// Start watching `<repo_root>/.git/` (recursively, to catch linked-worktree
/// index changes in `.git/worktrees/<name>/`) and emit `Event::RepoDirty(repo_idx)`
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
    let mut debouncer =
        new_debouncer(Duration::from_millis(150), handler).context("creating fs debouncer")?;
    debouncer
        .watcher()
        .watch(&dot_git, RecursiveMode::Recursive)
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
