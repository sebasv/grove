mod help;
mod main_pane;
mod sidebar;
pub mod text_input;

use ratatui::layout::{Constraint, Direction, Layout};
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

    if let Some(Modal::Help) = app.ui.modal {
        help::render(frame, frame.area());
    }
}
