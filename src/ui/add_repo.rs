use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{AddRepoModal, NewWorktreeModal};
use crate::ui::{centered_rect, text_input};

pub fn render(frame: &mut Frame, area: Rect, modal: &AddRepoModal) {
    let n = modal.completions.len().min(10);
    let height = if n == 0 { 11u16 } else { 11 + n as u16 + 1 };
    let modal_area = centered_rect(62, height, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Add repository ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut constraints = vec![
        Constraint::Length(1), // blank
        Constraint::Length(1), // label
        Constraint::Length(1), // input
    ];
    if n > 0 {
        constraints.push(Constraint::Length(1)); // separator
        constraints.push(Constraint::Length(n as u16)); // completions
    }
    constraints.push(Constraint::Length(1)); // blank / error gap
    constraints.push(Constraint::Length(1)); // error
    constraints.push(Constraint::Length(1)); // blank
    constraints.push(Constraint::Length(1)); // hint

    let rows = Layout::vertical(constraints).split(inner);

    frame.render_widget(Paragraph::new("  Path to git repository:"), rows[1]);

    let input_area = Rect {
        x: rows[2].x + 2,
        width: rows[2].width.saturating_sub(2),
        ..rows[2]
    };
    text_input::render(frame, input_area, &modal.input);

    let base_row = if n > 0 {
        // rows[3] = separator line, rows[4] = completions list
        frame.render_widget(
            Paragraph::new(Line::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(Color::DarkGray),
            )),
            rows[3],
        );

        let items: Vec<ListItem> = modal
            .completions
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let selected = modal.completion_cursor == Some(i);
                let style = if selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                ListItem::new(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(name.clone(), style),
                ]))
            })
            .collect();
        let mut state = ListState::default();
        state.select(modal.completion_cursor);
        frame.render_stateful_widget(List::new(items), rows[4], &mut state);
        5 // base_row index for the gap after completions
    } else {
        3
    };

    if let Some(err) = &modal.error {
        let line = Line::styled(format!("  ! {err}"), Style::default().fg(Color::Red));
        frame.render_widget(Paragraph::new(line), rows[base_row + 1]);
    }

    let hint = if n > 0 {
        "  ↑/↓ navigate  ·  Tab select  ·  Enter add  ·  Esc cancel"
    } else {
        "  Enter to add  ·  Esc to cancel"
    };
    frame.render_widget(Paragraph::new(hint), rows[base_row + 3]);
}

pub fn render_new_worktree(
    frame: &mut Frame,
    area: Rect,
    modal: &NewWorktreeModal,
    repo_name: &str,
) {
    let visible = 10usize;
    // input (1) + blank (1) + list (visible) + blank (1) + error (1) + blank (1) + hint (1) + padding
    let height = (visible + 9) as u16;
    let modal_area = centered_rect(70, height, area);
    frame.render_widget(Clear, modal_area);

    let title = format!(" New worktree in {repo_name} ");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let rows = Layout::vertical([
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // "Filter / new branch:"
        Constraint::Length(1),              // input
        Constraint::Length(1),              // separator
        Constraint::Length(visible as u16), // list
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // error
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // hint
    ])
    .split(inner);

    frame.render_widget(Paragraph::new("  Filter / new branch name:"), rows[1]);

    let input_area = Rect {
        x: rows[2].x + 2,
        width: rows[2].width.saturating_sub(2),
        ..rows[2]
    };
    text_input::render(frame, input_area, &modal.input);

    frame.render_widget(
        Paragraph::new(Line::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(Color::DarkGray),
        )),
        rows[3],
    );

    render_row_list(frame, rows[4], modal, visible);

    if let Some(err) = &modal.error {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("  ! {err}"),
                Style::default().fg(Color::Red),
            )),
            rows[6],
        );
    }

    frame.render_widget(
        Paragraph::new("  ↑/↓ navigate  ·  Enter confirm  ·  Esc cancel"),
        rows[8],
    );
}

fn render_row_list(frame: &mut Frame, area: Rect, modal: &NewWorktreeModal, visible: usize) {
    // Build the combined list: row 0 = "Create new <input>", rows 1..
    // = filter_matches[i].  Scroll so the selected row stays visible.
    let total = modal.total_rows();
    let scroll_offset = modal
        .cursor
        .saturating_sub(visible - 1)
        .min(total.saturating_sub(visible));
    let dim = Style::default().fg(Color::DarkGray);
    let bold = Style::default().add_modifier(Modifier::BOLD);

    let mut items: Vec<ListItem> = Vec::with_capacity(total);

    // Row 0: create new.
    {
        let input = modal.input.value();
        let is_cursor = modal.cursor == 0;
        let marker = if is_cursor { "▶ " } else { "  " };
        let label_style = if is_cursor { bold } else { Style::default() };
        let placeholder_style = if input.is_empty() { dim } else { label_style };
        let shown = if input.is_empty() {
            "(type a name)".to_string()
        } else {
            input.to_string()
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw(marker),
            Span::styled("Create new branch ", label_style),
            Span::styled(shown, placeholder_style),
        ])));
    }

    // Rows 1..: filtered existing branches.
    for (i, &branch_idx) in modal.filter_matches.iter().enumerate() {
        let row = i + 1;
        let is_cursor = modal.cursor == row;
        let marker = if is_cursor { "▶ " } else { "  " };
        let entry = &modal.branches[branch_idx];
        let name_style = if is_cursor { bold } else { Style::default() };
        let tag = if entry.is_remote_only() {
            Span::styled(" [remote]", dim)
        } else {
            Span::raw("")
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw(marker),
            Span::styled(entry.display(), name_style),
            tag,
        ])));
    }

    // Render the windowed slice.
    let windowed: Vec<ListItem> = items
        .into_iter()
        .skip(scroll_offset)
        .take(visible)
        .collect();
    frame.render_widget(List::new(windowed), area);
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
