use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use tui_term::widget::PseudoTerminal;

use crate::app::{AppState, FocusZone};

const DIVIDER: &str = "────────────────────────────────────────";

/// Render the right-hand main pane and return the inner Rect (minus the
/// bordering block) so callers can size the embedded PTY to match.
pub fn render(frame: &mut Frame, area: Rect, app: &AppState) -> Rect {
    let focused = app.ui.focus == FocusZone::Main;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };
    let title = main_pane_title(app);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(title, Style::default().add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Try to render a terminal first. If none, fall back to the informational
    // placeholder content.
    if let Some(id) = app.ui.active_worktree {
        if let Some(term) = app.terminals.get(&id) {
            if let Ok(parser) = term.parser.lock() {
                frame.render_widget(PseudoTerminal::new(parser.screen()), inner);
                return inner;
            }
        }
    }

    render_placeholder(frame, inner, app);
    inner
}

fn main_pane_title(app: &AppState) -> String {
    match app.ui.active_worktree {
        Some((r, w)) => {
            let name = app
                .repos
                .get(r)
                .map(|r| r.name.as_str())
                .unwrap_or("?");
            let branch = app
                .repos
                .get(r)
                .and_then(|repo| repo.worktrees.get(w))
                .map(|wt| wt.branch.as_str())
                .unwrap_or("?");
            format!(" {name} · {branch} ")
        }
        None => " grove ".to_string(),
    }
}

fn render_placeholder(frame: &mut Frame, area: Rect, app: &AppState) {
    let lines = match app.ui.active_worktree {
        None => empty_lines(),
        Some((r, w)) => active_lines_without_terminal(app, r, w),
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn empty_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from("  Select a worktree from the sidebar."),
        Line::from("  (↑↓ or j/k to move, Enter to activate)"),
        Line::from(""),
        Line::from("  Ctrl+Space cycles focus between sidebar and main."),
    ]
}

fn active_lines_without_terminal(app: &AppState, r: usize, w: usize) -> Vec<Line<'static>> {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let Some(repo) = app.repos.get(r) else {
        return empty_lines();
    };
    let Some(wt) = repo.worktrees.get(w) else {
        return empty_lines();
    };
    vec![
        Line::from(""),
        Line::styled("  Active worktree", bold),
        Line::from(format!("  {DIVIDER}")),
        Line::from(vec![
            Span::raw("  repo:    "),
            Span::styled(repo.name.clone(), bold),
        ]),
        Line::from(vec![
            Span::raw("  branch:  "),
            Span::styled(wt.branch.clone(), bold),
        ]),
        Line::from(format!("  path:    {}", wt.path.display())),
        Line::from(""),
        Line::from("  Press Enter to start a shell here."),
        Line::from("  Ctrl+Space to return to the sidebar."),
    ]
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::app::{AppMessage, AppState, SidebarCursor};

    fn render_to_string(app: &AppState) -> String {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                super::render(frame, frame.area(), app);
            })
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn shows_prompt_when_no_worktree_active() {
        let app = AppState::fixture();
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn shows_details_when_worktree_active_without_terminal() {
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        app.update(AppMessage::Activate);
        insta::assert_snapshot!(render_to_string(&app));
    }
}
