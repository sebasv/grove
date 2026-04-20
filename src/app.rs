use std::collections::HashMap;

use anyhow::Result;

use crate::config::Config;
use crate::git;
use crate::model::Repo;
use crate::state::{ActiveWorktreeId, PersistedState, PersistedUi};

pub struct AppState {
    pub repos: Vec<Repo>,
    pub ui: UiState,
    pub should_quit: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UiState {
    pub expanded: HashMap<String, bool>,
    pub cursor: Option<SidebarCursor>,
    pub active_worktree: Option<(usize, usize)>,
    pub modal: Option<Modal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarCursor {
    Repo(usize),
    Worktree { repo: usize, worktree: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    Help,
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy)]
pub enum AppMessage {
    MoveCursor(Direction),
    ExpandOrDescend,
    CollapseOrAscend,
    Activate,
    ToggleHelp,
    CloseModal,
    Quit,
    NoOp,
}

impl UiState {
    pub fn is_expanded(&self, repo_name: &str) -> bool {
        // Repos are expanded by default; state only tracks explicit collapses.
        self.expanded.get(repo_name).copied().unwrap_or(true)
    }
}

impl AppState {
    pub fn load(config: &Config) -> Result<Self> {
        let mut repos = Vec::with_capacity(config.repos.len());
        for repo_cfg in &config.repos {
            let worktrees = match git::list_worktrees(&repo_cfg.path) {
                Ok(list) => list,
                Err(err) => {
                    eprintln!("warning: skipping repo {}: {err:#}", repo_cfg.name);
                    continue;
                }
            };
            let base_branch = repo_cfg
                .base_branch
                .clone()
                .unwrap_or_else(|| config.general.default_base_branch.clone());
            repos.push(Repo {
                name: repo_cfg.name.clone(),
                root_path: repo_cfg.path.clone(),
                base_branch,
                worktrees,
            });
        }
        let cursor = if repos.is_empty() {
            None
        } else {
            Some(SidebarCursor::Repo(0))
        };
        Ok(Self {
            repos,
            ui: UiState {
                cursor,
                ..UiState::default()
            },
            should_quit: false,
        })
    }

    pub fn update(&mut self, msg: AppMessage) {
        match msg {
            AppMessage::MoveCursor(dir) => self.move_cursor(dir),
            AppMessage::ExpandOrDescend => self.expand_or_descend(),
            AppMessage::CollapseOrAscend => self.collapse_or_ascend(),
            AppMessage::Activate => self.activate(),
            AppMessage::ToggleHelp => self.toggle_help(),
            AppMessage::CloseModal => self.ui.modal = None,
            AppMessage::Quit => self.should_quit = true,
            AppMessage::NoOp => {}
        }
    }

    fn toggle_help(&mut self) {
        self.ui.modal = match self.ui.modal {
            Some(Modal::Help) => None,
            None => Some(Modal::Help),
        };
    }

    pub fn apply_persisted(&mut self, persisted: PersistedState) {
        self.ui.expanded = persisted.ui.expanded;
        self.ui.active_worktree = persisted.ui.active_worktree.and_then(|active| {
            self.repos.iter().enumerate().find_map(|(i, repo)| {
                if repo.name != active.repo {
                    return None;
                }
                repo.worktrees
                    .iter()
                    .position(|wt| wt.branch == active.branch)
                    .map(|j| (i, j))
            })
        });
        // Position cursor on the active worktree if present; otherwise leave default.
        if let Some((r, w)) = self.ui.active_worktree {
            self.ui.cursor = Some(SidebarCursor::Worktree {
                repo: r,
                worktree: w,
            });
        }
    }

    pub fn to_persisted(&self) -> PersistedState {
        let active_worktree = self.ui.active_worktree.and_then(|(r, w)| {
            let repo = self.repos.get(r)?;
            let wt = repo.worktrees.get(w)?;
            Some(ActiveWorktreeId {
                repo: repo.name.clone(),
                branch: wt.branch.clone(),
            })
        });
        PersistedState {
            schema_version: crate::state::current_schema_version(),
            ui: PersistedUi {
                active_worktree,
                expanded: self.ui.expanded.clone(),
            },
        }
    }

    fn visible_items(&self) -> Vec<SidebarCursor> {
        let mut items = Vec::new();
        for (i, repo) in self.repos.iter().enumerate() {
            items.push(SidebarCursor::Repo(i));
            if self.ui.is_expanded(&repo.name) {
                for j in 0..repo.worktrees.len() {
                    items.push(SidebarCursor::Worktree {
                        repo: i,
                        worktree: j,
                    });
                }
            }
        }
        items
    }

    fn move_cursor(&mut self, dir: Direction) {
        let visible = self.visible_items();
        if visible.is_empty() {
            return;
        }
        let current = match self.ui.cursor {
            Some(c) => c,
            None => {
                self.ui.cursor = Some(visible[0]);
                return;
            }
        };
        let pos = visible
            .iter()
            .position(|c| *c == current)
            .unwrap_or(0);
        let new_pos = match dir {
            Direction::Up => pos.saturating_sub(1),
            Direction::Down => (pos + 1).min(visible.len() - 1),
        };
        self.ui.cursor = Some(visible[new_pos]);
    }

    fn expand_or_descend(&mut self) {
        let Some(SidebarCursor::Repo(idx)) = self.ui.cursor else {
            return;
        };
        let name = self.repos[idx].name.clone();
        if self.ui.is_expanded(&name) {
            if !self.repos[idx].worktrees.is_empty() {
                self.ui.cursor = Some(SidebarCursor::Worktree {
                    repo: idx,
                    worktree: 0,
                });
            }
        } else {
            self.ui.expanded.insert(name, true);
        }
    }

    fn collapse_or_ascend(&mut self) {
        let Some(cursor) = self.ui.cursor else {
            return;
        };
        match cursor {
            SidebarCursor::Worktree { repo, .. } => {
                self.ui.cursor = Some(SidebarCursor::Repo(repo));
            }
            SidebarCursor::Repo(idx) => {
                let name = self.repos[idx].name.clone();
                if self.ui.is_expanded(&name) {
                    self.ui.expanded.insert(name, false);
                }
            }
        }
    }

    fn activate(&mut self) {
        let Some(cursor) = self.ui.cursor else {
            return;
        };
        match cursor {
            SidebarCursor::Worktree { repo, worktree } => {
                self.ui.active_worktree = Some((repo, worktree));
            }
            SidebarCursor::Repo(idx) => {
                let name = self.repos[idx].name.clone();
                let expanded = self.ui.is_expanded(&name);
                self.ui.expanded.insert(name, !expanded);
            }
        }
    }
}

#[cfg(test)]
impl AppState {
    pub fn fixture() -> Self {
        use std::path::PathBuf;

        use crate::model::Worktree;

        let mut state = Self {
            repos: vec![
                Repo {
                    name: "grove".to_string(),
                    root_path: PathBuf::from("/Users/sebas/dev/grove"),
                    base_branch: "main".to_string(),
                    worktrees: vec![
                        Worktree {
                            branch: "main".to_string(),
                            path: PathBuf::from("/Users/sebas/dev/grove"),
                            is_primary: true,
                        },
                        Worktree {
                            branch: "feat/sidebar".to_string(),
                            path: PathBuf::from("/Users/sebas/dev/grove-feat-sidebar"),
                            is_primary: false,
                        },
                        Worktree {
                            branch: "fix/deps".to_string(),
                            path: PathBuf::from("/Users/sebas/dev/grove-fix-deps"),
                            is_primary: false,
                        },
                    ],
                },
                Repo {
                    name: "dotfiles".to_string(),
                    root_path: PathBuf::from("/Users/sebas/dotfiles"),
                    base_branch: "main".to_string(),
                    worktrees: vec![
                        Worktree {
                            branch: "main".to_string(),
                            path: PathBuf::from("/Users/sebas/dotfiles"),
                            is_primary: true,
                        },
                        Worktree {
                            branch: "wip/zsh".to_string(),
                            path: PathBuf::from("/Users/sebas/dotfiles-wip-zsh"),
                            is_primary: false,
                        },
                    ],
                },
            ],
            ui: UiState::default(),
            should_quit: false,
        };
        state.ui.cursor = Some(SidebarCursor::Repo(0));
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j_moves_cursor_through_visible_tree() {
        let mut app = AppState::fixture();
        // Cursor starts on grove repo.
        assert_eq!(app.ui.cursor, Some(SidebarCursor::Repo(0)));
        app.update(AppMessage::MoveCursor(Direction::Down));
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 0,
                worktree: 0
            })
        );
        app.update(AppMessage::MoveCursor(Direction::Down));
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 0,
                worktree: 1
            })
        );
    }

    #[test]
    fn j_skips_collapsed_repo_children() {
        let mut app = AppState::fixture();
        app.ui.expanded.insert("grove".to_string(), false);
        app.update(AppMessage::MoveCursor(Direction::Down));
        // Should skip grove's worktrees and land on dotfiles.
        assert_eq!(app.ui.cursor, Some(SidebarCursor::Repo(1)));
    }

    #[test]
    fn h_on_worktree_ascends_to_parent() {
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        app.update(AppMessage::CollapseOrAscend);
        assert_eq!(app.ui.cursor, Some(SidebarCursor::Repo(0)));
    }

    #[test]
    fn h_on_expanded_repo_collapses() {
        let mut app = AppState::fixture();
        app.update(AppMessage::CollapseOrAscend);
        assert!(!app.ui.is_expanded("grove"));
    }

    #[test]
    fn l_on_collapsed_repo_expands() {
        let mut app = AppState::fixture();
        app.ui.expanded.insert("grove".to_string(), false);
        app.update(AppMessage::ExpandOrDescend);
        assert!(app.ui.is_expanded("grove"));
    }

    #[test]
    fn l_on_expanded_repo_descends_to_first_worktree() {
        let mut app = AppState::fixture();
        app.update(AppMessage::ExpandOrDescend);
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 0,
                worktree: 0
            })
        );
    }

    #[test]
    fn enter_on_worktree_sets_active() {
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 2,
        });
        app.update(AppMessage::Activate);
        assert_eq!(app.ui.active_worktree, Some((0, 2)));
    }

    #[test]
    fn enter_on_repo_toggles_expansion() {
        let mut app = AppState::fixture();
        app.update(AppMessage::Activate);
        assert!(!app.ui.is_expanded("grove"));
        app.update(AppMessage::Activate);
        assert!(app.ui.is_expanded("grove"));
    }

    #[test]
    fn persisted_round_trip_restores_expanded_and_active() {
        let mut app = AppState::fixture();
        app.ui.expanded.insert("dotfiles".to_string(), false);
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 2,
        });
        app.update(AppMessage::Activate);

        let persisted = app.to_persisted();

        let mut restored = AppState::fixture();
        restored.apply_persisted(persisted);
        assert!(!restored.ui.is_expanded("dotfiles"));
        assert_eq!(restored.ui.active_worktree, Some((0, 2)));
        assert_eq!(
            restored.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 0,
                worktree: 2
            })
        );
    }

    #[test]
    fn persisted_active_is_dropped_when_branch_no_longer_exists() {
        use crate::state::{ActiveWorktreeId, PersistedState, PersistedUi};
        let mut app = AppState::fixture();
        let persisted = PersistedState {
            schema_version: crate::state::current_schema_version(),
            ui: PersistedUi {
                active_worktree: Some(ActiveWorktreeId {
                    repo: "grove".to_string(),
                    branch: "gone-branch".to_string(),
                }),
                expanded: HashMap::new(),
            },
        };
        app.apply_persisted(persisted);
        assert_eq!(app.ui.active_worktree, None);
    }

    #[test]
    fn cursor_clamps_at_bounds() {
        let mut app = AppState::fixture();
        // Go up from top — should stay on top.
        app.update(AppMessage::MoveCursor(Direction::Up));
        assert_eq!(app.ui.cursor, Some(SidebarCursor::Repo(0)));
        // Go to bottom.
        for _ in 0..20 {
            app.update(AppMessage::MoveCursor(Direction::Down));
        }
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 1,
                worktree: 1
            })
        );
    }
}
