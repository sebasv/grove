use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::LogModal;
use crate::ui::centered_rect;

pub fn render(frame: &mut Frame, area: Rect, modal: &LogModal) {
    // Take most of the available area — a log view earns the screen
    // real estate, especially when the user is hunting for an error.
    let w = area.width.saturating_sub(4).max(40);
    let h = area.height.saturating_sub(2).max(8);
    let modal_area = centered_rect(w, h, area);
    frame.render_widget(Clear, modal_area);

    let path_label = modal
        .source
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no log file)".to_string());
    let title = format!(" Log — {path_label} ");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner_h = modal_area.height.saturating_sub(2) as usize;

    let dim = Style::default().fg(Color::DarkGray);
    let block = if modal.lines.is_empty() {
        // Don't bother computing scroll indicators when there's nothing to
        // show; just render the empty-state hint inside the block.
        block
    } else {
        let mut b = block;
        if modal.scroll > 0 {
            b = b.title_top(Line::from(Span::styled(" ↑ ", dim)).right_aligned());
        }
        if modal.scroll + inner_h < modal.lines.len() {
            b = b.title_bottom(Line::from(Span::styled(" ↓ more ", dim)));
        }
        b.title_bottom(
            Line::from(Span::styled(
                " j/k scroll · g/G top/bottom · Esc close ",
                dim,
            ))
            .right_aligned(),
        )
    };

    let lines: Vec<Line> = if modal.lines.is_empty() {
        vec![
            Line::from(""),
            Line::from("  No log entries yet — grove logs background warnings here."),
        ]
    } else {
        modal
            .lines
            .iter()
            .skip(modal.scroll)
            .take(inner_h)
            .map(|l| Line::from(l.as_str()))
            .collect()
    };

    frame.render_widget(Paragraph::new(lines).block(block), modal_area);
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::app::{AppMessage, AppState, LogModal, Modal};

    fn render_with_modal(modal: LogModal) -> String {
        let mut app = AppState::fixture();
        app.ui.modal = Some(Modal::ViewLog(modal));
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                crate::ui::render(frame, &app);
            })
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn empty_log_shows_placeholder() {
        let out = render_with_modal(LogModal {
            lines: Vec::new(),
            scroll: 0,
            source: None,
        });
        assert!(out.contains("No log entries yet"), "out:\n{out}");
    }

    #[test]
    fn scroll_clamps_to_tail() {
        let mut app = AppState::fixture();
        app.ui.modal = Some(Modal::ViewLog(LogModal {
            lines: (0..100).map(|i| format!("entry-{i}")).collect(),
            scroll: 90,
            source: None,
        }));

        // Scrolling past the end stays clamped.
        for _ in 0..50 {
            app.update(AppMessage::LogScrollDown);
        }
        let Some(Modal::ViewLog(m)) = &app.ui.modal else {
            panic!("expected log modal");
        };
        assert_eq!(m.scroll, 99);

        // Scrolling above the start stays at 0.
        for _ in 0..200 {
            app.update(AppMessage::LogScrollUp);
        }
        let Some(Modal::ViewLog(m)) = &app.ui.modal else {
            unreachable!();
        };
        assert_eq!(m.scroll, 0);
    }
}
