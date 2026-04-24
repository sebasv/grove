//! Background-activity tracking.
//!
//! Any task that runs off the main thread and can change `AppState`
//! registers itself here before it starts and deregisters when it
//! finishes.  The UI reads `ActivityState::in_flight` to render the
//! sidebar footer so the user always knows why a badge or a list is
//! about to change.
//!
//! This also underwrites the non-blocking-UX contract: grove does not
//! synchronously fetch or scan on a keypress.  Work happens in tokio
//! tasks, progress shows up in the footer, and completion events flow
//! back through `async_evt::Event`.
//!
//! Some variants / accessors here are consumed by downstream v1.3 PRs
//! (auth indicator, new-worktree modal, scan).  Tolerate the dead-code
//! surface until those PRs land rather than shipping a half-finished
//! API.

#![allow(dead_code)]

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActivityId(u64);

impl ActivityId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Fetch,
    StatusRefresh,
    DiffRefresh,
    PrPoll,
}

impl ActivityKind {
    fn short(self) -> &'static str {
        match self {
            ActivityKind::Fetch => "fetching",
            ActivityKind::StatusRefresh => "status",
            ActivityKind::DiffRefresh => "diff",
            ActivityKind::PrPoll => "PR status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityScope {
    Repo(usize),
    Global,
}

#[derive(Debug, Clone)]
pub struct Activity {
    pub id: ActivityId,
    pub kind: ActivityKind,
    pub scope: ActivityScope,
    pub started_at: Instant,
    pub label: String,
}

#[derive(Debug, Default)]
pub struct ActivityState {
    next_id: u64,
    in_flight: Vec<Activity>,
    /// Timestamp of the last successful fetch per repo (keyed by repo
    /// index).  Used by the scheduler to decide when the next tick is
    /// due.  `None` means "never fetched this session."
    pub last_fetched_at: Vec<Option<Instant>>,
}

impl ActivityState {
    pub fn start(
        &mut self,
        kind: ActivityKind,
        scope: ActivityScope,
        label: impl Into<String>,
    ) -> ActivityId {
        let id = ActivityId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        self.in_flight.push(Activity {
            id,
            kind,
            scope,
            started_at: Instant::now(),
            label: label.into(),
        });
        id
    }

    pub fn finish(&mut self, id: ActivityId) {
        self.in_flight.retain(|a| a.id != id);
    }

    pub fn in_flight(&self) -> &[Activity] {
        &self.in_flight
    }

    pub fn is_empty(&self) -> bool {
        self.in_flight.is_empty()
    }

    pub fn fetch_in_flight_for(&self, repo_idx: usize) -> bool {
        self.in_flight
            .iter()
            .any(|a| a.kind == ActivityKind::Fetch && a.scope == ActivityScope::Repo(repo_idx))
    }

    /// One-line summary suitable for the sidebar footer.  Returns an
    /// empty string when idle.
    pub fn summary(&self) -> String {
        match self.in_flight.len() {
            0 => String::new(),
            1 => {
                let a = &self.in_flight[0];
                format!("⟳ {}", a.label)
            }
            n => format!("⟳ {n} tasks: {}", distinct_kinds(&self.in_flight)),
        }
    }

    pub fn resize_repos(&mut self, n: usize) {
        self.last_fetched_at.resize(n, None);
    }

    /// Repos whose last fetch is older than `cadence` (or never), in
    /// order.  Excludes repos that already have a fetch in flight.
    pub fn due_for_fetch(&self, cadence: Duration) -> Vec<usize> {
        let now = Instant::now();
        self.last_fetched_at
            .iter()
            .enumerate()
            .filter_map(|(idx, last)| {
                if self.fetch_in_flight_for(idx) {
                    return None;
                }
                match last {
                    None => Some(idx),
                    Some(t) if now.saturating_duration_since(*t) >= cadence => Some(idx),
                    _ => None,
                }
            })
            .collect()
    }
}

fn distinct_kinds(acts: &[Activity]) -> String {
    let mut seen = Vec::new();
    for a in acts {
        let s = a.kind.short();
        if !seen.contains(&s) {
            seen.push(s);
        }
    }
    seen.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_issues_unique_ids_and_tracks_in_flight() {
        let mut s = ActivityState::default();
        let a = s.start(ActivityKind::Fetch, ActivityScope::Repo(0), "fetch grove");
        let b = s.start(ActivityKind::PrPoll, ActivityScope::Global, "PR poll");
        assert_ne!(a, b);
        assert_eq!(s.in_flight().len(), 2);
    }

    #[test]
    fn finish_removes_matching_id_only() {
        let mut s = ActivityState::default();
        let a = s.start(ActivityKind::Fetch, ActivityScope::Repo(0), "a");
        let _b = s.start(ActivityKind::Fetch, ActivityScope::Repo(1), "b");
        s.finish(a);
        assert_eq!(s.in_flight().len(), 1);
        assert!(s.in_flight().iter().all(|act| act.id != a));
    }

    #[test]
    fn finish_missing_id_is_noop() {
        let mut s = ActivityState::default();
        let _a = s.start(ActivityKind::Fetch, ActivityScope::Repo(0), "a");
        s.finish(ActivityId(999));
        assert_eq!(s.in_flight().len(), 1);
    }

    #[test]
    fn summary_empty_when_idle() {
        let s = ActivityState::default();
        assert_eq!(s.summary(), "");
    }

    #[test]
    fn summary_one_task_uses_its_label() {
        let mut s = ActivityState::default();
        s.start(
            ActivityKind::Fetch,
            ActivityScope::Repo(0),
            "fetching grove",
        );
        assert_eq!(s.summary(), "⟳ fetching grove");
    }

    #[test]
    fn summary_many_tasks_groups_by_kind() {
        let mut s = ActivityState::default();
        s.start(
            ActivityKind::Fetch,
            ActivityScope::Repo(0),
            "fetching grove",
        );
        s.start(
            ActivityKind::Fetch,
            ActivityScope::Repo(1),
            "fetching dotfiles",
        );
        s.start(ActivityKind::PrPoll, ActivityScope::Global, "PR poll");
        let out = s.summary();
        assert!(out.starts_with("⟳ 3 tasks:"));
        assert!(out.contains("fetching"));
        assert!(out.contains("PR status"));
    }

    #[test]
    fn fetch_in_flight_for_filters_by_scope_and_kind() {
        let mut s = ActivityState::default();
        s.start(ActivityKind::Fetch, ActivityScope::Repo(0), "a");
        s.start(ActivityKind::PrPoll, ActivityScope::Repo(1), "b");
        assert!(s.fetch_in_flight_for(0));
        assert!(!s.fetch_in_flight_for(1)); // PrPoll, not Fetch
        assert!(!s.fetch_in_flight_for(2)); // nothing
    }

    #[test]
    fn due_for_fetch_skips_in_flight_repos() {
        let mut s = ActivityState::default();
        s.resize_repos(3);
        s.start(ActivityKind::Fetch, ActivityScope::Repo(0), "a");
        // repo 0 is in flight; repos 1 and 2 have None last-fetched.
        let due = s.due_for_fetch(Duration::from_secs(300));
        assert_eq!(due, vec![1, 2]);
    }

    #[test]
    fn due_for_fetch_respects_cadence() {
        let mut s = ActivityState::default();
        s.resize_repos(2);
        s.last_fetched_at[0] = Some(Instant::now());
        // repo 0 was just fetched; repo 1 never. With a 300s cadence,
        // only repo 1 is due.
        let due = s.due_for_fetch(Duration::from_secs(300));
        assert_eq!(due, vec![1]);
    }
}
