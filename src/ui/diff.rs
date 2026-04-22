use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{DiffFocus, DiffMode, DiffState};
use crate::git::{DeltaKind, DiffLineKind};

pub fn render(frame: &mut Frame, area: Rect, state: &DiffState, base_branch: &str) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    render_mode_header(frame, rows[0], state, base_branch);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(30), Constraint::Min(0)])
        .split(rows[1]);

    render_file_list(frame, columns[0], state);
    render_content(frame, columns[1], state);
}

fn render_mode_header(frame: &mut Frame, area: Rect, state: &DiffState, base_branch: &str) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    let label = match state.mode {
        DiffMode::Local => "  Diff: local changes".to_string(),
        DiffMode::Branch => format!("  Diff: branch vs {base_branch}"),
    };
    let line = Line::from(vec![
        Span::styled(label, bold),
        Span::raw("    "),
        Span::styled("[m] switch mode".to_string(), dim),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_file_list(frame: &mut Frame, area: Rect, state: &DiffState) {
    let focused = state.diff_focus == DiffFocus::List;
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();

    if state.files.is_empty() {
        lines.push(Line::styled(
            "  No local changes",
            Style::default().add_modifier(Modifier::DIM),
        ));
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let mut printed_unstaged_header = false;
    let mut printed_staged_header = false;
    for (i, file) in state.files.iter().enumerate() {
        if !file.staged && !printed_unstaged_header {
            lines.push(Line::styled(" UNSTAGED", bold));
            printed_unstaged_header = true;
        }
        if file.staged && !printed_staged_header {
            if printed_unstaged_header {
                lines.push(Line::from(""));
            }
            lines.push(Line::styled(" STAGED", bold));
            printed_staged_header = true;
        }

        let is_cursor = i == state.cursor;
        let marker = if is_cursor && focused { "▸" } else { " " };
        let glyph = match file.kind {
            DeltaKind::Added => "+",
            DeltaKind::Modified => "~",
            DeltaKind::Deleted => "-",
            DeltaKind::Renamed => "»",
            DeltaKind::Other => " ",
        };
        let counts = format!("+{} -{}", file.adds, file.dels);
        let base_text = format!("{marker}{glyph} {}", file.path.display());
        let line = Line::from(vec![
            Span::raw(base_text),
            Span::raw("  "),
            Span::styled(counts, Style::default().add_modifier(Modifier::DIM)),
        ]);
        lines.push(if is_cursor {
            line.style(Style::default().add_modifier(Modifier::REVERSED))
        } else {
            line
        });
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_content(frame: &mut Frame, area: Rect, state: &DiffState) {
    let Some(file) = state.files.get(state.cursor) else {
        frame.render_widget(
            Paragraph::new("  (pick a file on the left)".to_string()),
            area,
        );
        return;
    };

    let header_style = Style::default().add_modifier(Modifier::BOLD);
    let scope_style = Style::default().fg(Color::Cyan);
    let add_style = Style::default().fg(Color::Green);
    let del_style = Style::default().fg(Color::Red);

    let mut lines: Vec<Line> = Vec::new();
    let scope = if file.staged {
        "[staged]"
    } else {
        "[unstaged]"
    };
    lines.push(Line::from(vec![
        Span::styled(format!("  {}", file.path.display()), header_style),
        Span::raw("  "),
        Span::styled(scope.to_string(), scope_style),
    ]));
    lines.push(Line::from(""));

    for hunk in &file.hunks {
        lines.push(Line::styled(
            format!("  {}", hunk.header.trim_end()),
            Style::default().add_modifier(Modifier::DIM),
        ));
        for line in &hunk.lines {
            let prefix = match line.kind {
                DiffLineKind::Add => "+",
                DiffLineKind::Del => "-",
                DiffLineKind::Context => " ",
            };
            let style = match line.kind {
                DiffLineKind::Add => add_style,
                DiffLineKind::Del => del_style,
                DiffLineKind::Context => Style::default(),
            };
            lines.push(Line::styled(format!("  {prefix}{}", line.content), style));
        }
    }

    let para = Paragraph::new(lines).scroll((state.scroll, 0));
    frame.render_widget(para, area);
}
