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
}
