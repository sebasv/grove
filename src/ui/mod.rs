mod add_repo;
mod confirm;
mod help;
mod main_pane;
mod sidebar;
pub mod text_input;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;

use crate::app::{AppState, Modal};

const SIDEBAR_WIDTH: u16 = 32;

pub fn render(frame: &mut Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
        .split(frame.area());

    sidebar::render(frame, chunks[0], app);
    main_pane::render(frame, chunks[1], app);

    if let Some(modal) = &app.ui.modal {
        match modal {
            Modal::Help => help::render(frame, frame.area()),
            Modal::AddRepo(state) => add_repo::render(frame, frame.area(), state),
            Modal::ConfirmRemoveRepo { repo_idx } => {
                if let Some(repo) = app.repos.get(*repo_idx) {
                    confirm::render_remove_repo(frame, frame.area(), &repo.name);
                }
            }
        }
    }
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}
