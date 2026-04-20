use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{AppState, SidebarCursor};

const DIVIDER: &str = "────────────────────────────";

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let cursor = app.ui.cursor;
    let highlight = Style::default().add_modifier(Modifier::REVERSED);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(" PROJECTS"));
    lines.push(Line::from(format!(" {DIVIDER}")));

    for (i, repo) in app.repos.iter().enumerate() {
        let expanded = app.ui.is_expanded(&repo.name);
        let glyph = if expanded { "▼" } else { "▶" };
        let is_here = cursor == Some(SidebarCursor::Repo(i));
        let marker = if is_here { "▸" } else { " " };
        let text = format!("{marker}{glyph} {}", repo.name);
        let line = if is_here {
            Line::styled(text, highlight)
        } else {
            Line::from(text)
        };
        lines.push(line);
        if expanded {
            for (j, wt) in repo.worktrees.iter().enumerate() {
                let branch_glyph = if wt.is_primary { "○" } else { "●" };
                let is_here = cursor
                    == Some(SidebarCursor::Worktree {
                        repo: i,
                        worktree: j,
                    });
                let marker = if is_here { "▸" } else { " " };
                let text = format!("{marker}  {branch_glyph} {}", wt.branch);
                let line = if is_here {
                    Line::styled(text, highlight)
                } else {
                    Line::from(text)
                };
                lines.push(line);
            }
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(format!(" {DIVIDER}")));
    lines.push(Line::from(" [a] Add repo"));
    lines.push(Line::from(" [w] New worktree"));
    lines.push(Line::from(""));
    lines.push(Line::from(" j/k  navigate"));
    lines.push(Line::from(" ?    help"));
    lines.push(Line::from(" q    quit"));

    frame.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::app::{AppMessage, AppState, Direction, SidebarCursor};

    fn render_to_string(app: &AppState) -> String {
        let backend = TestBackend::new(32, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::render(frame, frame.area(), app))
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn cursor_on_first_repo_is_highlighted() {
        let app = AppState::fixture();
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn cursor_on_worktree_is_highlighted() {
        let mut app = AppState::fixture();
        app.ui.cursor = Some(SidebarCursor::Worktree {
            repo: 0,
            worktree: 1,
        });
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn collapsed_repo_hides_worktrees() {
        let mut app = AppState::fixture();
        app.update(AppMessage::CollapseOrAscend);
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn navigating_down_moves_highlight() {
        let mut app = AppState::fixture();
        app.update(AppMessage::MoveCursor(Direction::Down));
        app.update(AppMessage::MoveCursor(Direction::Down));
        insta::assert_snapshot!(render_to_string(&app));
    }
}
