mod app;
mod async_evt;
mod config;
mod git;
mod github;
mod model;
mod paths;
mod keymap;
mod state;
mod terminal;
mod theme;
mod ui;

use std::io::{self, Stdout, Write};
use std::panic;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use portable_pty::PtySize;
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::app::{AppMessage, AppState, Direction, FocusZone, Modal, NewWorktreeModal};
use crate::async_evt::{Event, EventReceiver, EventSender, WorktreeId};
use crate::config::Config;
use crate::paths::AppPaths;

type Tui = Terminal<CrosstermBackend<Stdout>>;

enum InputAction {
    Message(AppMessage),
    PtyBytes(Vec<u8>),
    Noop,
}

#[derive(Parser)]
#[command(name = "grove", version, about)]
struct Cli {
    /// Override config file path
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Print resolved config, state and log paths, then exit
    #[arg(long)]
    print_paths: bool,

    /// Create an empty config file at the default location
    #[arg(long)]
    init: bool,
}

fn main() -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("error: failed to start async runtime: {err:#}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run_cli()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run_cli() -> Result<ExitCode> {
    let cli = Cli::parse();
    let paths = AppPaths::resolve()?;
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(|| paths.config_file.clone());

    if cli.print_paths {
        print_paths(&paths);
        return Ok(ExitCode::SUCCESS);
    }

    if cli.init {
        Config::write_template(&config_path)?;
        println!("Created config at {}", config_path.display());
        println!("Edit it to add your repositories, then run `grove`.");
        return Ok(ExitCode::SUCCESS);
    }

    let config = Config::load_or_default(&config_path)?;
    let theme_name = config.theme.base;
    if config.general.tmux_backing && !terminal::tmux_installed() {
        eprintln!(
            "warning: tmux_backing is enabled but `tmux` was not found on PATH; \
             falling back to direct shell spawns"
        );
    }
    let mut app = AppState::load(config, config_path.clone())?;
    app.theme_name = theme_name;
    app.theme = theme::resolve(theme_name);
    if let Some(persisted) = state::load(&paths.state_file)? {
        app.apply_persisted(persisted);
    }

    install_panic_hook();
    let mut tui = init_terminal()?;

    let (tx, rx) = async_evt::channel();
    async_evt::spawn_terminal_reader(tx.clone());
    let _watchers = spawn_watchers(&app, &tx);
    refresh_all_statuses(&app, &tx);

    let gh_client = github::build_client();
    if gh_client.is_some() {
        poll_github_prs(&app, &tx, gh_client.as_ref().unwrap());
    }

    let result = run(&mut tui, &mut app, rx, tx, gh_client).await;
    restore_terminal()?;
    result?;

    if let Err(err) = state::save(&app.to_persisted(), &paths.state_file) {
        eprintln!("warning: failed to save state: {err:#}");
    }
    Ok(ExitCode::SUCCESS)
}

async fn run(
    tui: &mut Tui,
    app: &mut AppState,
    mut rx: EventReceiver,
    tx: EventSender,
    gh_client: Option<std::sync::Arc<octocrab::Octocrab>>,
) -> Result<()> {
    let mut poll_interval = tokio::time::interval(std::time::Duration::from_secs(30));
    poll_interval.tick().await; // consume the immediate tick

    loop {
        let main_pane_size = draw(tui, app)?;
        resize_active_terminal(app, main_pane_size);

        let event = tokio::select! {
            ev = rx.recv() => match ev {
                Some(e) => e,
                None => break,
            },
            _ = poll_interval.tick() => {
                if let Some(client) = &gh_client {
                    poll_github_prs(app, &tx, client);
                }
                continue;
            }
        };
        match event {
            Event::Input(key) => {
                handle_input(key, app, &tx);
                if app.should_quit {
                    break;
                }
            }
            Event::Mouse(mouse) => {
                handle_mouse(mouse, app, &tx);
            }
            Event::RepoDirty(repo_idx) => {
                refresh_repo_statuses(repo_idx, app, &tx);
                // Re-trigger diff refresh for the active worktree if it's in
                // this repo and currently viewing the Diff pane.
                if let Some(id) = app.ui.active_worktree {
                    if id.0 == repo_idx
                        && app.main_views.get(&id).copied().unwrap_or_default()
                            == crate::app::MainView::Diff
                    {
                        refresh_diff_for_active(app, &tx);
                    }
                }
            }
            Event::StatusReady(id, status) => {
                app.set_worktree_status(id, status);
            }
            Event::DiffReady(id, files) => {
                app.set_diff(id, files);
            }
            Event::PrStatusReady(id, pr) => {
                app.set_worktree_pr(id, pr);
            }
            Event::TerminalOutput => {
                // Parser advanced; next draw picks it up.
            }
        }
    }
    Ok(())
}

fn draw(tui: &mut Tui, app: &mut AppState) -> Result<PtySize> {
    let mut main_size = PtySize::default();
    let mut layout_out = ui::RenderedLayout {
        sidebar: ratatui::layout::Rect::default(),
        main_inner: ratatui::layout::Rect::default(),
        tab_bar: None,
    };
    tui.draw(|frame| {
        let layout = ui::render(frame, app);
        main_size = PtySize {
            rows: layout.main_inner.height,
            cols: layout.main_inner.width,
            pixel_width: 0,
            pixel_height: 0,
        };
        layout_out = layout;
    })?;
    app.layout.sidebar = layout_out.sidebar;
    app.layout.main = layout_out.main_inner;
    app.layout.tab_bar = layout_out.tab_bar;
    Ok(main_size)
}

fn resize_active_terminal(app: &mut AppState, size: PtySize) {
    if size.rows == 0 || size.cols == 0 {
        return;
    }
    let Some(id) = app.ui.active_worktree else {
        return;
    };
    if let Some(ts) = app.terminals.get_mut(&id) {
        if let Some(term) = ts.active_mut() {
            let _ = term.resize(size);
        }
    }
}

fn handle_mouse(mouse: MouseEvent, app: &mut AppState, _tx: &EventSender) {
    // Scroll events are allowed through when the help overlay is open so the
    // user can wheel through it.  All other mouse events are swallowed while
    // any modal is visible to keep keyboard-only UX for inputs.
    let help_open = matches!(app.ui.modal, Some(Modal::Help));
    if app.ui.modal.is_some() && !help_open {
        return;
    }
    if help_open {
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                app.update(AppMessage::HelpScrollUp);
            }
            MouseEventKind::ScrollDown => {
                app.update(AppMessage::HelpScrollDown);
            }
            _ => {}
        }
        return;
    }
    let (col, row) = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if rect_contains(app.layout.sidebar, col, row) {
                app.ui.focus = crate::app::FocusZone::Sidebar;
                route_sidebar_click(app, row);
            } else if let Some(bar) = app.layout.tab_bar {
                if rect_contains(bar, col, row) {
                    app.ui.focus = crate::app::FocusZone::Main;
                    route_tab_click(app, col, bar);
                } else if rect_contains(app.layout.main, col, row) {
                    app.ui.focus = crate::app::FocusZone::Main;
                }
            } else if rect_contains(app.layout.main, col, row) {
                app.ui.focus = crate::app::FocusZone::Main;
            }
        }
        MouseEventKind::ScrollUp => {
            if rect_contains(app.layout.main, col, row) {
                if let Some(id) = app.ui.active_worktree {
                    if let Some(ts) = app.terminals.get_mut(&id) {
                        if ts.mode != crate::app::TerminalMode::Scrollback {
                            ts.mode = crate::app::TerminalMode::Scrollback;
                        }
                        ts.scroll(3);
                    }
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if rect_contains(app.layout.main, col, row) {
                if let Some(id) = app.ui.active_worktree {
                    if let Some(ts) = app.terminals.get_mut(&id) {
                        ts.scroll(-3);
                    }
                }
            }
        }
        _ => {}
    }
}

fn rect_contains(rect: ratatui::layout::Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x + rect.width
        && row >= rect.y
        && row < rect.y + rect.height
}

fn route_tab_click(app: &mut AppState, col: u16, bar: ratatui::layout::Rect) {
    let Some(id) = app.ui.active_worktree else {
        return;
    };
    let Some(ts) = app.terminals.get_mut(&id) else {
        return;
    };
    if col < bar.x {
        return;
    }
    let offset = col - bar.x;
    let mut x: u16 = 0;
    for i in 0..ts.list.len() {
        let is_active = i == ts.active;
        let mode_hint_len: u16 = if is_active && ts.mode == crate::app::TerminalMode::Scrollback {
            2
        } else {
            0
        };
        // label: " {marker}{tab_num}{mode_hint} " = 1 + 1 + digits + mode_hint + 1
        let digits = (i + 1).to_string().len() as u16;
        let label_width = 2 + digits + mode_hint_len + 1;
        let slot_width = label_width + 1; // label + separator space
        if offset >= x && offset < x + slot_width {
            ts.active = i;
            ts.mode = crate::app::TerminalMode::Insert;
            ts.scroll_offset = 0;
            return;
        }
        x += slot_width;
    }
}

fn route_sidebar_click(app: &mut AppState, row: u16) {
    // Reconstruct the visible sidebar flat list (matches sidebar::render).
    // 2 rows of header, then per-repo block.
    let base_row = app.layout.sidebar.y;
    if row < base_row.saturating_add(2) {
        return;
    }
    let mut line = base_row + 2; // first content row
    if app.repos.is_empty() {
        return;
    }
    for (i, repo) in app.repos.iter().enumerate() {
        // repo row
        if line == row {
            app.ui.cursor = Some(crate::app::SidebarCursor::Repo(i));
            return;
        }
        line += 1;
        let expanded = app.ui.is_expanded(&repo.name);
        if expanded {
            for (j, _wt) in repo.worktrees.iter().enumerate() {
                if line == row {
                    app.ui.cursor = Some(crate::app::SidebarCursor::Worktree {
                        repo: i,
                        worktree: j,
                    });
                    app.update(crate::app::AppMessage::Activate);
                    return;
                }
                line += 1;
            }
        }
        line += 1; // blank spacer row
    }
}

fn handle_input(key: KeyEvent, app: &mut AppState, tx: &EventSender) {
    let action = key_to_action(key, app);
    match action {
        InputAction::Message(msg) => {
            let is_activate = matches!(msg, AppMessage::Activate);
            let is_refresh = matches!(msg, AppMessage::RefreshAll);
            let is_new_terminal = matches!(msg, AppMessage::NewTerminal);
            let is_toggle_diff = matches!(msg, AppMessage::ToggleDiffView);
            let is_toggle_diff_mode = matches!(msg, AppMessage::ToggleDiffMode);
            let is_stage = matches!(msg, AppMessage::StageFocused);
            let is_unstage = matches!(msg, AppMessage::UnstageFocused);
            if is_refresh {
                refresh_all_statuses(app, tx);
            } else if is_stage || is_unstage {
                run_stage(app, tx, is_stage);
            } else {
                app.update(msg);
            }
            if is_activate {
                ensure_terminal_for_active(app, tx);
            }
            if is_new_terminal {
                if let Some(id) = app.ui.active_worktree {
                    spawn_terminal_for(id, app, tx);
                }
            }
            if is_toggle_diff || is_toggle_diff_mode {
                refresh_diff_for_active(app, tx);
            }
        }
        InputAction::PtyBytes(bytes) => {
            if let Some(id) = app.ui.active_worktree {
                if let Some(ts) = app.terminals.get_mut(&id) {
                    if let Some(term) = ts.active_mut() {
                        let _ = term.write(&bytes);
                    }
                }
            }
        }
        InputAction::Noop => {}
    }
}

fn ensure_terminal_for_active(app: &mut AppState, tx: &EventSender) {
    let Some(id) = app.ui.active_worktree else {
        return;
    };
    if app.terminals.contains_key(&id) {
        app.ui.focus = FocusZone::Main;
        return;
    }
    spawn_terminal_for(id, app, tx);
}

fn log_to_file(msg: &str) {
    if let Ok(paths) = AppPaths::resolve() {
        if let Some(parent) = paths.log_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&paths.log_file)
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

fn spawn_terminal_for(id: WorktreeId, app: &mut AppState, tx: &EventSender) {
    let Some(wt) = app
        .repos
        .get(id.0)
        .and_then(|repo| repo.worktrees.get(id.1))
    else {
        return;
    };
    let size = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };
    let tmux_session = if app.config.general.tmux_backing && terminal::tmux_installed() {
        Some(terminal::tmux_session_for(&wt.path))
    } else {
        None
    };
    let term = match terminal::Terminal::spawn(&wt.path, size, id, tx.clone(), tmux_session) {
        Ok(t) => t,
        Err(err) => {
            log_to_file(&format!(
                "terminal spawn failed for worktree ({}, {}) at {}: {err:#}",
                id.0,
                id.1,
                wt.path.display()
            ));
            return;
        }
    };
    if let Some(ts) = app.terminals.get_mut(&id) {
        ts.push(term);
    } else {
        app.terminals
            .insert(id, crate::app::WorktreeTerminals::new(term));
    }
    app.ui.focus = FocusZone::Main;
}

fn key_to_action(key: KeyEvent, app: &AppState) -> InputAction {
    if key.kind != KeyEventKind::Press {
        return InputAction::Noop;
    }

    // Ctrl+Space cycles focus from any context except when typing into a modal
    // (modals own all input to stay predictable).
    if app.ui.modal.is_none()
        && key.code == KeyCode::Char(' ')
        && key.modifiers.contains(KeyModifiers::CONTROL)
    {
        return InputAction::Message(AppMessage::CycleFocus);
    }

    // F2 cycles the color scheme globally — intercepted before PTY dispatch so
    // it works from both sidebar and terminal focus without sending an escape
    // sequence to the shell.
    if app.ui.modal.is_none() && key.code == KeyCode::F(2) {
        return InputAction::Message(AppMessage::CycleTheme);
    }

    if let Some(modal) = app.ui.modal.as_ref() {
        return InputAction::Message(match modal {
            Modal::Help => help_keys(key),
            Modal::AddRepo(_) => add_repo_keys(key),
            Modal::NewWorktree(m) => new_worktree_keys(key, m),
            Modal::ConfirmRemoveRepo { .. }
            | Modal::ConfirmRemoveWorktree { .. }
            | Modal::ConfirmDeleteBranch { .. }
            | Modal::ForceDeleteBranch { .. } => confirm_keys(key),
        });
    }

    match app.ui.focus {
        FocusZone::Sidebar => InputAction::Message(default_keys(key, &app.config.keys)),
        FocusZone::Main => main_focus_action(key, app),
    }
}

fn main_focus_action(key: KeyEvent, app: &AppState) -> InputAction {
    // Reserved grove keys in Main — active in both Insert and Scrollback modes.
    if let Some(msg) = main_reserved_keys(key) {
        return InputAction::Message(msg);
    }

    let Some(id) = app.ui.active_worktree else {
        return InputAction::Noop;
    };

    let view = app
        .main_views
        .get(&id)
        .copied()
        .unwrap_or_default();
    if view == crate::app::MainView::Diff {
        return InputAction::Message(diff_keys(key));
    }

    let Some(ts) = app.terminals.get(&id) else {
        return InputAction::Noop;
    };

    match ts.mode {
        crate::app::TerminalMode::Scrollback => {
            InputAction::Message(scrollback_keys(key))
        }
        crate::app::TerminalMode::Insert => match terminal::key_to_pty_bytes(key) {
            Some(bytes) => InputAction::PtyBytes(bytes),
            None => InputAction::Noop,
        },
    }
}

fn diff_keys(key: KeyEvent) -> AppMessage {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => AppMessage::DiffCursorDown,
        KeyCode::Char('k') | KeyCode::Up => AppMessage::DiffCursorUp,
        KeyCode::Char('J') => AppMessage::DiffContentDown,
        KeyCode::Char('K') => AppMessage::DiffContentUp,
        KeyCode::Tab => AppMessage::DiffToggleFocus,
        KeyCode::Char('s') => AppMessage::StageFocused,
        KeyCode::Char('u') => AppMessage::UnstageFocused,
        KeyCode::Char('m') => AppMessage::ToggleDiffMode,
        KeyCode::Esc => AppMessage::ToggleDiffView,
        _ => AppMessage::NoOp,
    }
}

fn main_reserved_keys(key: KeyEvent) -> Option<AppMessage> {
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl+h/l and Ctrl+←/→ switch terminal tabs.  Intercepted before PTY
    // dispatch so Ctrl+H doesn't reach the shell as backspace and Ctrl+L
    // doesn't clear the screen.
    if ctrl {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => return Some(AppMessage::PrevTab),
            KeyCode::Char('l') | KeyCode::Right => return Some(AppMessage::NextTab),
            _ => {}
        }
    }

    // Ctrl+\ toggles scrollback; note that Ctrl+[ would collide with Esc.
    if ctrl && matches!(key.code, KeyCode::Char('\\')) {
        return Some(AppMessage::ToggleScrollback);
    }
    // Ctrl+t: new terminal tab (Ctrl+W: close). Ctrl beats Alt here because
    // macOS terminals often swallow Option combos or remap them to Unicode.
    if ctrl && matches!(key.code, KeyCode::Char('t')) {
        return Some(AppMessage::NewTerminal);
    }
    if ctrl && matches!(key.code, KeyCode::Char('w')) {
        return Some(AppMessage::CloseTerminal);
    }
    // Ctrl+d: toggle diff view. Must be intercepted here so it doesn't reach
    // the PTY as EOF (which kills the shell).
    if ctrl && matches!(key.code, KeyCode::Char('d')) {
        return Some(AppMessage::ToggleDiffView);
    }
    if alt {
        return match key.code {
            KeyCode::Char('t') => Some(AppMessage::NewTerminal),
            KeyCode::Char('w') => Some(AppMessage::CloseTerminal),
            KeyCode::Char('h') | KeyCode::Left => Some(AppMessage::PrevTab),
            KeyCode::Char('l') | KeyCode::Right => Some(AppMessage::NextTab),
            KeyCode::Char('d') => Some(AppMessage::ToggleDiffView),
            _ => None,
        };
    }
    // macOS terminals that don't pass Option as Meta send Unicode characters
    // instead: Option+t → '†', Option+w → '∑', Option+h → '˙', Option+l → '¬'.
    // Map these to the same actions so the shortcuts work with default terminal
    // settings on macOS.
    if key.modifiers.is_empty() {
        return match key.code {
            KeyCode::Char('†') => Some(AppMessage::NewTerminal),
            KeyCode::Char('∑') => Some(AppMessage::CloseTerminal),
            KeyCode::Char('˙') => Some(AppMessage::PrevTab),
            KeyCode::Char('¬') => Some(AppMessage::NextTab),
            // Option+d on macOS sends '∂' (U+2202) when Option is not passed as Meta.
            KeyCode::Char('∂') => Some(AppMessage::ToggleDiffView),
            _ => None,
        };
    }
    None
}

fn scrollback_keys(key: KeyEvent) -> AppMessage {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => AppMessage::ScrollDown,
        KeyCode::Char('k') | KeyCode::Up => AppMessage::ScrollUp,
        KeyCode::PageDown => AppMessage::ScrollPageDown,
        KeyCode::PageUp => AppMessage::ScrollPageUp,
        KeyCode::Char('g') => AppMessage::ScrollTop,
        KeyCode::Char('G') => AppMessage::ScrollBottom,
        KeyCode::Esc | KeyCode::Char('i') | KeyCode::Char('q') => AppMessage::ToggleScrollback,
        _ => AppMessage::NoOp,
    }
}

fn spawn_watchers(app: &AppState, tx: &EventSender) -> Vec<async_evt::RepoWatcher> {
    let mut watchers = Vec::new();
    for (i, repo) in app.repos.iter().enumerate() {
        match async_evt::spawn_repo_watcher(i, repo.root_path.clone(), tx.clone()) {
            Ok(w) => watchers.push(w),
            Err(err) => {
                eprintln!("warning: watch failed for {}: {err:#}", repo.name);
            }
        }
    }
    watchers
}

fn poll_github_prs(
    app: &AppState,
    tx: &EventSender,
    client: &std::sync::Arc<octocrab::Octocrab>,
) {
    for (r, repo) in app.repos.iter().enumerate() {
        let Some(owner_repo) = github::discover_owner_repo(&repo.root_path) else {
            continue;
        };
        for (w, wt) in repo.worktrees.iter().enumerate() {
            github::spawn_pr_fetch(
                client.clone(),
                owner_repo.clone(),
                wt.branch.clone(),
                (r, w),
                tx.clone(),
            );
        }
    }
}

fn refresh_all_statuses(app: &AppState, tx: &EventSender) {
    for (r, repo) in app.repos.iter().enumerate() {
        for (w, wt) in repo.worktrees.iter().enumerate() {
            async_evt::spawn_status_refresh((r, w), wt.path.clone(), tx.clone());
        }
    }
}

fn refresh_diff_for_active(app: &AppState, tx: &EventSender) {
    let Some(id) = app.ui.active_worktree else {
        return;
    };
    let Some(repo) = app.repos.get(id.0) else {
        return;
    };
    let Some(wt) = repo.worktrees.get(id.1) else {
        return;
    };
    let mode = app.active_diff_mode();
    let base = repo.base_branch.clone();
    async_evt::spawn_diff_refresh(id, wt.path.clone(), mode, base, tx.clone());
}

fn run_stage(app: &mut AppState, tx: &EventSender, staging: bool) {
    // Stage/unstage only makes sense in Local mode.
    if app.active_diff_mode() != crate::app::DiffMode::Local {
        return;
    }
    let Some(id) = app.ui.active_worktree else {
        return;
    };
    let Some((worktree_path, file_path, is_staged, base)) = app.diffs.get(&id).and_then(|d| {
        let file = d.files.get(d.cursor)?;
        let repo = app.repos.get(id.0)?;
        let wt = repo.worktrees.get(id.1)?;
        Some((
            wt.path.clone(),
            file.path.clone(),
            file.staged,
            repo.base_branch.clone(),
        ))
    }) else {
        return;
    };
    // stage == true: only act when the focused file is unstaged, and vice versa.
    let result = match (staging, is_staged) {
        (true, false) => git::stage_path(&worktree_path, &file_path),
        (false, true) => git::unstage_path(&worktree_path, &file_path),
        _ => return,
    };
    if result.is_ok() {
        async_evt::spawn_diff_refresh(
            id,
            worktree_path,
            crate::app::DiffMode::Local,
            base,
            tx.clone(),
        );
    }
}

fn refresh_repo_statuses(repo_idx: usize, app: &AppState, tx: &EventSender) {
    let Some(repo) = app.repos.get(repo_idx) else {
        return;
    };
    for (w, wt) in repo.worktrees.iter().enumerate() {
        async_evt::spawn_status_refresh((repo_idx, w), wt.path.clone(), tx.clone());
    }
}

fn default_keys(key: KeyEvent, km: &keymap::Keymap) -> AppMessage {
    // Configurable actions take precedence.
    if km.quit.matches(&key) {
        return AppMessage::Quit;
    }
    if km.help.matches(&key) {
        return AppMessage::ToggleHelp;
    }
    if km.refresh.matches(&key) {
        return AppMessage::RefreshAll;
    }
    if km.add_repo.matches(&key) {
        return AppMessage::OpenAddRepo;
    }
    if km.remove_repo.matches(&key) {
        return AppMessage::OpenConfirmRemoveRepo;
    }
    if km.new_worktree.matches(&key) {
        return AppMessage::OpenNewWorktree;
    }
    if km.remove_worktree.matches(&key) {
        return AppMessage::OpenConfirmRemoveWorktree;
    }
    // Fixed navigation bindings (not yet configurable).
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => AppMessage::MoveCursor(Direction::Down),
        KeyCode::Char('k') | KeyCode::Up => AppMessage::MoveCursor(Direction::Up),
        KeyCode::Char('h') | KeyCode::Left => AppMessage::CollapseOrAscend,
        KeyCode::Char('l') | KeyCode::Right => AppMessage::ExpandOrDescend,
        KeyCode::Enter => AppMessage::Activate,
        _ => AppMessage::NoOp,
    }
}

fn help_keys(key: KeyEvent) -> AppMessage {
    match key.code {
        KeyCode::Char('?') | KeyCode::Esc => AppMessage::CloseModal,
        KeyCode::Char('j') | KeyCode::Down => AppMessage::HelpScrollDown,
        KeyCode::Char('k') | KeyCode::Up => AppMessage::HelpScrollUp,
        _ => AppMessage::NoOp,
    }
}

fn add_repo_keys(key: KeyEvent) -> AppMessage {
    match key.code {
        KeyCode::Esc => AppMessage::CloseModal,
        KeyCode::Enter => AppMessage::SubmitModal,
        KeyCode::Up => AppMessage::CompletionUp,
        KeyCode::Down => AppMessage::CompletionDown,
        KeyCode::Tab => AppMessage::CompletionAccept,
        KeyCode::Backspace => AppMessage::InputBackspace,
        KeyCode::Delete => AppMessage::InputDelete,
        KeyCode::Left => AppMessage::InputCursorLeft,
        KeyCode::Right => AppMessage::InputCursorRight,
        KeyCode::Home => AppMessage::InputHome,
        KeyCode::End => AppMessage::InputEnd,
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            AppMessage::InputChar(c)
        }
        _ => AppMessage::NoOp,
    }
}

fn new_worktree_keys(key: KeyEvent, modal: &NewWorktreeModal) -> AppMessage {
    use crate::app::NewWorktreeMode;
    match modal.mode {
        NewWorktreeMode::PickBranch => match key.code {
            KeyCode::Char('j') | KeyCode::Down => AppMessage::BranchCursorDown,
            KeyCode::Char('k') | KeyCode::Up => AppMessage::BranchCursorUp,
            KeyCode::Enter => AppMessage::SubmitModal,
            KeyCode::Tab => AppMessage::SwitchWorktreeMode,
            KeyCode::Esc => AppMessage::CloseModal,
            _ => AppMessage::NoOp,
        },
        NewWorktreeMode::NewBranch => match key.code {
            KeyCode::Esc => AppMessage::CloseModal,
            KeyCode::Enter => AppMessage::SubmitModal,
            KeyCode::Tab if !modal.branches.is_empty() => AppMessage::SwitchWorktreeMode,
            KeyCode::Backspace => AppMessage::InputBackspace,
            KeyCode::Delete => AppMessage::InputDelete,
            KeyCode::Left => AppMessage::InputCursorLeft,
            KeyCode::Right => AppMessage::InputCursorRight,
            KeyCode::Home => AppMessage::InputHome,
            KeyCode::End => AppMessage::InputEnd,
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                AppMessage::InputChar(c)
            }
            _ => AppMessage::NoOp,
        },
    }
}

fn confirm_keys(key: KeyEvent) -> AppMessage {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => AppMessage::SubmitModal,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => AppMessage::CloseModal,
        _ => AppMessage::NoOp,
    }
}

#[allow(dead_code)] // kept for potential future direct use
fn _silence(_: WorktreeId) {}

fn print_paths(paths: &AppPaths) {
    println!("config: {}", paths.config_file.display());
    println!("state:  {}", paths.state_file.display());
    println!("log:    {}", paths.log_file.display());
}

fn init_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(io::stdout()))?)
}

fn restore_terminal() -> Result<()> {
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn install_panic_hook() {
    let original = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original(info);
    }));
}
