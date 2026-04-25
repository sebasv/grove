use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub name: String,
    pub root_path: PathBuf,
    pub base_branch: String,
    pub worktrees: Vec<Worktree>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// What HEAD currently points at. This is the worktree's *current state*,
    /// not its identity — the worktree's identity is its `path`. Branches can
    /// change underneath (e.g. `git switch` inside a terminal); `path` cannot.
    pub head: HeadRef,
    pub path: PathBuf,
    pub is_primary: bool,
    pub status: Option<WorktreeStatus>,
    pub pr: Option<PrStatus>,
}

/// What a worktree's HEAD points at. Branch is the common case; Detached covers
/// rebase/bisect/explicit checkout-of-a-commit. Both are first-class — a
/// detached-HEAD linked worktree must still appear in the sidebar so the user
/// can navigate to it and resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadRef {
    Branch(String),
    /// 7-char abbreviated OID. Stored as a string so display is allocation-free.
    Detached(String),
}

impl HeadRef {
    /// Human-readable label for the sidebar, tab bar, and modals.
    pub fn label(&self) -> String {
        match self {
            HeadRef::Branch(name) => name.clone(),
            HeadRef::Detached(oid) => format!("(detached) {oid}"),
        }
    }

    /// The branch name if HEAD is on a branch. Returns `None` for detached
    /// HEAD; callers that need a branch (PR polling, branch deletion) should
    /// skip detached-HEAD worktrees.
    pub fn branch_name(&self) -> Option<&str> {
        match self {
            HeadRef::Branch(name) => Some(name),
            HeadRef::Detached(_) => None,
        }
    }
}

impl Worktree {
    /// Sidebar/title-bar label. Convenience over `head.label()` so test
    /// fixtures and UI code can read the worktree directly.
    pub fn label(&self) -> String {
        self.head.label()
    }

    pub fn branch_name(&self) -> Option<&str> {
        self.head.branch_name()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    pub number: u32,
    pub state: PrState,
    pub checks: ChecksRollup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChecksRollup {
    #[default]
    None,
    Pending,
    Passing,
    Failing,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeStatus {
    pub staged: u32,
    pub modified: u32,
    pub deleted: u32,
    pub conflicts: u32,
    pub ahead: u32,
    pub behind: u32,
}

impl WorktreeStatus {
    pub fn is_clean(&self) -> bool {
        self.staged == 0
            && self.modified == 0
            && self.deleted == 0
            && self.conflicts == 0
            && self.ahead == 0
            && self.behind == 0
    }
}
