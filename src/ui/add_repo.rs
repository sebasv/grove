use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{AddRepoModal, NewWorktreeModal};
use crate::theme::Theme;
use crate::ui::{centered_rect, text_input, wrap_message};

/// Cap on wrapped error lines inside modals — beyond this we ellipsize so a
/// pathological multi-line anyhow chain can't push the modal off-screen.
const ERROR_MAX_LINES: usize = 3;

pub fn render(frame: &mut Frame, area: Rect, modal: &AddRepoModal, theme: &Theme) {
    let modal_width: u16 = 62;
    // Error rows are padded by "  ! " (4 cols) on the first line and "    "
    // (4 cols) on continuations; budget the wrap to the inner content area
    // minus those gutters. Inner width = modal_width - 2 (borders).
    let err_width = (modal_width as usize).saturating_sub(2 + 4);
    let err_lines: Vec<String> = modal
        .error
        .as_deref()
        .map(|m| wrap_message(m, err_width.max(1), ERROR_MAX_LINES))
        .unwrap_or_default();
    let err_rows = err_lines.len() as u16;

    let n = modal.completions.len().min(10);
    // base layout: blank+label+input + (sep+completions) + blank + error + blank + hint
    // Borders consume 2 rows on top of `inner`, hence +2.
    let mut height = 2 + 3 + 1 + 1 + 1; // borders + (blank,label,input) + blank + blank + hint
    if n > 0 {
        height += 1 + n as u16; // separator + completions
    }
    height += err_rows;

    let modal_area = centered_rect(modal_width, height, area);
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
    constraints.push(Constraint::Length(err_rows)); // error (0 rows when no error)
    constraints.push(Constraint::Length(1)); // blank
    constraints.push(Constraint::Length(1)); // hint

    let rows = Layout::vertical(constraints).split(inner);

    frame.render_widget(Paragraph::new("  Path to git repository:"), rows[1]);

    let input_area = Rect {
        x: rows[2].x + 2,
        width: rows[2].width.saturating_sub(4),
        ..rows[2]
    };
    text_input::render(frame, input_area, &modal.input, theme.input_bg_focused);

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
        frame.render_widget(Paragraph::new(lines), rows[base_row + 1]);
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
    theme: &Theme,
) {
    let modal_width: u16 = 70;
    let err_width = (modal_width as usize).saturating_sub(2 + 4);
    let err_lines: Vec<String> = modal
        .error
        .as_deref()
        .map(|m| wrap_message(m, err_width.max(1), ERROR_MAX_LINES))
        .unwrap_or_default();
    let err_rows = err_lines.len() as u16;

    let visible = 10usize;
    // borders(2) + blank+label+input(3) + sep(1) + list(visible) + blank(1)
    // + err_rows + blank(1) + hint(1)
    let height = 2 + 3 + 1 + visible as u16 + 1 + err_rows + 1 + 1;
    let modal_area = centered_rect(modal_width, height, area);
    frame.render_widget(Clear, modal_area);

    // Reserve room around the title so the trailing space and borders survive
    // long repo names; the block uses chars from inside the borders.
    let title_budget = (modal_width as usize).saturating_sub(4);
    let title =
        crate::ui::truncate_to_width(&format!(" New worktree in {repo_name} "), title_budget);
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
        Constraint::Length(err_rows),       // error (0 rows when no error)
        Constraint::Length(1),              // blank
        Constraint::Length(1),              // hint
    ])
    .split(inner);

    frame.render_widget(Paragraph::new("  Filter / new branch name:"), rows[1]);

    let input_area = Rect {
        x: rows[2].x + 2,
        width: rows[2].width.saturating_sub(4),
        ..rows[2]
    };
    text_input::render(frame, input_area, &modal.input, theme.input_bg_focused);

    frame.render_widget(
        Paragraph::new(Line::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(Color::DarkGray),
        )),
        rows[3],
    );

    render_row_list(frame, rows[4], modal, theme);

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
        frame.render_widget(Paragraph::new(lines), rows[6]);
    }

    frame.render_widget(
        Paragraph::new("  ↑/↓ navigate  ·  Enter confirm  ·  Esc cancel"),
        rows[8],
    );
}

fn render_row_list(frame: &mut Frame, area: Rect, modal: &NewWorktreeModal, theme: &Theme) {
    // Build the combined list: row 0 = "Create new <input>", rows 1..
    // = filter_matches[i].  Uses `ListState` + `highlight_style` so the
    // cursor row is the only row ratatui styles differently — every
    // other item renders with an identical, plain style, avoiding the
    // terminal-dependent bleed we saw when individual items mixed
    // explicit and default foregrounds.
    let dim = Style::default().fg(theme.dim);

    let mut items: Vec<ListItem> = Vec::with_capacity(modal.total_rows());

    // Row 0: create new.
    {
        let input = modal.input.value();
        let shown = if input.is_empty() {
            "(type a name)".to_string()
        } else {
            input.to_string()
        };
        let placeholder_style = if input.is_empty() {
            dim
        } else {
            Style::default()
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw("Create new branch "),
            Span::styled(shown, placeholder_style),
        ])));
    }

    // Rows 1..: filtered existing branches.
    for &branch_idx in &modal.filter_matches {
        let entry = &modal.branches[branch_idx];
        let mut spans = vec![Span::raw(entry.display())];
        if entry.is_remote_only() {
            spans.push(Span::styled(" [remote]", dim));
        }
        items.push(ListItem::new(Line::from(spans)));
    }

    // `highlight_symbol` reserves a prefix column per row; non-selected
    // rows get blank padding, so the text lines up with the ▶ marker
    // without us having to hand-indent every item.  Using `REVERSED`
    // for highlight (fg/bg swap) rather than a foreground colour means
    // the cursor row is a self-contained toggled attribute, so
    // terminals that don't cleanly reset colour SGR between rows
    // (macOS Terminal with some profiles) can't leak the cursor
    // row's tint into adjacent rows.
    let mut state = ListState::default();
    state.select(Some(modal.cursor));
    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(list, area, &mut state);
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

    #[test]
    fn pathologically_long_error_is_wrapped_and_ellipsized() {
        // Inject a multi-paragraph anyhow-style error and verify the
        // rendered output stays within the modal: no ! line over the
        // border, an ellipsis on the last visible error line, and the
        // hint still rendered below.
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenAddRepo);
        if let Some(crate::app::Modal::AddRepo(state)) = app.ui.modal.as_mut() {
            state.error = Some(
                "resolving path /a/very/long/absolute/path/that/definitely/\
                 does/not/exist/anywhere/on/disk: No such file or directory \
                 (os error 2)\nwhile loading repository metadata\nfrom git2 \
                 backend"
                    .to_string(),
            );
        }
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                crate::ui::render(frame, &app);
            })
            .unwrap();
        let out = terminal.backend().to_string();
        assert!(out.contains("  ! "), "missing first error line:\n{out}");
        assert!(
            out.contains('…'),
            "expected ellipsis to mark truncated tail:\n{out}"
        );
        assert!(
            out.contains("Enter to add"),
            "hint must still render below the error:\n{out}"
        );
    }
}
