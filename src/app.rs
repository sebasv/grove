use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::async_evt::WorktreeId;
use crate::config::{Config, RepoConfig};
use crate::git;
use crate::model::Repo;
use crate::state::{ActiveWorktreeId, PersistedState, PersistedUi};
use crate::terminal::Terminal;
use crate::ui::text_input::TextInput;

pub struct AppState {
    pub config: Config,
    pub config_path: PathBuf,
    pub repos: Vec<Repo>,
    pub ui: UiState,
    pub terminals: HashMap<WorktreeId, WorktreeTerminals>,
    pub diffs: HashMap<WorktreeId, DiffState>,
    pub main_views: HashMap<WorktreeId, MainView>,
    pub theme: crate::theme::Theme,
    pub theme_name: crate::theme::ThemeName,
    pub layout: LayoutCache,
    pub should_quit: bool,
}

/// Cached rects from the most recent `ui::render` pass.  Used by mouse
/// dispatch to figure out what was clicked.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutCache {
    pub sidebar: ratatui::layout::Rect,
    pub main: ratatui::layout::Rect,
    pub tab_bar: Option<ratatui::layout::Rect>,
}

pub struct WorktreeTerminals {
    pub list: Vec<Terminal>,
    pub active: usize,
    pub mode: TerminalMode,
    pub scroll_offset: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TerminalMode {
    #[default]
    Insert,
    Scrollback,
}

impl WorktreeTerminals {
    pub fn new(term: Terminal) -> Self {
        Self {
            list: vec![term],
            active: 0,
            mode: TerminalMode::Insert,
            scroll_offset: 0,
        }
    }

    pub fn active_mut(&mut self) -> Option<&mut Terminal> {
        self.list.get_mut(self.active)
    }

    pub fn active_ref(&self) -> Option<&Terminal> {
        self.list.get(self.active)
    }

    pub fn push(&mut self, term: Terminal) {
        self.list.push(term);
        self.active = self.list.len() - 1;
        self.mode = TerminalMode::Insert;
        self.scroll_offset = 0;
    }

    /// Close the active tab; returns true when this was the last tab and
    /// the caller should drop the entire `WorktreeTerminals` entry.
    pub fn close_active(&mut self) -> bool {
        if self.list.is_empty() {
            return true;
        }
        self.list.remove(self.active);
        if self.list.is_empty() {
            return true;
        }
        if self.active >= self.list.len() {
            self.active = self.list.len() - 1;
        }
        self.mode = TerminalMode::Insert;
        self.scroll_offset = 0;
        false
    }

    pub fn next_tab(&mut self) {
        if self.list.is_empty() {
            return;
        }
        self.active = (self.active + 1) % self.list.len();
    }

    pub fn prev_tab(&mut self) {
        if self.list.is_empty() {
            return;
        }
        self.active = if self.active == 0 {
            self.list.len() - 1
        } else {
            self.active - 1
        };
    }

    pub fn toggle_scrollback(&mut self) {
        self.mode = match self.mode {
            TerminalMode::Insert => TerminalMode::Scrollback,
            TerminalMode::Scrollback => {
                self.scroll_offset = 0;
                TerminalMode::Insert
            }
        };
    }

    pub fn scroll(&mut self, delta: i32) {
        let new = self.scroll_offset as i32 + delta;
        self.scroll_offset = new.max(0) as usize;
    }

    pub fn scroll_home(&mut self) {
        self.scroll_offset = 10_000;
    }

    pub fn scroll_end(&mut self) {
        self.scroll_offset = 0;
    }
}

#[derive(Debug, Clone, Default)]
pub struct UiState {
    pub expanded: HashMap<String, bool>,
    pub cursor: Option<SidebarCursor>,
    pub active_worktree: Option<(usize, usize)>,
    pub modal: Option<Modal>,
    pub focus: FocusZone,
    pub help_scroll: u16,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FocusZone {
    #[default]
    Sidebar,
    Main,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MainView {
    #[default]
    Terminal,
    Diff,
}

#[derive(Debug, Clone, Default)]
pub struct DiffState {
    pub files: Vec<crate::git::DiffFile>,
    pub cursor: usize,
    pub scroll: u16,
    pub diff_focus: DiffFocus,
    pub mode: DiffMode,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiffMode {
    #[default]
    Local,
    Branch,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiffFocus {
    #[default]
    List,
    Content,
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
    ConfirmRemoveRepo {
        repo_idx: usize,
    },
    NewWorktree(NewWorktreeModal),
    ConfirmRemoveWorktree {
        id: WorktreeId,
    },
    ConfirmDeleteBranch {
        branch: String,
        repo_root: PathBuf,
        pr_number: Option<u32>,
    },
    ForceDeleteBranch {
        branch: String,
        repo_root: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum NewWorktreeMode {
    PickBranch,
    #[default]
    NewBranch,
}

#[derive(Debug, Clone, Default)]
pub struct AddRepoModal {
    pub input: TextInput,
    pub error: Option<String>,
    pub completions: Vec<String>,
    pub completion_cursor: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct NewWorktreeModal {
    pub mode: NewWorktreeMode,
    pub input: TextInput,
    pub error: Option<String>,
    pub repo_idx: usize,
    pub branches: Vec<crate::git::BranchEntry>,
    pub branch_cursor: usize,
}

impl NewWorktreeModal {
    pub fn for_repo(repo_idx: usize) -> Self {
        Self {
            mode: NewWorktreeMode::NewBranch,
            input: TextInput::default(),
            error: None,
            repo_idx,
            branches: Vec::new(),
            branch_cursor: 0,
        }
    }
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
    OpenNewWorktree,
    OpenConfirmRemoveWorktree,
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
    CycleFocus,
    NewTerminal,
    CloseTerminal,
    NextTab,
    PrevTab,
    ToggleScrollback,
    ScrollUp,
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    ToggleDiffView,
    ToggleDiffMode,
    DiffCursorUp,
    DiffCursorDown,
    DiffToggleFocus,
    DiffContentUp,
    DiffContentDown,
    StageFocused,
    UnstageFocused,
    CycleTheme,
    BranchCursorUp,
    BranchCursorDown,
    SwitchWorktreeMode,
    CompletionUp,
    CompletionDown,
    CompletionAccept,
    HelpScrollUp,
    HelpScrollDown,
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
            terminals: HashMap::new(),
            diffs: HashMap::new(),
            main_views: HashMap::new(),
            theme: crate::theme::Theme::default(),
            theme_name: crate::theme::ThemeName::default(),
            layout: LayoutCache::default(),
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
            AppMessage::OpenNewWorktree => self.open_new_worktree(),
            AppMessage::OpenConfirmRemoveWorktree => self.open_confirm_remove_worktree(),
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
            AppMessage::CycleFocus => self.cycle_focus(),
            AppMessage::NewTerminal => {} // handled outside update (spawn is a side effect)
            AppMessage::CloseTerminal => self.close_active_terminal(),
            AppMessage::NextTab => self.with_active_terminals(|t| t.next_tab()),
            AppMessage::PrevTab => self.with_active_terminals(|t| t.prev_tab()),
            AppMessage::ToggleScrollback => self.with_active_terminals(|t| t.toggle_scrollback()),
            AppMessage::ScrollUp => self.with_active_terminals(|t| t.scroll(1)),
            AppMessage::ScrollDown => self.with_active_terminals(|t| t.scroll(-1)),
            AppMessage::ScrollPageUp => self.with_active_terminals(|t| t.scroll(20)),
            AppMessage::ScrollPageDown => self.with_active_terminals(|t| t.scroll(-20)),
            AppMessage::ScrollTop => self.with_active_terminals(|t| t.scroll_home()),
            AppMessage::ScrollBottom => self.with_active_terminals(|t| t.scroll_end()),
            AppMessage::ToggleDiffView => self.toggle_diff_view(),
            AppMessage::ToggleDiffMode => self.toggle_diff_mode(),
            AppMessage::DiffCursorUp => self.move_diff_cursor(-1),
            AppMessage::DiffCursorDown => self.move_diff_cursor(1),
            AppMessage::DiffToggleFocus => self.toggle_diff_focus(),
            AppMessage::DiffContentUp => self.scroll_diff(-1),
            AppMessage::DiffContentDown => self.scroll_diff(1),
            // Stage/UnstageFocused need side effects (subprocess + refresh); handled outside update.
            AppMessage::StageFocused | AppMessage::UnstageFocused => {}
            AppMessage::CycleTheme => {
                self.theme_name = self.theme_name.next();
                self.theme = crate::theme::resolve(self.theme_name);
            }
            AppMessage::BranchCursorUp => {
                if let Some(Modal::NewWorktree(m)) = &mut self.ui.modal {
                    m.branch_cursor = m.branch_cursor.saturating_sub(1);
                }
            }
            AppMessage::BranchCursorDown => {
                if let Some(Modal::NewWorktree(m)) = &mut self.ui.modal {
                    if !m.branches.is_empty() {
                        m.branch_cursor = (m.branch_cursor + 1).min(m.branches.len() - 1);
                    }
                }
            }
            AppMessage::SwitchWorktreeMode => {
                if let Some(Modal::NewWorktree(m)) = &mut self.ui.modal {
                    m.mode = match m.mode {
                        NewWorktreeMode::PickBranch => NewWorktreeMode::NewBranch,
                        NewWorktreeMode::NewBranch => NewWorktreeMode::PickBranch,
                    };
                    m.error = None;
                }
            }
            AppMessage::CompletionUp => {
                if let Some(Modal::AddRepo(m)) = &mut self.ui.modal {
                    if !m.completions.is_empty() {
                        m.completion_cursor = Some(match m.completion_cursor {
                            None | Some(0) => 0,
                            Some(n) => n - 1,
                        });
                    }
                }
            }
            AppMessage::CompletionDown => {
                if let Some(Modal::AddRepo(m)) = &mut self.ui.modal {
                    if !m.completions.is_empty() {
                        let max = m.completions.len() - 1;
                        m.completion_cursor =
                            Some(m.completion_cursor.map(|n| (n + 1).min(max)).unwrap_or(0));
                    }
                }
            }
            AppMessage::CompletionAccept => self.accept_completion(),
            AppMessage::HelpScrollUp => {
                self.ui.help_scroll = self.ui.help_scroll.saturating_sub(1);
            }
            AppMessage::HelpScrollDown => {
                self.ui.help_scroll = self.ui.help_scroll.saturating_add(1);
            }
            AppMessage::Quit => self.should_quit = true,
            AppMessage::NoOp => {}
        }
    }

    fn toggle_diff_view(&mut self) {
        if let Some(id) = self.ui.active_worktree {
            let next = match self.main_views.get(&id).copied().unwrap_or_default() {
                MainView::Terminal => MainView::Diff,
                MainView::Diff => MainView::Terminal,
            };
            self.main_views.insert(id, next);
        }
    }

    fn toggle_diff_mode(&mut self) {
        if let Some(id) = self.ui.active_worktree {
            let d = self.diffs.entry(id).or_default();
            d.mode = match d.mode {
                DiffMode::Local => DiffMode::Branch,
                DiffMode::Branch => DiffMode::Local,
            };
            d.cursor = 0;
            d.scroll = 0;
            d.files.clear();
        }
    }

    pub fn active_diff_mode(&self) -> DiffMode {
        let Some(id) = self.ui.active_worktree else {
            return DiffMode::Local;
        };
        self.diffs.get(&id).map(|d| d.mode).unwrap_or_default()
    }

    fn move_diff_cursor(&mut self, delta: i32) {
        if let Some(id) = self.ui.active_worktree {
            if let Some(d) = self.diffs.get_mut(&id) {
                if d.files.is_empty() {
                    return;
                }
                let new = d.cursor as i32 + delta;
                let new = new.clamp(0, d.files.len() as i32 - 1) as usize;
                d.cursor = new;
                d.scroll = 0;
            }
        }
    }

    fn toggle_diff_focus(&mut self) {
        if let Some(id) = self.ui.active_worktree {
            if let Some(d) = self.diffs.get_mut(&id) {
                d.diff_focus = match d.diff_focus {
                    DiffFocus::List => DiffFocus::Content,
                    DiffFocus::Content => DiffFocus::List,
                };
            }
        }
    }

    fn scroll_diff(&mut self, delta: i32) {
        if let Some(id) = self.ui.active_worktree {
            if let Some(d) = self.diffs.get_mut(&id) {
                let new = d.scroll as i32 + delta;
                d.scroll = new.max(0) as u16;
            }
        }
    }

    pub fn set_diff(&mut self, id: WorktreeId, files: Vec<crate::git::DiffFile>) {
        let d = self.diffs.entry(id).or_default();
        let preserve_cursor = d.cursor.min(files.len().saturating_sub(1));
        d.files = files;
        d.cursor = preserve_cursor;
    }

    fn with_active_terminals<F: FnOnce(&mut WorktreeTerminals)>(&mut self, f: F) {
        if let Some(id) = self.ui.active_worktree {
            if let Some(ts) = self.terminals.get_mut(&id) {
                f(ts);
            }
        }
    }

    fn close_active_terminal(&mut self) {
        if let Some(id) = self.ui.active_worktree {
            let drop_entry = self
                .terminals
                .get_mut(&id)
                .map(|ts| ts.close_active())
                .unwrap_or(true);
            if drop_entry {
                self.terminals.remove(&id);
                self.ui.focus = FocusZone::Sidebar;
            }
        }
    }

    fn cycle_focus(&mut self) {
        self.ui.focus = match self.ui.focus {
            FocusZone::Sidebar => FocusZone::Main,
            FocusZone::Main => FocusZone::Sidebar,
        };
    }

    pub fn set_worktree_status(
        &mut self,
        id: (usize, usize),
        status: crate::model::WorktreeStatus,
    ) {
        let (r, w) = id;
        if let Some(repo) = self.repos.get_mut(r) {
            if let Some(wt) = repo.worktrees.get_mut(w) {
                wt.status = Some(status);
            }
        }
    }

    pub fn set_worktree_pr(&mut self, id: (usize, usize), pr: crate::model::PrStatus) {
        let (r, w) = id;
        if let Some(repo) = self.repos.get_mut(r) {
            if let Some(wt) = repo.worktrees.get_mut(w) {
                wt.pr = Some(pr);
            }
        }
    }

    fn open_add_repo(&mut self) {
        if self.ui.modal.is_some() {
            return;
        }
        self.ui.modal = Some(Modal::AddRepo(AddRepoModal::default()));
    }

    fn open_new_worktree(&mut self) {
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
        let branches = git::list_branches(&self.repos[repo_idx].root_path).unwrap_or_default();
        let mode = if branches.is_empty() {
            NewWorktreeMode::NewBranch
        } else {
            NewWorktreeMode::PickBranch
        };
        self.ui.modal = Some(Modal::NewWorktree(NewWorktreeModal {
            mode,
            branches,
            ..NewWorktreeModal::for_repo(repo_idx)
        }));
    }

    fn open_confirm_remove_worktree(&mut self) {
        if self.ui.modal.is_some() {
            return;
        }
        let Some(SidebarCursor::Worktree { repo, worktree }) = self.ui.cursor else {
            return;
        };
        // Refuse to remove the primary checkout.
        if self
            .repos
            .get(repo)
            .and_then(|r| r.worktrees.get(worktree))
            .map(|wt| wt.is_primary)
            .unwrap_or(true)
        {
            return;
        }
        self.ui.modal = Some(Modal::ConfirmRemoveWorktree {
            id: (repo, worktree),
        });
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
        match &mut self.ui.modal {
            Some(Modal::AddRepo(m)) => {
                f(&mut m.input);
                m.error = None;
            }
            Some(Modal::NewWorktree(m)) if m.mode == NewWorktreeMode::NewBranch => {
                f(&mut m.input);
                m.error = None;
            }
            _ => return,
        }
        self.recompute_completions();
    }

    fn submit_modal(&mut self) {
        match self.ui.modal.take() {
            Some(Modal::AddRepo(m)) => {
                let raw = m.input.value().trim().to_string();
                if let Err(err) = self.try_add_repo(&raw) {
                    self.ui.modal = Some(Modal::AddRepo(AddRepoModal {
                        error: Some(format!("{err:#}")),
                        ..m
                    }));
                }
            }
            Some(Modal::ConfirmRemoveRepo { repo_idx }) => {
                if let Err(err) = self.remove_repo(repo_idx) {
                    eprintln!("warning: failed to remove repo: {err:#}");
                }
            }
            Some(Modal::NewWorktree(m)) => {
                if let Err(err) = self.try_create_worktree_modal(&m) {
                    self.ui.modal = Some(Modal::NewWorktree(NewWorktreeModal {
                        error: Some(format!("{err:#}")),
                        ..m
                    }));
                }
            }
            Some(Modal::ConfirmRemoveWorktree { id }) => {
                let info = self.repos.get(id.0).and_then(|repo| {
                    repo.worktrees.get(id.1).map(|wt| {
                        let pr_number = wt.pr.as_ref().and_then(|p| {
                            matches!(
                                p.state,
                                crate::model::PrState::Open | crate::model::PrState::Draft
                            )
                            .then_some(p.number)
                        });
                        (wt.branch.clone(), repo.root_path.clone(), pr_number)
                    })
                });
                if let Err(err) = self.try_remove_worktree(id) {
                    eprintln!("warning: failed to remove worktree: {err:#}");
                    return;
                }
                if let Some((branch, repo_root, pr_number)) = info {
                    self.ui.modal = Some(Modal::ConfirmDeleteBranch {
                        branch,
                        repo_root,
                        pr_number,
                    });
                }
            }
            Some(Modal::ConfirmDeleteBranch {
                branch,
                repo_root,
                pr_number: _,
            }) => match git::delete_branch(&repo_root, &branch) {
                Ok(()) => {}
                Err(err) => {
                    let msg = format!("{err:#}");
                    if msg.contains("not fully merged") {
                        self.ui.modal = Some(Modal::ForceDeleteBranch { branch, repo_root });
                    } else {
                        eprintln!("warning: failed to delete branch {branch}: {err:#}");
                    }
                }
            },
            Some(Modal::ForceDeleteBranch { branch, repo_root }) => {
                if let Err(err) = git::force_delete_branch(&repo_root, &branch) {
                    eprintln!("warning: failed to force-delete branch {branch}: {err:#}");
                }
            }
            other => {
                self.ui.modal = other;
            }
        }
    }

    /// Resolve the effective worktree root for a repo: repo-level override wins
    /// over general, then falls back to `None` (sibling strategy).
    fn effective_worktree_root(&self, repo_idx: usize) -> Option<PathBuf> {
        let raw = self
            .config
            .repos
            .get(repo_idx)
            .and_then(|r| r.worktree_root.as_deref())
            .or(self.config.general.worktree_root.as_deref())?;
        expand_path(raw).ok()
    }

    fn try_create_worktree_modal(&mut self, m: &NewWorktreeModal) -> Result<()> {
        let repo_idx = m.repo_idx;
        let Some(repo) = self.repos.get(repo_idx) else {
            anyhow::bail!("repo not found");
        };
        let repo_root = repo.root_path.clone();
        let repo_name = repo.name.clone();
        let base_branch = repo.base_branch.clone();
        let wt_root = self.effective_worktree_root(repo_idx);
        let wt_root_ref = wt_root.as_deref();

        match m.mode {
            NewWorktreeMode::NewBranch => {
                let branch = m.input.value().trim();
                if branch.is_empty() {
                    anyhow::bail!("branch name cannot be empty");
                }
                let path = git::derive_worktree_path(&repo_root, &repo_name, branch, wt_root_ref);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                git::create_worktree(&repo_root, branch, &path, &base_branch)?;
            }
            NewWorktreeMode::PickBranch => {
                let Some(entry) = m.branches.get(m.branch_cursor) else {
                    anyhow::bail!("no branch selected");
                };
                let path =
                    git::derive_worktree_path(&repo_root, &repo_name, &entry.name, wt_root_ref);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                if entry.is_remote_only() {
                    let remote = entry.remote.as_deref().unwrap_or("origin");
                    let local_name = generate_unique_local_name(
                        &self.repos[repo_idx],
                        &entry.name,
                        wt_root_ref,
                    )?;
                    git::create_worktree_from_remote(
                        &repo_root,
                        remote,
                        &entry.name,
                        &local_name,
                        &path,
                    )?;
                } else {
                    git::create_worktree_from_existing(&repo_root, &entry.name, &path)?;
                }
            }
        }

        let new_list = git::list_worktrees(&self.repos[repo_idx].root_path)?;
        self.repos[repo_idx].worktrees = new_list;
        let last = self.repos[repo_idx].worktrees.len().saturating_sub(1);
        self.ui.cursor = Some(SidebarCursor::Worktree {
            repo: repo_idx,
            worktree: last,
        });
        Ok(())
    }

    fn try_remove_worktree(&mut self, id: WorktreeId) -> Result<()> {
        let (r, w) = id;
        let Some(repo) = self.repos.get(r) else {
            return Ok(());
        };
        let Some(wt) = repo.worktrees.get(w) else {
            return Ok(());
        };
        if wt.is_primary {
            anyhow::bail!("refusing to remove primary checkout");
        }
        let wt_path = wt.path.clone();
        let repo_root = repo.root_path.clone();
        git::remove_worktree(&repo_root, &wt_path)?;

        // Re-list worktrees for the repo.
        let new_list = git::list_worktrees(&repo_root)?;
        self.repos[r].worktrees = new_list;

        // Drop any Terminal / Diff state for this specific worktree index.
        // Worktrees later in the list shift down by one.
        let old_len = self.terminals.keys().filter(|(rr, _)| *rr == r).count();
        let surviving: HashMap<WorktreeId, WorktreeTerminals> = self
            .terminals
            .drain()
            .filter_map(|((rr, ww), ts)| {
                if rr == r && ww == w {
                    None
                } else if rr == r && ww > w {
                    Some(((rr, ww - 1), ts))
                } else {
                    Some(((rr, ww), ts))
                }
            })
            .collect();
        self.terminals = surviving;
        let _ = old_len;

        // Similarly shift diffs & main_views.
        let diffs: HashMap<WorktreeId, DiffState> = self
            .diffs
            .drain()
            .filter_map(|((rr, ww), d)| {
                if rr == r && ww == w {
                    None
                } else if rr == r && ww > w {
                    Some(((rr, ww - 1), d))
                } else {
                    Some(((rr, ww), d))
                }
            })
            .collect();
        self.diffs = diffs;
        let views: HashMap<WorktreeId, MainView> = self
            .main_views
            .drain()
            .filter_map(|((rr, ww), v)| {
                if rr == r && ww == w {
                    None
                } else if rr == r && ww > w {
                    Some(((rr, ww - 1), v))
                } else {
                    Some(((rr, ww), v))
                }
            })
            .collect();
        self.main_views = views;

        self.ui.active_worktree = self.ui.active_worktree.and_then(|(rr, ww)| {
            if rr == r && ww == w {
                None
            } else if rr == r && ww > w {
                Some((rr, ww - 1))
            } else {
                Some((rr, ww))
            }
        });

        // Move cursor sensibly.
        if self.repos[r].worktrees.is_empty() {
            self.ui.cursor = Some(SidebarCursor::Repo(r));
        } else {
            let new_w = w.min(self.repos[r].worktrees.len() - 1);
            self.ui.cursor = Some(SidebarCursor::Worktree {
                repo: r,
                worktree: new_w,
            });
        }
        Ok(())
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
            worktree_root: None,
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

        // Drop terminals for the removed repo and shift indices for later repos.
        let surviving: HashMap<WorktreeId, WorktreeTerminals> = self
            .terminals
            .drain()
            .filter_map(|((r, w), ts)| {
                if r == idx {
                    None
                } else if r > idx {
                    Some(((r - 1, w), ts))
                } else {
                    Some(((r, w), ts))
                }
            })
            .collect();
        self.terminals = surviving;

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
            None => {
                self.ui.help_scroll = 0;
                Some(Modal::Help)
            }
            other => other,
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

    fn recompute_completions(&mut self) {
        let val = if let Some(Modal::AddRepo(m)) = &self.ui.modal {
            Some(m.input.value().to_string())
        } else {
            None
        };
        let Some(v) = val else { return };
        let comps = list_matching_dirs(&v);
        if let Some(Modal::AddRepo(m)) = &mut self.ui.modal {
            m.completions = comps;
            m.completion_cursor = None;
        }
    }

    fn accept_completion(&mut self) {
        let info = if let Some(Modal::AddRepo(m)) = &self.ui.modal {
            m.completion_cursor.and_then(|cursor| {
                m.completions
                    .get(cursor)
                    .map(|dir| (dir.clone(), m.input.value().to_string()))
            })
        } else {
            None
        };
        let Some((dir, current)) = info else { return };
        let base = match current.rfind('/') {
            Some(pos) => current[..=pos].to_string(),
            None => String::new(),
        };
        let new_value = format!("{base}{dir}/");
        let comps = list_matching_dirs(&new_value);
        let mut new_input = TextInput::default();
        for c in new_value.chars() {
            new_input.insert_char(c);
        }
        if let Some(Modal::AddRepo(m)) = &mut self.ui.modal {
            m.input = new_input;
            m.completion_cursor = None;
            m.error = None;
            m.completions = comps;
        }
    }
}

/// Expand `~` and resolve relative paths without requiring the path to exist.
fn expand_path(raw: &std::path::Path) -> Result<PathBuf> {
    let s = raw.to_string_lossy();
    let expanded = if s == "~" {
        home_dir()?
    } else if let Some(rest) = s.strip_prefix("~/") {
        home_dir()?.join(rest)
    } else {
        raw.to_path_buf()
    };
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        Ok(std::env::current_dir()
            .context("getting current directory")?
            .join(expanded))
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

fn list_matching_dirs(prefix: &str) -> Vec<String> {
    const MAX: usize = 10;
    let expanded = if prefix == "~" || prefix.is_empty() {
        match std::env::var_os("HOME") {
            Some(h) => format!("{}/", PathBuf::from(h).display()),
            None => return Vec::new(),
        }
    } else if let Some(rest) = prefix.strip_prefix("~/") {
        match std::env::var_os("HOME") {
            Some(h) => format!("{}/{rest}", PathBuf::from(h).display()),
            None => prefix.to_string(),
        }
    } else {
        prefix.to_string()
    };

    let (dir_part, segment) = match expanded.rfind('/') {
        Some(pos) => (&expanded[..=pos], &expanded[pos + 1..]),
        None => ("./", expanded.as_str()),
    };
    let dir = PathBuf::from(dir_part);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut matches: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().ok().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .filter(|name| name.starts_with(segment))
        .collect();
    matches.sort();
    matches.truncate(MAX);
    matches
}

fn generate_unique_local_name(
    repo: &crate::model::Repo,
    branch_name: &str,
    worktree_root: Option<&std::path::Path>,
) -> Result<String> {
    let existing: std::collections::HashSet<&str> =
        repo.worktrees.iter().map(|wt| wt.branch.as_str()).collect();
    for _ in 0..5 {
        let slug = random_slug();
        let candidate = format!("{branch_name}-{slug}");
        let path =
            git::derive_worktree_path(&repo.root_path, &repo.name, &candidate, worktree_root);
        if !existing.contains(candidate.as_str()) && !path.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("could not generate a unique local branch name after 5 attempts")
}

fn random_slug() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:06x}", n & 0x00ff_ffff)
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
                            pr: None,
                            status: None,
                        },
                        Worktree {
                            branch: "feat/sidebar".to_string(),
                            path: PathBuf::from("/Users/sebas/dev/grove-feat-sidebar"),
                            is_primary: false,
                            pr: None,
                            status: None,
                        },
                        Worktree {
                            branch: "fix/deps".to_string(),
                            path: PathBuf::from("/Users/sebas/dev/grove-fix-deps"),
                            is_primary: false,
                            pr: None,
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
                            pr: None,
                            status: None,
                        },
                        Worktree {
                            branch: "wip/zsh".to_string(),
                            path: PathBuf::from("/Users/sebas/dotfiles-wip-zsh"),
                            is_primary: false,
                            pr: None,
                            status: None,
                        },
                    ],
                },
            ],
            ui: UiState::default(),
            terminals: HashMap::new(),
            diffs: HashMap::new(),
            main_views: HashMap::new(),
            theme: crate::theme::Theme::default(),
            theme_name: crate::theme::ThemeName::default(),
            layout: LayoutCache::default(),
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
            terminals: HashMap::new(),
            diffs: HashMap::new(),
            main_views: HashMap::new(),
            theme: crate::theme::Theme::default(),
            theme_name: crate::theme::ThemeName::default(),
            layout: LayoutCache::default(),
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
    fn worktree_terminals_tab_arithmetic() {
        // We can't construct a real Terminal in tests, so exercise the
        // bookkeeping methods on an empty-list fixture that starts with
        // `scroll_offset=0` and `mode=Insert`.
        let mut ts = WorktreeTerminals {
            list: Vec::new(),
            active: 0,
            mode: TerminalMode::Insert,
            scroll_offset: 0,
        };
        // next/prev on empty → no panic
        ts.next_tab();
        ts.prev_tab();
        ts.toggle_scrollback();
        assert_eq!(ts.mode, TerminalMode::Scrollback);
        ts.scroll(5);
        assert_eq!(ts.scroll_offset, 5);
        ts.scroll(-20);
        assert_eq!(ts.scroll_offset, 0);
        ts.scroll_home();
        assert!(ts.scroll_offset > 0);
        ts.scroll_end();
        assert_eq!(ts.scroll_offset, 0);
        ts.toggle_scrollback();
        assert_eq!(ts.mode, TerminalMode::Insert);
    }

    #[test]
    fn cycle_focus_toggles_between_sidebar_and_main() {
        let mut app = AppState::fixture();
        assert_eq!(app.ui.focus, FocusZone::Sidebar);
        app.update(AppMessage::CycleFocus);
        assert_eq!(app.ui.focus, FocusZone::Main);
        app.update(AppMessage::CycleFocus);
        assert_eq!(app.ui.focus, FocusZone::Sidebar);
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
            worktree_root: None,
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
        assert!(
            !config_path.exists(),
            "config must not be written on cancel"
        );
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
                worktree_root: None,
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
        run_git(
            path,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "init",
                "--quiet",
            ],
        );
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
