use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::activity::ActivityState;
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
    pub activity: ActivityState,
    /// True when `github::build_client` produced a client — i.e. grove
    /// could see a `GITHUB_TOKEN`/`GH_TOKEN` env var or a logged-in
    /// `gh` CLI.  Set by `main.rs` right after token discovery.
    pub github_authenticated: bool,
    /// True when at least one configured repo has a recognised GitHub
    /// `origin` remote.  The sidebar shows an "auth" warning iff this
    /// is true and `github_authenticated` is false.  Cached to avoid
    /// shelling out to `git remote` on every render.
    pub has_github_repo: bool,
    /// Set when `config.toml` exists but failed to parse.  Grove falls
    /// back to defaults so the user still gets a working TUI; this
    /// message surfaces in the sidebar so the typo isn't silent.
    pub config_error: Option<String>,
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

    /// Aggregate per-worktree agent state across every terminal in this
    /// worktree.  Precedence: bell (waiting on user) → thinking → idle.
    ///
    /// Two signals feed "thinking":
    /// 1. The shell's window title starts with a braille spinner glyph
    ///    (Claude Code, gum, oh-my-zsh — anything that uses U+2800–U+28FF
    ///    as a busy indicator).  This is precise: the spinner is ONLY
    ///    rendered while the agent is producing output.
    /// 2. Falling-edge fallback: a TUI that doesn't set a title is
    ///    treated as "thinking" for `fallback_window` after each PTY
    ///    write.  Noisy but better than nothing for shells that don't
    ///    announce themselves.
    pub fn agent_state(&self, fallback_window: std::time::Duration) -> AgentState {
        let mut bell = false;
        let mut thinking = false;
        let now = std::time::Instant::now();
        for t in &self.list {
            let snap = t.activity_snapshot();
            if snap.bell_pending {
                bell = true;
            }
            if crate::terminal::title_is_thinking(&snap.title) {
                thinking = true;
                continue;
            }
            // Fallback: only consult last_output_at when no title is
            // available; otherwise the title is authoritative ("idle"
            // titles wouldn't get overridden by a recent output blip).
            if snap.title.is_empty() {
                if let Some(last) = snap.last_output_at {
                    if now.saturating_duration_since(last) <= fallback_window {
                        thinking = true;
                    }
                }
            }
        }
        if bell {
            AgentState::Waiting
        } else if thinking {
            AgentState::Thinking
        } else {
            AgentState::Idle
        }
    }

    /// Clear the bell-pending flag on every terminal in this worktree.
    /// Called when the user activates the worktree so the indicator
    /// disappears as soon as the user has acknowledged it.
    pub fn clear_bells(&self) {
        for t in &self.list {
            t.clear_bell();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Idle,
    /// Recent PTY output — something is happening / "thinking".
    Thinking,
    /// BEL received and not yet acknowledged — needs attention.
    Waiting,
}

#[derive(Debug, Clone, Default)]
pub struct UiState {
    pub expanded: HashMap<String, bool>,
    pub cursor: Option<SidebarCursor>,
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
    /// Unified worktree-deletion modal.  Offers keep + a single
    /// contextual delete option based on `variant`.  Keys are
    /// absolute (`k` / `d` / `D` / `Esc`) rather than an
    /// arrow-navigable list, so muscle-memory Enter can't silently
    /// destroy unmerged work.
    ConfirmRemoveWorktree {
        id: WorktreeId,
        variant: DeleteVariant,
        /// An open PR number for this branch, if any.  Renders a
        /// "force-delete will close PR #N" warning above the options in
        /// the unmerged variant; the regular `-d` path in the merged
        /// variant is safe for an open PR so the warning is suppressed
        /// there.
        pr_number: Option<u32>,
        /// Inline error after a failed submit (e.g. `d` attempted on an
        /// unmerged branch).  Clears when the user picks a different
        /// option.
        error: Option<String>,
    },
    /// Results of a `grove <dir>` scan waiting for the user to pick
    /// which repos to add.
    DiscoveredRepos(DiscoveredReposModal),
    /// Tail of grove's log file — last few hundred lines, scrollable.
    /// Surfaces background warnings (failed-to-delete-branch etc.) that
    /// would otherwise live silently in `~/.cache/grove/grove.log`.
    ViewLog(LogModal),
}

#[derive(Debug, Clone)]
pub struct LogModal {
    pub lines: Vec<String>,
    pub scroll: usize,
    /// `None` when the log file doesn't exist yet (first run, no warnings).
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteVariant {
    /// Branch is fully merged into its base — safe to `git branch -d`.
    Merged,
    /// Branch has unmerged commits — only force-delete (`-D`) will
    /// remove it.
    Unmerged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteChoice {
    /// Keep the branch; just remove the worktree (safest default).
    KeepBranch,
    /// `git branch -d` — rejected by git if the branch isn't merged.
    Delete,
    /// `git branch -D` — unconditional.
    ForceDelete,
}

#[derive(Debug, Clone)]
pub struct DiscoveredReposModal {
    pub scan_root: PathBuf,
    pub scanning: bool,
    pub candidates: Vec<DiscoveredRepo>,
    pub cursor: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredRepo {
    pub path: PathBuf,
    pub name: String,
    pub already_configured: bool,
    pub selected: bool,
}

impl DiscoveredReposModal {
    pub fn scanning(root: PathBuf) -> Self {
        Self {
            scan_root: root,
            scanning: true,
            candidates: Vec::new(),
            cursor: 0,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AddRepoModal {
    pub input: TextInput,
    pub error: Option<String>,
    pub completions: Vec<String>,
    pub completion_cursor: Option<usize>,
}

/// Unified new-worktree modal.  One text input + one filter-as-you-type
/// list.  The first row is always "Create new branch <input>" (disabled
/// when the input is empty); rows below are existing local and remote
/// branches matching the filter.
#[derive(Debug, Clone)]
pub struct NewWorktreeModal {
    pub repo_idx: usize,
    pub input: TextInput,
    pub error: Option<String>,
    pub branches: Vec<crate::git::BranchEntry>,
    /// Indices into `branches` that match the current filter.  Empty when
    /// the modal is fresh (no filter) but also when the repo genuinely
    /// has no branches.
    pub filter_matches: Vec<usize>,
    /// 0 = "create new branch" row. 1.. = `filter_matches[cursor - 1]`.
    /// Stays at 0 when the user types, so Enter defaults to creating.
    pub cursor: usize,
}

impl NewWorktreeModal {
    pub fn for_repo(repo_idx: usize, branches: Vec<crate::git::BranchEntry>) -> Self {
        let filter_matches = (0..branches.len()).collect();
        Self {
            repo_idx,
            input: TextInput::default(),
            error: None,
            branches,
            filter_matches,
            cursor: 0,
        }
    }

    /// Total row count (create-new + filtered branches).
    pub fn total_rows(&self) -> usize {
        1 + self.filter_matches.len()
    }

    /// Return the selected branch entry, if the cursor is on a branch row.
    pub fn selected_branch(&self) -> Option<&crate::git::BranchEntry> {
        if self.cursor == 0 {
            return None;
        }
        let idx = *self.filter_matches.get(self.cursor - 1)?;
        self.branches.get(idx)
    }

    pub fn recompute_filter(&mut self) {
        let needle = self.input.value().to_lowercase();
        let matches: Vec<usize> = if needle.is_empty() {
            (0..self.branches.len()).collect()
        } else {
            self.branches
                .iter()
                .enumerate()
                .filter_map(|(i, b)| {
                    if b.display().to_lowercase().contains(&needle) {
                        Some(i)
                    } else {
                        None
                    }
                })
                .collect()
        };
        self.filter_matches = matches;
        // Keep the cursor on the "create new" row by default so typing
        // never silently steers Enter onto an existing branch.
        self.cursor = 0;
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
}

#[derive(Debug, Clone)]
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
    CompletionUp,
    CompletionDown,
    CompletionAccept,
    OpenLogViewer,
    LogScrollUp,
    LogScrollDown,
    LogScrollPageUp,
    LogScrollPageDown,
    LogScrollTop,
    LogScrollBottom,
    HelpScrollUp,
    HelpScrollDown,
    /// A `grove <dir>` scan finished; populate the DiscoveredRepos modal.
    ScanReady(Vec<PathBuf>),
    /// Toggle the selection flag for the candidate under the cursor.
    ToggleDiscoveredSelection,
    DiscoveredCursorUp,
    DiscoveredCursorDown,
    /// User picked a deletion option in the ConfirmRemoveWorktree modal.
    ConfirmWorktreeDeletion(DeleteChoice),
    Quit,
    NoOp,
}

impl UiState {
    pub fn is_expanded(&self, repo_path: &Path) -> bool {
        // Repos are expanded by default; state only tracks explicit collapses.
        self.expanded
            .get(repo_path.to_string_lossy().as_ref())
            .copied()
            .unwrap_or(true)
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
        let mut activity = ActivityState::default();
        activity.resize_repos(repos.len());
        let has_github_repo = any_github_remote(&repos);
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
            activity,
            github_authenticated: false,
            has_github_repo,
            config_error: None,
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
                // Persist immediately so the choice survives restarts.
                // Best-effort: failures go to the log rather than killing
                // the TUI (config could be read-only, disk full, etc.).
                self.config.theme.base = self.theme_name;
                if let Err(err) = self.config.save(&self.config_path) {
                    crate::paths::log_warning(&format!("failed to persist theme: {err:#}"));
                }
            }
            AppMessage::BranchCursorUp => {
                if let Some(Modal::NewWorktree(m)) = &mut self.ui.modal {
                    m.cursor = m.cursor.saturating_sub(1);
                }
            }
            AppMessage::BranchCursorDown => {
                if let Some(Modal::NewWorktree(m)) = &mut self.ui.modal {
                    let last = m.total_rows().saturating_sub(1);
                    m.cursor = (m.cursor + 1).min(last);
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
            AppMessage::OpenLogViewer => self.open_log_viewer(),
            AppMessage::LogScrollUp => self.scroll_log(-1),
            AppMessage::LogScrollDown => self.scroll_log(1),
            AppMessage::LogScrollPageUp => self.scroll_log(-20),
            AppMessage::LogScrollPageDown => self.scroll_log(20),
            AppMessage::LogScrollTop => self.scroll_log(i32::MIN),
            AppMessage::LogScrollBottom => self.scroll_log(i32::MAX),
            AppMessage::HelpScrollUp => {
                self.ui.help_scroll = self.ui.help_scroll.saturating_sub(1);
            }
            AppMessage::HelpScrollDown => {
                self.ui.help_scroll = self.ui.help_scroll.saturating_add(1);
            }
            AppMessage::ScanReady(paths) => self.apply_scan_results(paths),
            AppMessage::ToggleDiscoveredSelection => self.toggle_discovered_selection(),
            AppMessage::DiscoveredCursorUp => self.move_discovered_cursor(-1),
            AppMessage::DiscoveredCursorDown => self.move_discovered_cursor(1),
            AppMessage::ConfirmWorktreeDeletion(choice) => self.resolve_worktree_deletion(choice),
            AppMessage::Quit => self.should_quit = true,
            AppMessage::NoOp => {}
        }
    }

    fn apply_scan_results(&mut self, paths: Vec<PathBuf>) {
        let Some(Modal::DiscoveredRepos(m)) = &mut self.ui.modal else {
            return;
        };
        // Config stores canonicalised paths (from `resolve_repo_path`).
        // Canonicalise on both sides of the comparison so duplicate
        // detection works regardless of which spelling the user typed.
        let configured: std::collections::HashSet<PathBuf> = self
            .config
            .repos
            .iter()
            .map(|r| std::fs::canonicalize(&r.path).unwrap_or_else(|_| r.path.clone()))
            .collect();
        let candidates: Vec<DiscoveredRepo> = paths
            .into_iter()
            .map(|path| {
                let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                let already_configured = configured.contains(&canonical);
                let name = canonical
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| canonical.display().to_string());
                DiscoveredRepo {
                    already_configured,
                    selected: !already_configured,
                    path: canonical,
                    name,
                }
            })
            .collect();
        m.scanning = false;
        m.cursor = 0;
        m.candidates = candidates;
    }

    fn toggle_discovered_selection(&mut self) {
        let Some(Modal::DiscoveredRepos(m)) = &mut self.ui.modal else {
            return;
        };
        if let Some(c) = m.candidates.get_mut(m.cursor) {
            c.selected = !c.selected;
        }
    }

    fn move_discovered_cursor(&mut self, delta: i32) {
        let Some(Modal::DiscoveredRepos(m)) = &mut self.ui.modal else {
            return;
        };
        if m.candidates.is_empty() {
            return;
        }
        let last = m.candidates.len() - 1;
        let new = (m.cursor as i32 + delta).clamp(0, last as i32) as usize;
        m.cursor = new;
    }

    fn toggle_diff_view(&mut self) {
        if let Some(id) = self.active_worktree_id() {
            let next = match self.main_views.get(&id).copied().unwrap_or_default() {
                MainView::Terminal => MainView::Diff,
                MainView::Diff => MainView::Terminal,
            };
            self.main_views.insert(id, next);
        }
    }

    fn toggle_diff_mode(&mut self) {
        if let Some(id) = self.active_worktree_id() {
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
        let Some(id) = self.active_worktree_id() else {
            return DiffMode::Local;
        };
        self.diffs.get(&id).map(|d| d.mode).unwrap_or_default()
    }

    fn move_diff_cursor(&mut self, delta: i32) {
        if let Some(id) = self.active_worktree_id() {
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
        if let Some(id) = self.active_worktree_id() {
            if let Some(d) = self.diffs.get_mut(&id) {
                d.diff_focus = match d.diff_focus {
                    DiffFocus::List => DiffFocus::Content,
                    DiffFocus::Content => DiffFocus::List,
                };
            }
        }
    }

    fn scroll_diff(&mut self, delta: i32) {
        if let Some(id) = self.active_worktree_id() {
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
        if let Some(id) = self.active_worktree_id() {
            if let Some(ts) = self.terminals.get_mut(&id) {
                f(ts);
            }
        }
    }

    fn close_active_terminal(&mut self) {
        if let Some(id) = self.active_worktree_id() {
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

    pub fn active_worktree_id(&self) -> Option<WorktreeId> {
        let SidebarCursor::Worktree { repo, worktree } = self.ui.cursor? else {
            return None;
        };
        let _ = self.repos.get(repo)?.worktrees.get(worktree)?;
        Some((repo, worktree))
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
        self.ui.modal = Some(Modal::NewWorktree(NewWorktreeModal::for_repo(
            repo_idx, branches,
        )));
    }

    fn open_confirm_remove_worktree(&mut self) {
        if self.ui.modal.is_some() {
            return;
        }
        let Some(SidebarCursor::Worktree { repo, worktree }) = self.ui.cursor else {
            return;
        };
        let Some(repo_ref) = self.repos.get(repo) else {
            return;
        };
        let Some(wt) = repo_ref.worktrees.get(worktree) else {
            return;
        };
        // Refuse to remove the primary checkout.
        if wt.is_primary {
            return;
        }

        // Figure out whether a regular `-d` would succeed, so the modal
        // shows `[d]` or `[D]` depending on the branch's merge state.
        // libgit2 does this offline, no network. Detached HEAD has no
        // branch to delete, so treat it as `Merged` — the delete-branch
        // path is a no-op for that case (no branch name to pass).
        let variant = match wt.branch_name() {
            Some(branch)
                if !git::is_branch_merged(&repo_ref.root_path, branch, &repo_ref.base_branch) =>
            {
                DeleteVariant::Unmerged
            }
            _ => DeleteVariant::Merged,
        };
        let pr_number = wt.pr.as_ref().and_then(|p| {
            matches!(
                p.state,
                crate::model::PrState::Open | crate::model::PrState::Draft
            )
            .then_some(p.number)
        });

        self.ui.modal = Some(Modal::ConfirmRemoveWorktree {
            id: (repo, worktree),
            variant,
            pr_number,
            error: None,
        });
    }

    fn resolve_worktree_deletion(&mut self, choice: DeleteChoice) {
        // Pull the modal out so we can rebuild it with any inline error
        // without holding a borrow on `self`.
        let Some(Modal::ConfirmRemoveWorktree {
            id,
            variant,
            pr_number,
            ..
        }) = self.ui.modal.take()
        else {
            return;
        };

        // `d` on an unmerged branch is rejected inline rather than chaining
        // to a force-delete modal.  The user must explicitly pick `D`.
        if choice == DeleteChoice::Delete && variant == DeleteVariant::Unmerged {
            self.ui.modal = Some(Modal::ConfirmRemoveWorktree {
                id,
                variant,
                pr_number,
                error: Some("branch has unmerged commits; press D to force-delete".to_string()),
            });
            return;
        }

        // Collect branch + repo_root before we invalidate indices with
        // `try_remove_worktree`. Detached HEAD has no branch to delete —
        // skip the branch-deletion step regardless of the user's choice.
        let Some((branch, repo_root)) = self.repos.get(id.0).and_then(|repo| {
            repo.worktrees
                .get(id.1)
                .map(|wt| (wt.branch_name().map(str::to_string), repo.root_path.clone()))
        }) else {
            return;
        };

        if let Err(err) = self.try_remove_worktree(id) {
            crate::paths::log_warning(&format!("failed to remove worktree: {err:#}"));
            return;
        }

        match (choice, branch) {
            (DeleteChoice::KeepBranch, _) | (_, None) => {}
            (DeleteChoice::Delete, Some(branch)) => {
                if let Err(err) = git::delete_branch(&repo_root, &branch) {
                    crate::paths::log_warning(&format!(
                        "failed to delete branch {branch}: {err:#}"
                    ));
                }
            }
            (DeleteChoice::ForceDelete, Some(branch)) => {
                if let Err(err) = git::force_delete_branch(&repo_root, &branch) {
                    crate::paths::log_warning(&format!(
                        "failed to force-delete branch {branch}: {err:#}"
                    ));
                }
            }
        }
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
                self.recompute_completions();
            }
            Some(Modal::NewWorktree(m)) => {
                f(&mut m.input);
                m.error = None;
                // Filter recomputes on every keystroke.  The branch list
                // for a repo is typically < 100 entries, so a linear
                // substring match is fast enough to not warrant a real
                // fuzzy library.
                m.recompute_filter();
            }
            _ => (),
        }
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
                    crate::paths::log_warning(&format!("failed to remove repo: {err:#}"));
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
            // ConfirmRemoveWorktree now resolves via ConfirmWorktreeDeletion
            // (absolute-key path), not via the generic SubmitModal.  A stray
            // Enter on the keep-branch row routes via KeepBranch below.
            Some(modal @ Modal::ConfirmRemoveWorktree { .. }) => {
                self.ui.modal = Some(modal);
                self.resolve_worktree_deletion(DeleteChoice::KeepBranch);
            }
            Some(Modal::DiscoveredRepos(m)) => {
                if m.scanning {
                    // Scan still running; keep the modal open.
                    self.ui.modal = Some(Modal::DiscoveredRepos(m));
                    return;
                }
                if let Err(err) = self.add_discovered_repos(&m) {
                    self.ui.modal = Some(Modal::DiscoveredRepos(DiscoveredReposModal {
                        error: Some(format!("{err:#}")),
                        ..m
                    }));
                }
            }
            other => {
                self.ui.modal = other;
            }
        }
    }

    fn add_discovered_repos(&mut self, modal: &DiscoveredReposModal) -> Result<()> {
        let mut added = 0usize;
        for c in &modal.candidates {
            if !c.selected || c.already_configured {
                continue;
            }
            if let Err(err) = self.try_add_repo(&c.path.display().to_string()) {
                anyhow::bail!(
                    "failed to add {} ({}); stopped after {added} repo{plural}",
                    c.path.display(),
                    err,
                    plural = if added == 1 { "" } else { "s" },
                );
            }
            added += 1;
        }
        Ok(())
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

        // Cursor == 0 → "Create new branch <input>".  Any other cursor
        // position selects an existing branch.
        let new_path: PathBuf = match m.selected_branch() {
            None => {
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
                path
            }
            Some(entry) => {
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
                path
            }
        };

        let new_list = git::list_worktrees(&self.repos[repo_idx].root_path)?;
        // Remap state keyed by worktree index before replacing the list.
        // libgit2 enumerates linked worktrees from the filesystem (alphabetical
        // on APFS/macOS), so adding a new worktree can shift existing indices.
        self.remap_worktree_state(repo_idx, &new_list);
        self.repos[repo_idx].worktrees = new_list;
        // Land the cursor on the *just-created* worktree by path: list order
        // is alphabetical, so it isn't necessarily at the tail.  Falls back
        // to the parent repo when the lookup somehow fails (shouldn't, but
        // a stale cursor index would point at the wrong worktree).
        let new_idx = self.repos[repo_idx]
            .worktrees
            .iter()
            .position(|wt| wt.path == new_path);
        self.ui.cursor = match new_idx {
            Some(idx) => Some(SidebarCursor::Worktree {
                repo: repo_idx,
                worktree: idx,
            }),
            None => Some(SidebarCursor::Repo(repo_idx)),
        };
        Ok(())
    }

    /// Remap `terminals`, `diffs`, `main_views`, and the sidebar cursor so
    /// they continue to track the same worktrees after a re-list may have
    /// reordered them.  Matching is done by filesystem path, which is stable
    /// across re-lists.
    fn remap_worktree_state(&mut self, repo_idx: usize, new_list: &[crate::model::Worktree]) {
        let r = repo_idx;
        let path_to_new: HashMap<std::path::PathBuf, usize> = new_list
            .iter()
            .enumerate()
            .map(|(j, wt)| (wt.path.clone(), j))
            .collect();
        let old_to_new: HashMap<usize, usize> = self.repos[r]
            .worktrees
            .iter()
            .enumerate()
            .filter_map(|(i, wt)| path_to_new.get(&wt.path).map(|&j| (i, j)))
            .collect();

        let new_terminals: HashMap<WorktreeId, WorktreeTerminals> = self
            .terminals
            .drain()
            .filter_map(|((rr, ww), v)| {
                if rr == r {
                    old_to_new.get(&ww).map(|&nw| ((rr, nw), v))
                } else {
                    Some(((rr, ww), v))
                }
            })
            .collect();
        self.terminals = new_terminals;

        let new_diffs: HashMap<WorktreeId, DiffState> = self
            .diffs
            .drain()
            .filter_map(|((rr, ww), v)| {
                if rr == r {
                    old_to_new.get(&ww).map(|&nw| ((rr, nw), v))
                } else {
                    Some(((rr, ww), v))
                }
            })
            .collect();
        self.diffs = new_diffs;

        let new_views: HashMap<WorktreeId, MainView> = self
            .main_views
            .drain()
            .filter_map(|((rr, ww), v)| {
                if rr == r {
                    old_to_new.get(&ww).map(|&nw| ((rr, nw), v))
                } else {
                    Some(((rr, ww), v))
                }
            })
            .collect();
        self.main_views = new_views;

        // The cursor IS the active selection now that the main pane derives
        // from it.  Translate by path: if the worktree the cursor pointed at
        // survived the re-list, follow it to its new index; if it was
        // removed, fall back to a nearby survivor (or the parent repo when
        // the list is empty) so the main pane never displays content for a
        // worktree the user can no longer see in the sidebar.
        if let Some(SidebarCursor::Worktree {
            repo: cr,
            worktree: cw,
        }) = self.ui.cursor
        {
            if cr == r {
                if let Some(&nw) = old_to_new.get(&cw) {
                    self.ui.cursor = Some(SidebarCursor::Worktree {
                        repo: cr,
                        worktree: nw,
                    });
                } else if new_list.is_empty() {
                    self.ui.cursor = Some(SidebarCursor::Repo(cr));
                } else {
                    let nw = cw.min(new_list.len() - 1);
                    self.ui.cursor = Some(SidebarCursor::Worktree {
                        repo: cr,
                        worktree: nw,
                    });
                }
            }
        }
    }

    /// Re-list a repo's worktrees from disk and reconcile in-memory state.
    /// Called from the FS-watcher event handler so a `git switch`, `git
    /// worktree add`, or `git worktree remove` performed in a terminal is
    /// reflected in the sidebar without requiring a manual refresh. State
    /// keyed by path (terminals, diffs, main views, active selection)
    /// survives the reconcile.
    pub fn reconcile_worktrees(&mut self, repo_idx: usize) {
        let Some(repo) = self.repos.get(repo_idx) else {
            return;
        };
        let new_list = match git::list_worktrees(&repo.root_path) {
            Ok(list) => list,
            Err(err) => {
                crate::paths::log_warning(&format!(
                    "failed to re-list worktrees for {}: {err:#}",
                    repo.name
                ));
                return;
            }
        };
        if new_list == self.repos[repo_idx].worktrees {
            return; // no-op when nothing changed (HEADs and paths match)
        }
        self.remap_worktree_state(repo_idx, &new_list);
        self.repos[repo_idx].worktrees = new_list;
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

        // Move cursor sensibly — this is also what drives the main pane
        // now, so landing on a survivor (or the parent repo when empty)
        // automatically refreshes what's shown.
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

    pub fn try_add_repo(&mut self, raw: &str) -> Result<()> {
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

        // Resolve the repo's default branch from `origin/HEAD` if it's
        // already been fetched. Falling back to `general.default_base_branch`
        // when the remote hasn't been fetched yet means grove hands a repo
        // that uses `master` (or `develop`, or …) the wrong base on first
        // add; reading `origin/HEAD` avoids that for the common case.
        let detected = git::detect_default_branch(&path);
        let base_branch = detected
            .clone()
            .unwrap_or_else(|| self.config.general.default_base_branch.clone());

        // Persist to disk first — only commit to in-memory state after save succeeds.
        let mut new_config = self.config.clone();
        new_config.repos.push(RepoConfig {
            name: name.clone(),
            path: path.clone(),
            base_branch: detected,
            worktree_root: None,
        });
        new_config.save(&self.config_path)?;
        self.config = new_config;

        self.repos.push(Repo {
            name,
            root_path: path,
            base_branch,
            worktrees,
        });
        self.activity.resize_repos(self.repos.len());
        self.has_github_repo = any_github_remote(&self.repos);
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

        let path_key = self.repos[idx].root_path.to_string_lossy().into_owned();
        self.repos.remove(idx);
        // Shift the activity scheduler's last-fetched timeline so subsequent
        // repos keep their timestamps.  Repos at and after `idx` slide left by
        // one; the tail becomes `None` via `resize_repos`.
        if idx < self.activity.last_fetched_at.len() {
            self.activity.last_fetched_at.remove(idx);
        }
        self.activity.resize_repos(self.repos.len());
        self.has_github_repo = any_github_remote(&self.repos);
        self.ui.expanded.remove(&path_key);

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

    fn open_log_viewer(&mut self) {
        if self.ui.modal.is_some() {
            return;
        }
        // Cap the read at 256 KB.  At ~120 columns / line that's well over
        // 1500 lines — plenty of recent context, with bounded memory and
        // a snappy open even on multi-MB log files.
        let lines = crate::paths::read_log_tail(256 * 1024).unwrap_or_default();
        let scroll = lines.len().saturating_sub(1); // open scrolled to the tail
        self.ui.modal = Some(Modal::ViewLog(LogModal {
            lines,
            scroll,
            source: crate::paths::log_path(),
        }));
    }

    fn scroll_log(&mut self, delta: i32) {
        let Some(Modal::ViewLog(m)) = &mut self.ui.modal else {
            return;
        };
        let max = m.lines.len().saturating_sub(1);
        m.scroll = if delta == i32::MIN {
            0
        } else if delta == i32::MAX {
            max
        } else {
            let cur = m.scroll as i64;
            cur.saturating_add(delta as i64).clamp(0, max as i64) as usize
        };
    }

    pub fn apply_persisted(&mut self, persisted: PersistedState) {
        self.ui.expanded = persisted.ui.expanded;
        if let Some(active) = persisted.ui.active_worktree {
            // Translate the persisted path back to today's (repo, worktree)
            // indices. Identity is the path because worktrees may have been
            // added, removed, or reordered between sessions; if the path is
            // gone, leave the cursor at its default.
            let found = self.repos.iter().enumerate().find_map(|(i, repo)| {
                repo.worktrees
                    .iter()
                    .position(|wt| wt.path == active.path)
                    .map(|j| (i, j))
            });
            if let Some((r, w)) = found {
                self.ui.cursor = Some(SidebarCursor::Worktree {
                    repo: r,
                    worktree: w,
                });
            }
        }
    }

    pub fn to_persisted(&self) -> PersistedState {
        // Persist the cursor's worktree path (when it's on one) so the next
        // session can restore the same selection even if indices have
        // shifted.  Path is the only identity stable across re-lists.
        let active_worktree = match self.ui.cursor {
            Some(SidebarCursor::Worktree { repo, worktree }) => self
                .repos
                .get(repo)
                .and_then(|r| r.worktrees.get(worktree))
                .map(|wt| ActiveWorktreeId {
                    path: wt.path.clone(),
                }),
            _ => None,
        };
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
            if self.ui.is_expanded(&repo.root_path) {
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
        let path = self.repos[idx].root_path.clone();
        if self.ui.is_expanded(&path) {
            if !self.repos[idx].worktrees.is_empty() {
                self.ui.cursor = Some(SidebarCursor::Worktree {
                    repo: idx,
                    worktree: 0,
                });
            }
        } else {
            self.ui
                .expanded
                .insert(path.to_string_lossy().into_owned(), true);
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
                let path = self.repos[idx].root_path.clone();
                if self.ui.is_expanded(&path) {
                    self.ui
                        .expanded
                        .insert(path.to_string_lossy().into_owned(), false);
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
                // Acknowledge any pending bells in this worktree's
                // terminals — pressing Enter is the explicit "I'm looking
                // at this now" gesture, distinct from arrowing past it.
                if let Some(ts) = self.terminals.get(&(repo, worktree)) {
                    ts.clear_bells();
                }
            }
            SidebarCursor::Repo(idx) => {
                let path = self.repos[idx].root_path.clone();
                let expanded = self.ui.is_expanded(&path);
                self.ui
                    .expanded
                    .insert(path.to_string_lossy().into_owned(), !expanded);
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
pub fn expand_path(raw: &std::path::Path) -> Result<PathBuf> {
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
    let existing: std::collections::HashSet<&str> = repo
        .worktrees
        .iter()
        .filter_map(|wt| wt.branch_name())
        .collect();
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

/// Does at least one repo have a GitHub `origin` remote?  Used to decide
/// whether the "not authenticated" footer is actionable for this user.
pub fn any_github_remote(repos: &[Repo]) -> bool {
    repos
        .iter()
        .any(|r| crate::github::discover_owner_repo(&r.root_path).is_some())
}

fn load_repos(config: &Config) -> Vec<Repo> {
    let mut repos = Vec::with_capacity(config.repos.len());
    for repo_cfg in &config.repos {
        let worktrees = match git::list_worktrees(&repo_cfg.path) {
            Ok(list) => list,
            Err(err) => {
                crate::paths::log_warning(&format!("skipping repo {}: {err:#}", repo_cfg.name));
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
                            head: crate::model::HeadRef::Branch("main".to_string()),
                            path: PathBuf::from("/Users/sebas/dev/grove"),
                            is_primary: true,
                            pr: None,
                            status: None,
                        },
                        Worktree {
                            head: crate::model::HeadRef::Branch("feat/sidebar".to_string()),
                            path: PathBuf::from("/Users/sebas/dev/grove-feat-sidebar"),
                            is_primary: false,
                            pr: None,
                            status: None,
                        },
                        Worktree {
                            head: crate::model::HeadRef::Branch("fix/deps".to_string()),
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
                            head: crate::model::HeadRef::Branch("main".to_string()),
                            path: PathBuf::from("/Users/sebas/dotfiles"),
                            is_primary: true,
                            pr: None,
                            status: None,
                        },
                        Worktree {
                            head: crate::model::HeadRef::Branch("wip/zsh".to_string()),
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
            activity: {
                let mut a = ActivityState::default();
                a.resize_repos(2);
                a
            },
            github_authenticated: false,
            has_github_repo: false,
            config_error: None,
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
            activity: ActivityState::default(),
            github_authenticated: false,
            has_github_repo: false,
            config_error: None,
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
        assert_eq!(app.active_worktree_id(), Some((0, 0)));
        app.update(AppMessage::MoveCursor(Direction::Down));
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 0,
                worktree: 1
            })
        );
        assert_eq!(app.active_worktree_id(), Some((0, 1)));
    }

    #[test]
    fn moving_cursor_to_worktree_activates_it() {
        let mut app = AppState::fixture();
        // Cursor starts on a Repo node — no worktree shown in main pane.
        assert_eq!(app.active_worktree_id(), None);
        app.update(AppMessage::MoveCursor(Direction::Down));
        assert!(app.active_worktree_id().is_some());
    }

    #[test]
    fn j_skips_collapsed_repo_children() {
        let mut app = AppState::fixture();
        app.ui
            .expanded
            .insert("/Users/sebas/dev/grove".to_string(), false);
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
        assert!(!app.ui.is_expanded(Path::new("/Users/sebas/dev/grove")));
    }

    #[test]
    fn l_on_collapsed_repo_expands() {
        let mut app = AppState::fixture();
        app.ui
            .expanded
            .insert("/Users/sebas/dev/grove".to_string(), false);
        app.update(AppMessage::ExpandOrDescend);
        assert!(app.ui.is_expanded(Path::new("/Users/sebas/dev/grove")));
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
    fn cursor_on_worktree_is_the_active_worktree() {
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 2,
        });
        // Active worktree is derived from the cursor — no extra step.
        assert_eq!(app.active_worktree_id(), Some((0, 2)));
    }

    #[test]
    fn enter_on_repo_toggles_expansion() {
        let mut app = AppState::fixture();
        app.update(AppMessage::Activate);
        assert!(!app.ui.is_expanded(Path::new("/Users/sebas/dev/grove")));
        app.update(AppMessage::Activate);
        assert!(app.ui.is_expanded(Path::new("/Users/sebas/dev/grove")));
    }

    #[test]
    fn persisted_round_trip_restores_expanded_and_active() {
        let mut app = AppState::fixture();
        app.ui
            .expanded
            .insert("/Users/sebas/dotfiles".to_string(), false);
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 2,
        });

        let persisted = app.to_persisted();

        let mut restored = AppState::fixture();
        restored.apply_persisted(persisted);
        assert!(!restored.ui.is_expanded(Path::new("/Users/sebas/dotfiles")));
        assert_eq!(
            restored.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 0,
                worktree: 2
            })
        );
        assert_eq!(restored.active_worktree_id(), Some((0, 2)));
    }

    #[test]
    fn persisted_active_is_dropped_when_path_no_longer_exists() {
        let mut app = AppState::fixture();
        let persisted = PersistedState {
            schema_version: crate::state::current_schema_version(),
            ui: PersistedUi {
                active_worktree: Some(ActiveWorktreeId {
                    path: PathBuf::from("/Users/sebas/dev/grove-gone-worktree"),
                }),
                expanded: HashMap::new(),
            },
        };
        app.apply_persisted(persisted);
        // Cursor stays at its default (Repo(0)) — restoring a missing path
        // shouldn't surface a stale selection in the main pane.
        assert_eq!(app.active_worktree_id(), None);
    }

    #[test]
    fn active_worktree_survives_branch_switch_inside_worktree() {
        // Branch switches inside a worktree don't move it in the sidebar,
        // so the cursor (and therefore the main pane) stays put.
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        assert!(app.active_worktree_id().is_some());

        // Simulate the FS watcher re-listing after a branch switch by
        // mutating the head label in place.
        app.repos[0].worktrees[1].head =
            crate::model::HeadRef::Branch("now-on-some-other-branch".to_string());

        // Same cursor → same active worktree.
        assert_eq!(app.active_worktree_id(), Some((0, 1)));
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

        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        assert!(app.active_worktree_id().is_some());
        app.remove_repo(0).unwrap();
        // Removing the repo whose worktree was active must drop the active
        // worktree — cursor lands on the surviving repo's row, so the main
        // pane shows no worktree content.
        assert_eq!(app.active_worktree_id(), None);
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

    #[test]
    fn remap_worktree_state_preserves_views_across_reorder() {
        let mut app = AppState::fixture();
        let r = 0;

        // Assign distinct views to each worktree in repo 0.
        app.main_views.insert((r, 0), MainView::Terminal);
        app.main_views.insert((r, 1), MainView::Diff);
        app.main_views.insert((r, 2), MainView::Terminal);
        // Cursor on feat/sidebar (index 1).  After the swap below it should
        // follow the path to its new index 2, keeping the main pane in sync.
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: r,
            worktree: 1,
        });

        // Simulate a re-list that swaps indices 1 and 2 (e.g. a new worktree
        // was inserted before feat/sidebar alphabetically, pushing it back).
        let new_list = vec![
            app.repos[r].worktrees[0].clone(), // main stays at 0
            app.repos[r].worktrees[2].clone(), // fix/deps moves to 1
            app.repos[r].worktrees[1].clone(), // feat/sidebar moves to 2
        ];

        app.remap_worktree_state(r, &new_list);

        // main (path unchanged) stays at index 0.
        assert_eq!(app.main_views.get(&(r, 0)), Some(&MainView::Terminal));
        // feat/sidebar (was 1) is now at 2.
        assert_eq!(app.main_views.get(&(r, 2)), Some(&MainView::Diff));
        // fix/deps (was 2) is now at 1.
        assert_eq!(app.main_views.get(&(r, 1)), Some(&MainView::Terminal));
        // Cursor followed feat/sidebar's path to its new index.
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: r,
                worktree: 2
            })
        );
    }

    #[test]
    fn remap_worktree_state_does_not_affect_other_repos() {
        let mut app = AppState::fixture();

        app.main_views.insert((0, 0), MainView::Diff);
        app.main_views.insert((1, 0), MainView::Diff);
        // Cursor on a repo-1 worktree must not move when repo-0 is remapped.
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 1,
            worktree: 0,
        });

        // Remap repo 0 only; repo 1 entries must be untouched.
        let new_list = vec![app.repos[0].worktrees[0].clone()];
        app.remap_worktree_state(0, &new_list);

        assert_eq!(app.main_views.get(&(1, 0)), Some(&MainView::Diff));
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 1,
                worktree: 0
            })
        );
    }

    #[test]
    fn remap_worktree_state_drops_cursor_to_repo_when_list_empty() {
        // The repo loses all its worktrees externally.  Cursor must move
        // up to the parent repo so the main pane shows no worktree
        // content for one that no longer exists.
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        app.remap_worktree_state(0, &[]);
        assert_eq!(app.ui.cursor, Some(SidebarCursor::Repo(0)));
        assert_eq!(app.active_worktree_id(), None);
    }

    #[test]
    fn remap_worktree_state_falls_back_to_neighbor_when_cursor_path_gone() {
        // Cursor was on feat/sidebar (idx 1).  Re-list drops feat/sidebar
        // but keeps main and fix/deps.  Cursor must land on a survivor at
        // a similar index — never on a stale (repo, idx) pair.
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        let new_list = vec![
            app.repos[0].worktrees[0].clone(), // main
            app.repos[0].worktrees[2].clone(), // fix/deps
        ];
        app.remap_worktree_state(0, &new_list);
        assert_eq!(
            app.ui.cursor,
            Some(SidebarCursor::Worktree {
                repo: 0,
                worktree: 1
            })
        );
    }

    fn branch(name: &str, remote: Option<&str>) -> crate::git::BranchEntry {
        crate::git::BranchEntry {
            name: name.to_string(),
            remote: remote.map(str::to_string),
        }
    }

    #[test]
    fn new_worktree_modal_total_rows_is_create_plus_matches() {
        let m = NewWorktreeModal::for_repo(0, vec![branch("main", None), branch("feat/x", None)]);
        // Empty input → no filter → all branches match.
        assert_eq!(m.total_rows(), 3); // create + main + feat/x
        assert_eq!(m.cursor, 0);
    }

    #[test]
    fn recompute_filter_is_case_insensitive_substring_and_resets_cursor() {
        let mut m = NewWorktreeModal::for_repo(
            0,
            vec![
                branch("main", None),
                branch("feat/Search", None),
                branch("fix/typo", None),
                branch("search-ui", Some("origin")),
            ],
        );
        m.cursor = 2;
        for c in "search".chars() {
            m.input.insert_char(c);
        }
        m.recompute_filter();
        // Both "feat/Search" and "origin/search-ui" contain "search".
        assert_eq!(m.filter_matches.len(), 2);
        // Cursor resets so Enter creates a new branch named "search"
        // rather than picking one that happens to be on the cursor row.
        assert_eq!(m.cursor, 0);
    }

    #[test]
    fn selected_branch_returns_none_when_cursor_on_create_row() {
        let m = NewWorktreeModal::for_repo(0, vec![branch("main", None)]);
        assert_eq!(m.cursor, 0);
        assert!(m.selected_branch().is_none());
    }

    #[test]
    fn selected_branch_returns_entry_at_filter_cursor() {
        let mut m =
            NewWorktreeModal::for_repo(0, vec![branch("main", None), branch("feat/x", None)]);
        m.cursor = 2; // "feat/x" (row 1 is main, row 2 is feat/x)
        let got = m.selected_branch().unwrap();
        assert_eq!(got.name, "feat/x");
    }

    #[test]
    fn cycle_theme_persists_to_disk() {
        let tmp = temp_dir();
        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path.clone());
        assert_eq!(app.theme_name, crate::theme::ThemeName::default());

        app.update(AppMessage::CycleTheme);
        let after = app.theme_name;
        assert_ne!(after, crate::theme::ThemeName::default());
        assert!(config_path.exists(), "config file should have been written");

        let reloaded = crate::config::Config::load(&config_path).unwrap();
        assert_eq!(reloaded.theme.base, after);
    }

    #[test]
    fn try_add_repo_auto_detects_base_branch_from_origin_head() {
        let tmp = temp_dir();
        let bare = tmp.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["init", "--bare", "--quiet", "--initial-branch=develop"])
            .status()
            .unwrap();
        assert!(status.success());

        let local = tmp.join("local");
        std::fs::create_dir_all(&local).unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&local)
            .args(["init", "--quiet", "--initial-branch=develop"])
            .status()
            .unwrap();
        assert!(status.success());
        run_git(
            &local,
            &["remote", "add", "origin", &bare.display().to_string()],
        );
        std::fs::write(local.join("a.txt"), "a").unwrap();
        run_git(&local, &["add", "."]);
        run_git(
            &local,
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
        run_git(&local, &["push", "--quiet", "-u", "origin", "develop"]);
        // Set origin/HEAD so detect_default_branch picks it up.
        run_git(&local, &["remote", "set-head", "origin", "--auto"]);

        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path);
        app.try_add_repo(local.to_str().unwrap()).unwrap();
        assert_eq!(app.repos.len(), 1);
        assert_eq!(app.repos[0].base_branch, "develop");
        assert_eq!(
            app.config.repos[0].base_branch.as_deref(),
            Some("develop"),
            "persisted config should carry the detected base branch"
        );
    }

    #[test]
    fn try_add_repo_falls_back_to_default_when_no_origin_head() {
        let tmp = temp_dir();
        let repo = tmp.join("local");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);

        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path);
        app.try_add_repo(repo.to_str().unwrap()).unwrap();
        assert_eq!(app.repos[0].base_branch, "main");
        // Detected base is None → config stays null so the general default
        // can still override later.
        assert!(app.config.repos[0].base_branch.is_none());
    }

    fn make_scanning_modal(root: PathBuf) -> AppState {
        let config_path = PathBuf::from("/tmp/grove-scan-test-config.toml");
        let mut app = AppState::empty_fixture(config_path);
        app.ui.modal = Some(Modal::DiscoveredRepos(DiscoveredReposModal::scanning(root)));
        app
    }

    #[test]
    fn apply_scan_results_marks_already_configured() {
        let tmp = temp_dir();
        let existing = tmp.join("existing");
        let fresh = tmp.join("fresh");
        std::fs::create_dir_all(&existing).unwrap();
        std::fs::create_dir_all(&fresh).unwrap();
        init_git_repo(&existing);
        init_git_repo(&fresh);

        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path);
        app.try_add_repo(existing.to_str().unwrap()).unwrap();
        app.ui.modal = Some(Modal::DiscoveredRepos(DiscoveredReposModal::scanning(
            tmp.clone(),
        )));

        app.update(AppMessage::ScanReady(vec![existing.clone(), fresh.clone()]));

        let Some(Modal::DiscoveredRepos(m)) = &app.ui.modal else {
            panic!("expected DiscoveredRepos");
        };
        assert!(!m.scanning);
        assert_eq!(m.candidates.len(), 2);
        // Candidates are stored in canonical form; look them up the same way.
        let canonical_existing = std::fs::canonicalize(&existing).unwrap();
        let canonical_fresh = std::fs::canonicalize(&fresh).unwrap();
        let by_path: std::collections::HashMap<_, _> =
            m.candidates.iter().map(|c| (c.path.clone(), c)).collect();
        assert!(by_path[&canonical_existing].already_configured);
        assert!(!by_path[&canonical_existing].selected);
        assert!(!by_path[&canonical_fresh].already_configured);
        assert!(by_path[&canonical_fresh].selected);
    }

    #[test]
    fn toggle_discovered_selection_flips_cursor_row() {
        let mut app = make_scanning_modal(PathBuf::from("/tmp"));
        app.update(AppMessage::ScanReady(vec![
            PathBuf::from("/tmp/a"),
            PathBuf::from("/tmp/b"),
        ]));
        app.update(AppMessage::DiscoveredCursorDown);
        app.update(AppMessage::ToggleDiscoveredSelection);
        let Some(Modal::DiscoveredRepos(m)) = &app.ui.modal else {
            unreachable!()
        };
        assert!(m.candidates[0].selected); // default-on, untouched
        assert!(!m.candidates[1].selected); // was on (not configured), now off
    }

    #[test]
    fn submit_discovered_adds_only_selected_and_new_candidates() {
        let tmp = temp_dir();
        let a = tmp.join("a");
        let b = tmp.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        init_git_repo(&a);
        init_git_repo(&b);

        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path);
        app.ui.modal = Some(Modal::DiscoveredRepos(DiscoveredReposModal::scanning(
            tmp.clone(),
        )));
        app.update(AppMessage::ScanReady(vec![a.clone(), b.clone()]));

        // Deselect `b`.
        app.update(AppMessage::DiscoveredCursorDown);
        app.update(AppMessage::ToggleDiscoveredSelection);

        app.update(AppMessage::SubmitModal);
        assert_eq!(app.repos.len(), 1);
        // Both the stored repo and the test's `a` path go through the
        // same canonicalisation, so comparing canonical forms is robust
        // to macOS /private/var symlink expansion.
        let canonical_a = std::fs::canonicalize(&a).unwrap();
        assert_eq!(app.repos[0].root_path, canonical_a);
    }

    /// Helper: build a real repo with `main` and a feature branch, then
    /// register it in an `AppState` and cursor onto the feature worktree.
    /// Returns (app, feature_branch_name).
    fn setup_repo_with_feature_worktree(merged: bool) -> (AppState, String) {
        let tmp = temp_dir();
        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        // Starting on main with one commit.  Create feat/x off main.
        run_git(&repo, &["checkout", "-b", "feat/x"]);
        std::fs::write(repo.join("f.txt"), "f").unwrap();
        run_git(&repo, &["add", "."]);
        run_git(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "feat",
                "--quiet",
            ],
        );
        if merged {
            // Fast-forward main to feat/x so `feat/x` is fully merged.
            run_git(&repo, &["checkout", "main"]);
            run_git(&repo, &["merge", "--ff-only", "--quiet", "feat/x"]);
        } else {
            run_git(&repo, &["checkout", "main"]);
        }

        // Add a linked worktree for feat/x.
        let wt_path = tmp.join("repo-feat-x");
        run_git(
            &repo,
            &["worktree", "add", &wt_path.display().to_string(), "feat/x"],
        );

        let config_path = tmp.join("config.toml");
        let mut app = AppState::empty_fixture(config_path);
        app.try_add_repo(repo.to_str().unwrap()).unwrap();
        // Cursor onto the feat/x worktree.
        let feat_idx = app.repos[0]
            .worktrees
            .iter()
            .position(|w| w.branch_name() == Some("feat/x"))
            .unwrap();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: feat_idx,
        });
        (app, "feat/x".to_string())
    }

    #[test]
    fn remove_worktree_modal_opens_merged_variant_when_branch_is_merged() {
        let (mut app, _branch) = setup_repo_with_feature_worktree(true);
        app.update(AppMessage::OpenConfirmRemoveWorktree);
        let Some(Modal::ConfirmRemoveWorktree { variant, .. }) = &app.ui.modal else {
            panic!("expected ConfirmRemoveWorktree");
        };
        assert_eq!(*variant, DeleteVariant::Merged);
    }

    #[test]
    fn remove_worktree_modal_opens_unmerged_variant_when_branch_diverges() {
        let (mut app, _branch) = setup_repo_with_feature_worktree(false);
        app.update(AppMessage::OpenConfirmRemoveWorktree);
        let Some(Modal::ConfirmRemoveWorktree { variant, .. }) = &app.ui.modal else {
            panic!("expected ConfirmRemoveWorktree");
        };
        assert_eq!(*variant, DeleteVariant::Unmerged);
    }

    #[test]
    fn keep_branch_removes_worktree_but_keeps_branch() {
        let (mut app, branch) = setup_repo_with_feature_worktree(true);
        app.update(AppMessage::OpenConfirmRemoveWorktree);
        app.update(AppMessage::ConfirmWorktreeDeletion(
            DeleteChoice::KeepBranch,
        ));

        // Worktree gone.
        let wts = &app.repos[0].worktrees;
        assert!(
            wts.iter().all(|w| w.branch_name() != Some(branch.as_str())),
            "worktree should have been removed: {wts:?}"
        );
        // Branch still exists on disk.
        let branches = crate::git::list_branches(&app.repos[0].root_path).unwrap();
        assert!(branches
            .iter()
            .any(|b| b.name == branch && b.remote.is_none()));
    }

    #[test]
    fn regular_delete_succeeds_on_merged_branch() {
        let (mut app, branch) = setup_repo_with_feature_worktree(true);
        app.update(AppMessage::OpenConfirmRemoveWorktree);
        app.update(AppMessage::ConfirmWorktreeDeletion(DeleteChoice::Delete));

        let branches = crate::git::list_branches(&app.repos[0].root_path).unwrap();
        assert!(branches
            .iter()
            .all(|b| b.name != branch || b.remote.is_some()));
    }

    #[test]
    fn regular_delete_on_unmerged_shows_inline_error_and_keeps_modal_open() {
        let (mut app, branch) = setup_repo_with_feature_worktree(false);
        app.update(AppMessage::OpenConfirmRemoveWorktree);
        app.update(AppMessage::ConfirmWorktreeDeletion(DeleteChoice::Delete));

        // Modal stays open with an inline error pointing at `D`.
        let Some(Modal::ConfirmRemoveWorktree { error, .. }) = &app.ui.modal else {
            panic!("expected modal still open");
        };
        let msg = error.as_deref().unwrap_or("");
        assert!(msg.contains("force-delete"), "message: {msg}");
        assert!(msg.to_lowercase().contains("d"));
        // Branch still present.
        let branches = crate::git::list_branches(&app.repos[0].root_path).unwrap();
        assert!(branches
            .iter()
            .any(|b| b.name == branch && b.remote.is_none()));
    }

    #[test]
    fn force_delete_removes_unmerged_branch() {
        let (mut app, branch) = setup_repo_with_feature_worktree(false);
        app.update(AppMessage::OpenConfirmRemoveWorktree);
        app.update(AppMessage::ConfirmWorktreeDeletion(
            DeleteChoice::ForceDelete,
        ));

        let branches = crate::git::list_branches(&app.repos[0].root_path).unwrap();
        assert!(branches
            .iter()
            .all(|b| b.name != branch || b.remote.is_some()));
    }

    #[test]
    fn reconcile_worktrees_picks_up_branch_switch_in_terminal() {
        // Simulates the FS-watcher firing after a user runs `git switch` in
        // a terminal. The worktree's path is unchanged but its head moves
        // to a new branch — `reconcile_worktrees` should update the label
        // and keep the cursor (which the main pane derives from) intact.
        let (mut app, _) = setup_repo_with_feature_worktree(true);
        let feat_idx = app.repos[0]
            .worktrees
            .iter()
            .position(|w| w.branch_name() == Some("feat/x"))
            .unwrap();
        let wt_path = app.repos[0].worktrees[feat_idx].path.clone();

        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: feat_idx,
        });
        assert_eq!(app.active_worktree_id(), Some((0, feat_idx)));

        // Switch the branch *inside* the worktree.
        run_git(&wt_path, &["checkout", "-b", "feat/renamed"]);

        // The reconcile path mirrors what RepoDirty does on a real FS event.
        app.reconcile_worktrees(0);

        // Cursor remap follows the path → still active at the same index.
        assert_eq!(app.active_worktree_id(), Some((0, feat_idx)));
        // Label tracked the switch.
        assert_eq!(
            app.repos[0].worktrees[feat_idx].branch_name(),
            Some("feat/renamed"),
        );
    }

    /// A new branch created off local `main` is identical to `main` and
    /// must be reported as merged — even when `origin/main` is behind
    /// (e.g. local has unpushed commits).  Otherwise the user gets the
    /// "unmerged" warning the moment they create+delete a fresh worktree.
    #[test]
    fn fresh_branch_off_local_main_is_merged_when_origin_is_behind() {
        let tmp = temp_dir();
        let bare = tmp.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["init", "--bare", "--quiet", "--initial-branch=main"])
            .status()
            .unwrap();
        assert!(status.success());

        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        run_git(
            &repo,
            &["remote", "add", "origin", &bare.display().to_string()],
        );
        run_git(&repo, &["push", "--quiet", "-u", "origin", "main"]);
        // Land an extra commit on local main so it's ahead of origin/main.
        std::fs::write(repo.join("ahead.txt"), "ahead").unwrap();
        run_git(&repo, &["add", "."]);
        run_git(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "ahead",
                "--quiet",
            ],
        );
        // Cut a fresh branch off local main's current tip.
        run_git(&repo, &["branch", "fresh"]);

        assert!(
            crate::git::is_branch_merged(&repo, "fresh", "main"),
            "branch at the same tip as local main should be reported merged \
             even when origin/main is behind"
        );
    }

    /// `git branch -d` consults the branch's upstream when one is set.
    /// If the upstream is behind the branch, the CLI rejects the delete
    /// even when our `is_branch_merged` (which checks against base, not
    /// upstream) said it was safe.  Verify our libgit2-based delete
    /// succeeds in that exact case.
    #[test]
    fn delete_branch_succeeds_when_upstream_is_behind() {
        let tmp = temp_dir();
        let bare = tmp.join("remote.git");
        std::fs::create_dir_all(&bare).unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&bare)
            .args(["init", "--bare", "--quiet", "--initial-branch=main"])
            .status()
            .unwrap();
        assert!(status.success());

        let repo = tmp.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_git_repo(&repo);
        run_git(
            &repo,
            &["remote", "add", "origin", &bare.display().to_string()],
        );
        run_git(&repo, &["push", "--quiet", "-u", "origin", "main"]);
        // Local main lands a commit ahead of origin/main.
        std::fs::write(repo.join("ahead.txt"), "ahead").unwrap();
        run_git(&repo, &["add", "."]);
        run_git(
            &repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-m",
                "ahead",
                "--quiet",
            ],
        );
        // New branch off local main with an upstream pointing at origin/main
        // — exactly the config a user with `branch.autoSetupMerge=always`
        // would end up in.  `git branch -d fresh` would refuse here
        // because `fresh` is ahead of its tracked upstream.
        run_git(&repo, &["branch", "fresh"]);
        run_git(&repo, &["branch", "--set-upstream-to=origin/main", "fresh"]);

        crate::git::delete_branch(&repo, "fresh").expect("delete should succeed via libgit2");
        let branches = crate::git::list_branches(&repo).unwrap();
        assert!(branches
            .iter()
            .all(|b| b.name != "fresh" || b.remote.is_some()));
    }
}
