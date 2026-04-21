use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::AddRepoModal;
use crate::ui::{centered_rect, text_input};

pub fn render(frame: &mut Frame, area: Rect, modal: &AddRepoModal) {
    let modal_area = centered_rect(62, 11, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Add repository ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let rows = Layout::vertical([
        Constraint::Length(1), // blank
        Constraint::Length(1), // label
        Constraint::Length(1), // input
        Constraint::Length(1), // blank
        Constraint::Length(1), // error (rendered blank when None)
        Constraint::Length(1), // blank
        Constraint::Length(1), // hint
    ])
    .split(inner);

    frame.render_widget(Paragraph::new("  Path to git repository:"), rows[1]);

    let input_area = Rect {
        x: rows[2].x + 2,
        width: rows[2].width.saturating_sub(2),
        ..rows[2]
    };
    text_input::render(frame, input_area, &modal.input);

    if let Some(err) = &modal.error {
        let line = Line::styled(
            format!("  ! {err}"),
            Style::default().fg(Color::Red),
        );
        frame.render_widget(Paragraph::new(line), rows[4]);
    }

    frame.render_widget(
        Paragraph::new("  Enter to add  ·  Esc to cancel"),
        rows[6],
    );
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
    fn empty_modal_renders_with_prompt_and_hint() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn modal_shows_typed_path() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        for c in "/tmp/my-new-project".chars() {
            app.update(AppMessage::InputChar(c));
        }
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn modal_shows_error_after_invalid_submit() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        for c in "/definitely/not/a/real/path".chars() {
            app.update(AppMessage::InputChar(c));
        }
        app.update(AppMessage::SubmitModal);
        insta::assert_snapshot!(render_to_string(&app));
    }
}
