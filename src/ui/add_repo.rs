use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{AddRepoModal, NewWorktreeModal, NewWorktreeMode};
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
    match modal.mode {
        NewWorktreeMode::PickBranch => render_pick_branch(frame, area, modal, repo_name),
        NewWorktreeMode::NewBranch => render_new_branch(frame, area, modal, repo_name),
    }
}

fn render_pick_branch(frame: &mut Frame, area: Rect, modal: &NewWorktreeModal, repo_name: &str) {
    let visible = 8usize;
    let height = (visible + 6) as u16;
    let modal_area = centered_rect(66, height, area);
    frame.render_widget(Clear, modal_area);

    let title = format!(" New worktree in {repo_name} ");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let rows = Layout::vertical([
        Constraint::Length(1),              // blank
        Constraint::Length(visible as u16), // branch list
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // error
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // hint
    ])
    .split(inner);

    if modal.branches.is_empty() {
        frame.render_widget(Paragraph::new("  No existing branches found."), rows[1]);
    } else {
        let scroll_offset = modal
            .branch_cursor
            .saturating_sub(visible - 1)
            .min(modal.branches.len().saturating_sub(visible));

        let items: Vec<ListItem> = modal
            .branches
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible)
            .map(|(i, entry)| {
                let selected = i == modal.branch_cursor;
                let marker = if selected { "▶ " } else { "  " };
                let name_style = if selected {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let tag = if entry.is_remote_only() {
                    Span::styled(" [remote]", Style::default().fg(Color::DarkGray))
                } else {
                    Span::raw("")
                };
                ListItem::new(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(entry.display(), name_style),
                    tag,
                ]))
            })
            .collect();
        frame.render_widget(List::new(items), rows[1]);
    }

    if let Some(err) = &modal.error {
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!("  ! {err}"),
                Style::default().fg(Color::Red),
            )),
            rows[3],
        );
    }

    let hint = if modal.branches.is_empty() {
        "  Tab: type new branch name  ·  Esc cancel"
    } else {
        "  j/k navigate  ·  Enter create  ·  Tab: new branch  ·  Esc cancel"
    };
    frame.render_widget(Paragraph::new(hint), rows[5]);
}

fn render_new_branch(frame: &mut Frame, area: Rect, modal: &NewWorktreeModal, repo_name: &str) {
    let modal_area = centered_rect(62, 11, area);
    frame.render_widget(Clear, modal_area);
    let title = format!(" New worktree in {repo_name} ");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(Paragraph::new("  Branch name:"), rows[1]);

    let input_area = Rect {
        x: rows[2].x + 2,
        width: rows[2].width.saturating_sub(2),
        ..rows[2]
    };
    text_input::render(frame, input_area, &modal.input);

    if let Some(err) = &modal.error {
        let line = Line::styled(format!("  ! {err}"), Style::default().fg(Color::Red));
        frame.render_widget(Paragraph::new(line), rows[4]);
    }

    let hint = if !modal.branches.is_empty() {
        "  Enter create  ·  Tab pick existing  ·  Esc cancel"
    } else {
        "  Enter to create  ·  Esc to cancel"
    };
    frame.render_widget(Paragraph::new(hint), rows[6]);
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
