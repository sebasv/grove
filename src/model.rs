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
    pub branch: String,
    pub path: PathBuf,
    pub is_primary: bool,
    pub status: Option<WorktreeStatus>,
    pub pr: Option<PrStatus>,
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
