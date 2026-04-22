use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{AppState, SidebarCursor};
use crate::model::WorktreeStatus;
use crate::ui::badges::{append_pr_spans, badge_spans, badge_width};

const SIDEBAR_WIDTH: usize = 32;
const DIVIDER: &str = "────────────────────────────";
/// Columns consumed by the fixed prefix of a worktree row: marker + 2 spaces
/// + glyph + space. Everything after this is branch + padding + badges.
const WORKTREE_PREFIX: usize = 5;

pub fn render(frame: &mut Frame, area: Rect, app: &AppState) {
    let cursor = app.ui.cursor;
    let highlight = Style::default().add_modifier(Modifier::REVERSED);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(" PROJECTS"));
    lines.push(Line::from(format!(" {DIVIDER}")));

    if app.repos.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(" No repositories configured."));
        lines.push(Line::from(""));
        lines.push(Line::from(" Press [a] to add one."));
        lines.push(Line::from(""));
    }

    for (i, repo) in app.repos.iter().enumerate() {
        let expanded = app.ui.is_expanded(&repo.root_path);
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
                let line = worktree_line(
                    marker,
                    branch_glyph,
                    &wt.branch,
                    wt.status.as_ref(),
                    wt.pr.as_ref(),
                    is_here,
                    highlight,
                );
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

fn worktree_line(
    marker: &str,
    branch_glyph: &str,
    branch: &str,
    status: Option<&WorktreeStatus>,
    pr: Option<&crate::model::PrStatus>,
    is_cursor: bool,
    highlight: Style,
) -> Line<'static> {
    let prefix = format!("{marker}  {branch_glyph} ");
    let mut status_spans = status.map(badge_spans).unwrap_or_default();
    append_pr_spans(pr, &mut status_spans);
    let status_cols = if status_spans.is_empty() {
        0
    } else {
        badge_width(&status_spans) + 1 // one-space gap before badges
    };
    let branch_budget = SIDEBAR_WIDTH.saturating_sub(WORKTREE_PREFIX + status_cols);
    let branch_shown = truncate(branch, branch_budget);

    // Pad so badges end exactly at SIDEBAR_WIDTH.
    let used = WORKTREE_PREFIX + branch_shown.chars().count();
    let pad_cols = SIDEBAR_WIDTH.saturating_sub(
        used + status_spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum::<usize>(),
    );

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::raw(prefix));
    spans.push(Span::raw(branch_shown));
    spans.push(Span::raw(" ".repeat(pad_cols)));
    for s in status_spans {
        spans.push(s);
    }

    let line = Line::from(spans);
    if is_cursor {
        line.style(highlight)
    } else {
        line
    }
}

fn truncate(s: &str, max_cols: usize) -> String {
    let len = s.chars().count();
    if len <= max_cols {
        return s.to_string();
    }
    if max_cols == 0 {
        return String::new();
    }
    let keep = max_cols - 1;
    let mut t: String = s.chars().take(keep).collect();
    t.push('…');
    t
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::app::{AppMessage, AppState, Direction, SidebarCursor};
    use crate::model::WorktreeStatus;

    fn render_to_string(app: &AppState) -> String {
        let backend = TestBackend::new(32, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::render(frame, frame.area(), app))
            .unwrap();
        terminal.backend().to_string()
    }

    fn fixture_with_statuses() -> AppState {
        let mut app = AppState::fixture();
        app.repos[0].worktrees[0].status = Some(WorktreeStatus::default());
        app.repos[0].worktrees[1].status = Some(WorktreeStatus {
            staged: 4,
            ahead: 1,
            ..WorktreeStatus::default()
        });
        app.repos[0].worktrees[2].status = Some(WorktreeStatus {
            modified: 2,
            conflicts: 1,
            ..WorktreeStatus::default()
        });
        app.repos[1].worktrees[0].status = Some(WorktreeStatus::default());
        app.repos[1].worktrees[1].status = Some(WorktreeStatus {
            ahead: 1,
            behind: 2,
            ..WorktreeStatus::default()
        });
        app
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

    #[test]
    fn empty_state_shows_add_hint() {
        use std::path::PathBuf;
        let app = AppState::empty_fixture(PathBuf::from("/tmp/grove-test-config.toml"));
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn worktrees_with_statuses_show_badges() {
        let app = fixture_with_statuses();
        insta::assert_snapshot!(render_to_string(&app));
    }

    #[test]
    fn long_branch_name_is_truncated_when_badges_present() {
        let mut app = AppState::fixture();
        app.repos[0].worktrees[1].branch = "feature/this-is-a-very-long-branch".to_string();
        app.repos[0].worktrees[1].status = Some(WorktreeStatus {
            staged: 4,
            ahead: 2,
            ..WorktreeStatus::default()
        });
        insta::assert_snapshot!(render_to_string(&app));
    }
}
