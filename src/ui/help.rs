use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::centered_rect;

pub fn render(frame: &mut Frame, area: Rect) {
    let modal = centered_rect(60, 32, area);
    frame.render_widget(Clear, modal);

    let block = Block::default().borders(Borders::ALL).title(" Help ");
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(""),
        Line::styled("  Sidebar", bold),
        Line::from("    j / k             move up / down"),
        Line::from("    h / l             collapse / expand repo"),
        Line::from("    Enter             activate worktree"),
        Line::from("    a                 add repository"),
        Line::from("    R                 remove repository"),
        Line::from("    r                 refresh git status"),
        Line::from(""),
        Line::styled("  Main (terminal)", bold),
        Line::from("    Enter             open terminal for worktree"),
        Line::from("    Ctrl+t            new tab"),
        Line::from("    Ctrl+w            close tab"),
        Line::from("    Alt+h / Alt+l     previous / next tab"),
        Line::from("    Ctrl+\\            toggle scrollback mode"),
        Line::from(""),
        Line::styled("  Diff (focus=Main)", bold),
        Line::from("    Ctrl+d            toggle diff view"),
        Line::from("    j / k             prev / next file"),
        Line::from("    Tab               list ↔ content"),
        Line::from("    J / K             scroll content"),
        Line::from("    s / u             stage / unstage"),
        Line::from("    m                 switch local ↔ branch mode"),
        Line::from(""),
        Line::styled("  Global", bold),
        Line::from("    Ctrl+Space        cycle focus (sidebar ↔ main)"),
        Line::from("    ?                 toggle this help"),
        Line::from("    q                 quit (sidebar focus only)"),
        Line::from(""),
        Line::styled("  PR badges", bold),
        Line::from("    set GITHUB_TOKEN, GH_TOKEN, or run 'gh auth login'"),
        Line::from(""),
        Line::from("  (Esc or ? to close)"),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), modal);
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
            .draw(|frame| {
                crate::ui::render(frame, &app);
            })
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
