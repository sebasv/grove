use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use tui_term::widget::PseudoTerminal;

use crate::app::{AppState, FocusZone, MainView, TerminalMode, WorktreeTerminals};
use crate::ui::diff;

const DIVIDER: &str = "────────────────────────────────────────";

/// Render the right-hand main pane and return the inner Rect (minus the
/// bordering block) so callers can size the embedded PTY to match.
pub fn render(frame: &mut Frame, area: Rect, app: &AppState) -> Rect {
    let focused = app.ui.focus == FocusZone::Main;
    let border_style = if focused {
        Style::default().fg(app.theme.accent)
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

    if let Some(id) = app.ui.active_worktree {
        let view = app.main_views.get(&id).copied().unwrap_or_default();
        match view {
            MainView::Diff => {
                if let Some(state) = app.diffs.get(&id) {
                    let base = app
                        .repos
                        .get(id.0)
                        .map(|r| r.base_branch.as_str())
                        .unwrap_or("main");
                    diff::render(frame, inner, state, base);
                    return inner;
                }
                frame.render_widget(
                    Paragraph::new("  loading diff…")
                        .style(Style::default().add_modifier(Modifier::DIM)),
                    inner,
                );
                return inner;
            }
            MainView::Terminal => {
                if let Some(ts) = app.terminals.get(&id) {
                    if !ts.list.is_empty() {
                        let layout = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([Constraint::Length(1), Constraint::Min(0)])
                            .split(inner);
                        render_tab_bar(frame, layout[0], ts);
                        render_active_terminal(frame, layout[1], ts);
                        return layout[1];
                    }
                }
            }
        }
    }

    render_placeholder(frame, inner, app);
    inner
}

fn render_tab_bar(frame: &mut Frame, area: Rect, ts: &WorktreeTerminals) {
    let mut spans = Vec::with_capacity(ts.list.len() * 2 + 1);
    for i in 0..ts.list.len() {
        let is_active = i == ts.active;
        let marker = if is_active { "▸" } else { " " };
        let mode_hint = if is_active && ts.mode == TerminalMode::Scrollback {
            " ⇅"
        } else {
            ""
        };
        let label = format!(" {marker}{}{mode_hint} ", i + 1);
        let style = if is_active {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().add_modifier(Modifier::DIM)
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        " + ",
        Style::default().add_modifier(Modifier::DIM),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_active_terminal(frame: &mut Frame, area: Rect, ts: &WorktreeTerminals) {
    let Some(term) = ts.active_ref() else {
        return;
    };
    let Ok(mut parser) = term.parser.lock() else {
        return;
    };
    parser.set_scrollback(ts.scroll_offset);
    frame.render_widget(PseudoTerminal::new(parser.screen()), area);
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
