use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::DiscoveredReposModal;
use crate::ui::{centered_rect, truncate_to_width, wrap_message};

const ERROR_MAX_LINES: usize = 3;

pub fn render(frame: &mut Frame, area: Rect, modal: &DiscoveredReposModal) {
    let modal_width: u16 = 72;
    let inner_width = (modal_width as usize).saturating_sub(2);
    let err_width = inner_width.saturating_sub(4);

    let err_lines: Vec<String> = modal
        .error
        .as_deref()
        .map(|m| wrap_message(m, err_width.max(1), ERROR_MAX_LINES))
        .unwrap_or_default();
    let err_rows = err_lines.len() as u16;

    let title_budget = (modal_width as usize).saturating_sub(4);
    let title = truncate_to_width(
        &format!(" Scan: {} ", modal.scan_root.display()),
        title_budget,
    );
    let visible = 10usize;
    // borders(2) + blank+header(2) + list(visible) + blank(1)
    // + err_rows + blank(1) + hint(1)
    let height = 2 + 2 + visible as u16 + 1 + err_rows + 1 + 1;
    let modal_area = centered_rect(modal_width, height, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let rows = Layout::vertical([
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // header
        Constraint::Length(visible as u16), // list
        Constraint::Length(1),              // blank
        Constraint::Length(err_rows),       // error (0 rows when no error)
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // hint
    ])
    .split(inner);

    if modal.scanning {
        frame.render_widget(
            Paragraph::new("  ⟳ Scanning for git repositories…"),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new(Line::styled(
                "  (Esc to cancel)",
                Style::default().fg(Color::DarkGray),
            )),
            rows[6],
        );
        return;
    }

    if modal.candidates.is_empty() {
        frame.render_widget(
            Paragraph::new("  No git repositories found below this path."),
            rows[1],
        );
        frame.render_widget(Paragraph::new("  Esc cancel"), rows[6]);
        return;
    }

    let selected_count = modal.candidates.iter().filter(|c| c.selected).count();
    let total = modal.candidates.len();
    let header = format!("  {selected_count} of {total} selected.  Space to toggle.");
    frame.render_widget(Paragraph::new(header), rows[1]);

    let scroll_offset = modal
        .cursor
        .saturating_sub(visible - 1)
        .min(modal.candidates.len().saturating_sub(visible));

    let items: Vec<ListItem> = modal
        .candidates
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible)
        .map(|(i, c)| {
            let is_cursor = i == modal.cursor;
            let marker = if is_cursor { "▶ " } else { "  " };
            let checkbox = if c.selected { "[x]" } else { "[ ]" };
            let dim = Style::default().fg(Color::DarkGray);
            let name_style = if c.already_configured {
                dim
            } else if is_cursor {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let tag = if c.already_configured {
                Span::styled(" [already configured]", dim)
            } else {
                Span::raw("")
            };
            ListItem::new(Line::from(vec![
                Span::raw(marker),
                Span::raw(checkbox),
                Span::raw(" "),
                Span::styled(c.name.clone(), name_style),
                Span::raw("  "),
                Span::styled(c.path.display().to_string(), dim),
                tag,
            ]))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(modal.cursor.saturating_sub(scroll_offset)));
    frame.render_stateful_widget(List::new(items), rows[2], &mut state);

    if !err_lines.is_empty() {
        let red = Style::default().fg(Color::Red);
        let lines: Vec<Line> = err_lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let prefix = if i == 0 { "  ! " } else { "    " };
                Line::styled(format!("{prefix}{l}"), red)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), rows[4]);
    }

    let hint = "  j/k navigate  ·  Space toggle  ·  Enter add selected  ·  Esc cancel";
    frame.render_widget(Paragraph::new(hint), rows[6]);
}
