mod add_repo;
mod badges;
mod confirm;
pub mod diff;
mod help;
mod main_pane;
mod sidebar;
pub mod text_input;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::app::{AppState, Modal};

const SIDEBAR_WIDTH: u16 = 32;

pub struct RenderedLayout {
    pub sidebar: Rect,
    pub main_inner: Rect,
}

/// Render the whole frame and return layout rects (used by the caller to
/// sync PTY size on resize and to route mouse clicks).
pub fn render(frame: &mut Frame, app: &AppState) -> RenderedLayout {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
        .split(frame.area());

    sidebar::render(frame, chunks[0], app);
    let inner = main_pane::render(frame, chunks[1], app);

    if let Some(modal) = &app.ui.modal {
        match modal {
            Modal::Help => help::render(frame, frame.area(), app.ui.help_scroll),
            Modal::AddRepo(state) => add_repo::render(frame, frame.area(), state),
            Modal::NewWorktree(state) => {
                let repo_name = app
                    .repos
                    .get(state.repo_idx)
                    .map(|r| r.name.as_str())
                    .unwrap_or("?");
                add_repo::render_new_worktree(frame, frame.area(), state, repo_name);
            }
            Modal::ConfirmRemoveRepo { repo_idx } => {
                if let Some(repo) = app.repos.get(*repo_idx) {
                    confirm::render_remove_repo(frame, frame.area(), &repo.name);
                }
            }
            Modal::ConfirmRemoveWorktree { id } => {
                if let Some(wt) = app
                    .repos
                    .get(id.0)
                    .and_then(|r| r.worktrees.get(id.1))
                {
                    confirm::render_remove_worktree(frame, frame.area(), &wt.branch);
                }
            }
            Modal::ConfirmDeleteBranch { branch, pr_number, .. } => {
                confirm::render_delete_branch(frame, frame.area(), branch, *pr_number);
            }
            Modal::ForceDeleteBranch { branch, .. } => {
                confirm::render_force_delete_branch(frame, frame.area(), branch);
            }
        }
    }

    RenderedLayout {
        sidebar: chunks[0],
        main_inner: inner,
    }
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}
