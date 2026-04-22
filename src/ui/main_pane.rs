use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::AppState;

const DIVIDER: &str = "────────────────────────────────────────";

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let lines = match app.ui.active_worktree {
        None => empty_lines(),
        Some((repo_idx, wt_idx)) => active_lines(app, repo_idx, wt_idx),
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn empty_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from("  Select a worktree from the sidebar."),
        Line::from("  (↑↓ or j/k to move, Enter to activate)"),
    ]
}

fn active_lines(app: &AppState, repo_idx: usize, wt_idx: usize) -> Vec<Line<'static>> {
    let Some(repo) = app.repos.get(repo_idx) else {
        return empty_lines();
    };
    let Some(wt) = repo.worktrees.get(wt_idx) else {
        return empty_lines();
    };
    let bold = Style::default().add_modifier(Modifier::BOLD);
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
        Line::from("  In later phases this pane will show embedded"),
        Line::from("  terminals and the diff viewer."),
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
            .draw(|frame| super::render(frame, frame.area(), app))
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn shows_prompt_when_no_worktree_active() {
        let app = AppState::fixture();
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn shows_details_when_worktree_active() {
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        app.update(AppMessage::Activate);
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn stale_active_worktree_index_shows_prompt_not_panic() {
        let mut app = AppState::fixture();
        app.ui.active_worktree = Some((99, 99));
        let output = render_to_string(&app);
        assert!(output.contains("Select a worktree"));
    }
}
