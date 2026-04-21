use std::path::PathBuf;

use crossterm::event::{Event as CEvent, EventStream, KeyEvent};
use futures::StreamExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::model::WorktreeStatus;

pub type RepoId = usize;
pub type WorktreeId = (usize, usize);

pub enum Event {
    Input(KeyEvent),
    #[allow(dead_code)] // Wired up in the notify-watcher commit.
    RepoDirty(RepoId),
    StatusReady(WorktreeId, WorktreeStatus),
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
