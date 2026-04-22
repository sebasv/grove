use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::centered_rect;

pub fn render_remove_repo(frame: &mut Frame, area: Rect, repo_name: &str) {
    let modal_area = centered_rect(58, 8, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Remove repository? ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(""),
        Line::from(format!("  Remove \"{repo_name}\" from grove?")),
        Line::from("  (config.toml will be rewritten;"),
        Line::from("   the repository on disk is not touched)"),
        Line::from(""),
        Line::from("  y/Enter confirm  ·  n/Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::app::{AppMessage, AppState};

    fn render_to_string(app: &AppState) -> String {
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                crate::ui::render(frame, app);
            })
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn confirm_remove_modal_names_the_repo() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenConfirmRemoveRepo);
        insta::assert_snapshot!(render_to_string(&app));
    }
}
