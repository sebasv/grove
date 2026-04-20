use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, area: Rect) {
    let modal = centered_rect(54, 14, area);
    frame.render_widget(Clear, modal);

    let block = Block::default().borders(Borders::ALL).title(" Help ");
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(""),
        Line::styled("  Sidebar", bold),
        Line::from("    j / k             move up / down"),
        Line::from("    h / l             collapse / expand repo"),
        Line::from("    Enter             activate worktree"),
        Line::from(""),
        Line::styled("  Global", bold),
        Line::from("    ?                 toggle this help"),
        Line::from("    q                 quit"),
        Line::from(""),
        Line::from("  (Esc or ? to close)"),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), modal);
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::app::{AppMessage, AppState};

    #[test]
    fn modal_renders_over_app() {
        let mut app = AppState::fixture();
        app.update(AppMessage::ToggleHelp);
        let backend = TestBackend::new(70, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &app))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
