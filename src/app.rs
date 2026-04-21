use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::config::{Config, RepoConfig};
use crate::git;
use crate::model::Repo;
use crate::state::{ActiveWorktreeId, PersistedState, PersistedUi};
use crate::ui::text_input::TextInput;

pub struct AppState {
    pub config: Config,
    pub config_path: PathBuf,
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

#[derive(Debug, Clone)]
pub enum Modal {
    Help,
    AddRepo(AddRepoModal),
    ConfirmRemoveRepo { repo_idx: usize },
}

#[derive(Debug, Clone, Default)]
pub struct AddRepoModal {
    pub input: TextInput,
    pub error: Option<String>,
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
    OpenAddRepo,
    OpenConfirmRemoveRepo,
    InputChar(char),
    InputBackspace,
    InputDelete,
    InputCursorLeft,
    InputCursorRight,
    InputHome,
    InputEnd,
    SubmitModal,
    CloseModal,
    RefreshAll,
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
    pub fn load(config: Config, config_path: PathBuf) -> Result<Self> {
        let repos = load_repos(&config);
        let cursor = if repos.is_empty() {
            None
        } else {
            Some(SidebarCursor::Repo(0))
        };
        Ok(Self {
            config,
            config_path,
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
            AppMessage::OpenAddRepo => self.open_add_repo(),
            AppMessage::OpenConfirmRemoveRepo => self.open_confirm_remove_repo(),
            AppMessage::InputChar(c) => self.with_add_repo_input(|i| i.insert_char(c)),
            AppMessage::InputBackspace => self.with_add_repo_input(TextInput::backspace),
            AppMessage::InputDelete => self.with_add_repo_input(TextInput::delete),
            AppMessage::InputCursorLeft => self.with_add_repo_input(TextInput::move_left),
            AppMessage::InputCursorRight => self.with_add_repo_input(TextInput::move_right),
            AppMessage::InputHome => self.with_add_repo_input(TextInput::home),
            AppMessage::InputEnd => self.with_add_repo_input(TextInput::end),
            AppMessage::SubmitModal => self.submit_modal(),
            AppMessage::CloseModal => self.ui.modal = None,
            // RefreshAll is handled outside update() because it spawns tasks; the
            // no-op here keeps the match exhaustive.
            AppMessage::RefreshAll => {}
            AppMessage::Quit => self.should_quit = true,
            AppMessage::NoOp => {}
        }
    }

    pub fn set_worktree_status(&mut self, id: (usize, usize), status: crate::model::WorktreeStatus) {
        let (r, w) = id;
        if let Some(repo) = self.repos.get_mut(r) {
            if let Some(wt) = repo.worktrees.get_mut(w) {
                wt.status = Some(status);
            }
        }
    }

    fn open_add_repo(&mut self) {
        if self.ui.modal.is_some() {
            return;
        }
        self.ui.modal = Some(Modal::AddRepo(AddRepoModal::default()));
    }

    fn open_confirm_remove_repo(&mut self) {
        if self.ui.modal.is_some() {
            return;
        }
        let repo_idx = match self.ui.cursor {
            Some(SidebarCursor::Repo(i)) => i,
            Some(SidebarCursor::Worktree { repo, .. }) => repo,
            None => return,
        };
        if repo_idx >= self.repos.len() {
            return;
        }
        self.ui.modal = Some(Modal::ConfirmRemoveRepo { repo_idx });
    }

    fn with_add_repo_input<F>(&mut self, f: F)
    where
        F: FnOnce(&mut TextInput),
    {
        if let Some(Modal::AddRepo(m)) = &mut self.ui.modal {
            f(&mut m.input);
            m.error = None;
        }
    }

    fn submit_modal(&mut self) {
        match self.ui.modal.take() {
            Some(Modal::AddRepo(m)) => {
                let raw = m.input.value().trim().to_string();
                if let Err(err) = self.try_add_repo(&raw) {
                    self.ui.modal = Some(Modal::AddRepo(AddRepoModal {
                        input: m.input,
                        error: Some(format!("{err:#}")),
                    }));
                }
            }
            Some(Modal::ConfirmRemoveRepo { repo_idx }) => {
                if let Err(err) = self.remove_repo(repo_idx) {
                    eprintln!("warning: failed to remove repo: {err:#}");
                }
            }
            other => {
                self.ui.modal = other;
            }
        }
    }

    fn try_add_repo(&mut self, raw: &str) -> Result<()> {
        if raw.is_empty() {
            anyhow::bail!("path cannot be empty");
        }
        let path = resolve_repo_path(raw)?;
        if !path.is_dir() {
            anyhow::bail!("not a directory: {}", path.display());
        }
        if !git::is_git_repo(&path) {
            anyhow::bail!("not a git repository: {}", path.display());
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("could not derive repo name from path"))?;
        if self.config.has_repo_named(&name) {
            anyhow::bail!("a repository named '{name}' already exists");
        }
        let worktrees = git::list_worktrees(&path)
            .with_context(|| format!("discovering worktrees in {}", path.display()))?;

        // Persist to disk first — only commit to in-memory state after save succeeds.
        let mut new_config = self.config.clone();
        new_config.repos.push(RepoConfig {
            name: name.clone(),
            path: path.clone(),
            base_branch: None,
        });
        new_config.save(&self.config_path)?;
        self.config = new_config;

        let base_branch = self.config.general.default_base_branch.clone();
        self.repos.push(Repo {
            name,
            root_path: path,
            base_branch,
            worktrees,
        });
        let new_idx = self.repos.len() - 1;
        self.ui.cursor = Some(SidebarCursor::Repo(new_idx));
        Ok(())
    }

    fn remove_repo(&mut self, idx: usize) -> Result<()> {
        if idx >= self.repos.len() {
            return Ok(());
        }
        let name = self.repos[idx].name.clone();

        let mut new_config = self.config.clone();
        new_config.repos.retain(|r| r.name != name);
        new_config.save(&self.config_path)?;
        self.config = new_config;

        self.repos.remove(idx);
        self.ui.expanded.remove(&name);
        self.ui.active_worktree = self.ui.active_worktree.and_then(|(r, w)| {
            if r == idx {
                None
            } else if r > idx {
                Some((r - 1, w))
            } else {
                Some((r, w))
            }
        });
        if self.repos.is_empty() {
            self.ui.cursor = None;
        } else {
            let new_idx = idx.min(self.repos.len() - 1);
            self.ui.cursor = Some(SidebarCursor::Repo(new_idx));
        }
        Ok(())
    }

    fn toggle_help(&mut self) {
        self.ui.modal = match self.ui.modal.take() {
            Some(Modal::Help) => None,
            None => Some(Modal::Help),
            other => other, // another modal open — leave it alone
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
        let pos = visible.iter().position(|c| *c == current).unwrap_or(0);
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

fn resolve_repo_path(raw: &str) -> Result<PathBuf> {
    let expanded = if raw == "~" {
        home_dir()?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home_dir()?.join(rest)
    } else {
        PathBuf::from(raw)
    };
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .context("getting current directory")?
            .join(expanded)
    };
    std::fs::canonicalize(&absolute)
        .with_context(|| format!("resolving path {}", absolute.display()))
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME environment variable not set"))
}

fn load_repos(config: &Config) -> Vec<Repo> {
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
    repos
}

#[cfg(test)]
impl AppState {
    pub fn fixture() -> Self {
        use crate::model::Worktree;

        let mut state = Self {
            config: Config::default(),
            config_path: PathBuf::from("/tmp/grove-fixture-config.toml"),
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
                            status: None,
                        },
                        Worktree {
                            branch: "feat/sidebar".to_string(),
                            path: PathBuf::from("/Users/sebas/dev/grove-feat-sidebar"),
                            is_primary: false,
                            status: None,
                        },
                        Worktree {
                            branch: "fix/deps".to_string(),
                            path: PathBuf::from("/Users/sebas/dev/grove-fix-deps"),
                            is_primary: false,
                            status: None,
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
                            status: None,
                        },
                        Worktree {
                            branch: "wip/zsh".to_string(),
                            path: PathBuf::from("/Users/sebas/dotfiles-wip-zsh"),
                            is_primary: false,
                            status: None,
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

    pub fn empty_fixture(config_path: PathBuf) -> Self {
        Self {
            config: Config::default(),
            config_path,
            repos: Vec::new(),
            ui: UiState::default(),
            should_quit: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j_moves_cursor_through_visible_tree() {
        let mut app = AppState::fixture();
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
        app.update(AppMessage::MoveCursor(Direction::Up));
        assert_eq!(app.ui.cursor, Some(SidebarCursor::Repo(0)));
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

    #[test]
    fn open_add_repo_replaces_modal_with_add_repo_variant() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        assert!(matches!(app.ui.modal, Some(Modal::AddRepo(_))));
    }

    #[test]
    fn typing_fills_the_add_repo_input() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        for c in "/tmp/foo".chars() {
            app.update(AppMessage::InputChar(c));
        }
        let Some(Modal::AddRepo(m)) = &app.ui.modal else {
            panic!("expected AddRepo modal");
        };
        assert_eq!(m.input.value(), "/tmp/foo");
    }

    #[test]
    fn submit_modal_on_empty_input_surfaces_error() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        app.update(AppMessage::SubmitModal);
        let Some(Modal::AddRepo(m)) = &app.ui.modal else {
            panic!("modal should remain open with error");
        };
        assert!(m.error.is_some(), "expected error to be set");
    }

    #[test]
    fn submit_modal_on_nonexistent_path_surfaces_error() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        for c in "/definitely/does/not/exist/anywhere".chars() {
            app.update(AppMessage::InputChar(c));
        }
        app.update(AppMessage::SubmitModal);
        let Some(Modal::AddRepo(m)) = &app.ui.modal else {
            panic!("modal should remain open with error");
        };
        assert!(m.error.is_some());
    }

    #[test]
    fn try_add_repo_rejects_duplicate_name() {
        use crate::config::RepoConfig;
        let tmp = temp_dir();
        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path);
        // Seed: one repo already registered.
        app.config.repos.push(RepoConfig {
            name: "existing".to_string(),
            path: tmp.clone(),
            base_branch: None,
        });
        let dup_path = tmp.join("existing");
        std::fs::create_dir_all(&dup_path).unwrap();
        init_git_repo(&dup_path);
        let err = app
            .try_add_repo(dup_path.to_str().unwrap())
            .expect_err("should reject duplicate name");
        assert!(format!("{err:#}").contains("already exists"));
    }

    #[test]
    fn try_add_repo_succeeds_on_valid_git_dir_and_persists() {
        let tmp = temp_dir();
        let repo_dir = tmp.join("myproject");
        std::fs::create_dir_all(&repo_dir).unwrap();
        init_git_repo(&repo_dir);

        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path.clone());
        app.try_add_repo(repo_dir.to_str().unwrap()).unwrap();

        assert_eq!(app.repos.len(), 1);
        assert_eq!(app.repos[0].name, "myproject");
        // Config file was written.
        assert!(config_path.exists());
        let reloaded = crate::config::Config::load(&config_path).unwrap();
        assert_eq!(reloaded.repos.len(), 1);
        assert_eq!(reloaded.repos[0].name, "myproject");
    }

    #[test]
    fn remove_repo_drops_from_config_and_state() {
        let tmp = temp_dir();
        let repo_dir = tmp.join("todelete");
        std::fs::create_dir_all(&repo_dir).unwrap();
        init_git_repo(&repo_dir);

        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path.clone());
        app.try_add_repo(repo_dir.to_str().unwrap()).unwrap();
        assert_eq!(app.repos.len(), 1);

        app.remove_repo(0).unwrap();
        assert_eq!(app.repos.len(), 0);
        assert!(app.ui.cursor.is_none());

        let reloaded = crate::config::Config::load(&config_path).unwrap();
        assert_eq!(reloaded.repos.len(), 0);
    }

    #[test]
    fn open_confirm_remove_uses_parent_repo_when_cursor_on_worktree() {
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 1,
            worktree: 0,
        });
        app.update(AppMessage::OpenConfirmRemoveRepo);
        match app.ui.modal {
            Some(Modal::ConfirmRemoveRepo { repo_idx }) => assert_eq!(repo_idx, 1),
            other => panic!("expected ConfirmRemoveRepo(1), got {other:?}"),
        }
    }

    #[test]
    fn canceling_add_repo_does_not_write_config() {
        let tmp = temp_dir();
        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path.clone());
        app.update(AppMessage::OpenAddRepo);
        for c in "/some/path".chars() {
            app.update(AppMessage::InputChar(c));
        }
        app.update(AppMessage::CloseModal);
        assert!(app.ui.modal.is_none());
        assert!(!config_path.exists(), "config must not be written on cancel");
        assert!(app.repos.is_empty());
    }

    #[test]
    fn remove_repo_clears_matching_active_worktree() {
        let mut app = AppState::fixture();
        // Write a dummy config so save() works.
        let tmp = temp_dir();
        app.config_path = tmp.join("config.toml");
        app.config.repos = app
            .repos
            .iter()
            .map(|r| crate::config::RepoConfig {
                name: r.name.clone(),
                path: r.root_path.clone(),
                base_branch: None,
            })
            .collect();

        app.ui.active_worktree = Some((0, 1));
        app.remove_repo(0).unwrap();
        assert_eq!(app.ui.active_worktree, None);
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "grove-app-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn init_git_repo(path: &std::path::Path) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["init", "--quiet", "--initial-branch=main"])
            .status()
            .expect("git init");
        assert!(status.success(), "git init failed");
        // Create an initial commit so `git worktree list` has real output.
        std::fs::write(path.join("README.md"), "test").unwrap();
        run_git(path, &["add", "."]);
        run_git(path, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-m", "init", "--quiet"]);
    }

    fn run_git(path: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }
}
