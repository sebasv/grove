mod add_repo;
mod badges;
mod confirm;
pub mod diff;
mod discovered;
mod help;
mod log_view;
mod main_pane;
mod sidebar;
pub mod text_input;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::app::{AppState, Modal};

const SIDEBAR_WIDTH: u16 = 32;

/// Below this, the sidebar (32 cols) plus a usable main pane no longer fit.
/// Modals also assume at least this much room. Anything smaller renders a
/// placeholder instead of a broken layout.
const MIN_FRAME_WIDTH: u16 = 50;
const MIN_FRAME_HEIGHT: u16 = 12;

pub struct RenderedLayout {
    pub sidebar: Rect,
    pub main_inner: Rect,
    pub tab_bar: Option<Rect>,
}

/// Render the whole frame and return layout rects (used by the caller to
/// sync PTY size on resize and to route mouse clicks).
pub fn render(frame: &mut Frame, app: &AppState) -> RenderedLayout {
    let area = frame.area();
    if area.width < MIN_FRAME_WIDTH || area.height < MIN_FRAME_HEIGHT {
        render_too_small(frame, area);
        return RenderedLayout {
            sidebar: Rect::new(area.x, area.y, 0, area.height),
            main_inner: area,
            tab_bar: None,
        };
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
        .split(area);

    sidebar::render(frame, chunks[0], app);
    let (inner, tab_bar) = main_pane::render(frame, chunks[1], app);

    if let Some(modal) = &app.ui.modal {
        match modal {
            Modal::Help => help::render(frame, frame.area(), app.ui.help_scroll),
            Modal::AddRepo(state) => add_repo::render(frame, frame.area(), state, &app.theme),
            Modal::NewWorktree(state) => {
                let repo_name = app
                    .repos
                    .get(state.repo_idx)
                    .map(|r| r.name.as_str())
                    .unwrap_or("?");
                add_repo::render_new_worktree(frame, frame.area(), state, repo_name, &app.theme);
            }
            Modal::ConfirmRemoveRepo { repo_idx } => {
                if let Some(repo) = app.repos.get(*repo_idx) {
                    confirm::render_remove_repo(frame, frame.area(), &repo.name);
                }
            }
            Modal::ConfirmRemoveWorktree {
                id,
                variant,
                pr_number,
                error,
            } => {
                if let Some((repo, wt)) = app
                    .repos
                    .get(id.0)
                    .and_then(|r| r.worktrees.get(id.1).map(|wt| (r, wt)))
                {
                    confirm::render_remove_worktree(
                        frame,
                        frame.area(),
                        &wt.label(),
                        &wt.path,
                        &repo.name,
                        *variant,
                        *pr_number,
                        error.as_deref(),
                    );
                }
            }
            Modal::DiscoveredRepos(state) => {
                discovered::render(frame, frame.area(), state);
            }
            Modal::ViewLog(state) => {
                log_view::render(frame, frame.area(), state);
            }
        }
    }

    RenderedLayout {
        sidebar: chunks[0],
        main_inner: inner,
        tab_bar,
    }
}

pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

/// Truncate `s` to fit within `max_cols` columns, replacing the cut tail
/// with `…`. Counts chars, matching the rest of grove's display arithmetic.
pub fn truncate_to_width(s: &str, max_cols: usize) -> String {
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

/// Word-wrap `msg` into at most `max_lines` lines of `width` columns. Honors
/// embedded `\n` (anyhow chains formatted with `{:#}` produce them), breaks
/// at whitespace, and hard-cuts words longer than `width`. If the message
/// would exceed `max_lines`, the last shown line is ellipsized so the user
/// can tell the tail was dropped.
pub fn wrap_message(msg: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut all: Vec<String> = Vec::new();
    let mut overflowed = false;

    'paragraphs: for paragraph in msg.split('\n') {
        if paragraph.trim().is_empty() {
            // Collapse runs of blank lines to a single visual gap.
            if !all.last().map(String::is_empty).unwrap_or(true) {
                all.push(String::new());
            }
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            // Hard-break a word that's wider than the line on its own.
            let mut remaining: String = word.to_string();
            while remaining.chars().count() > width {
                if !current.is_empty() {
                    all.push(std::mem::take(&mut current));
                    if all.len() > max_lines {
                        overflowed = true;
                        break 'paragraphs;
                    }
                }
                let head: String = remaining.chars().take(width).collect();
                let tail: String = remaining.chars().skip(width).collect();
                all.push(head);
                if all.len() > max_lines {
                    overflowed = true;
                    break 'paragraphs;
                }
                remaining = tail;
            }
            if remaining.is_empty() {
                continue;
            }

            let cur_len = current.chars().count();
            let rem_len = remaining.chars().count();
            let needed = if current.is_empty() {
                rem_len
            } else {
                cur_len + 1 + rem_len
            };
            if needed <= width {
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(&remaining);
            } else {
                all.push(std::mem::take(&mut current));
                if all.len() > max_lines {
                    overflowed = true;
                    break 'paragraphs;
                }
                current.push_str(&remaining);
            }
        }
        if !current.is_empty() {
            all.push(current);
            if all.len() > max_lines {
                overflowed = true;
                break 'paragraphs;
            }
        }
    }

    while all.last().map(String::is_empty).unwrap_or(false) {
        all.pop();
    }

    if overflowed || all.len() > max_lines {
        all.truncate(max_lines);
        if let Some(last) = all.last_mut() {
            // Trim the last line so an ellipsis fits without overflow, and
            // also strip a trailing space so we don't render "word …".
            let max_with_ellipsis = width.saturating_sub(1);
            let chars: Vec<char> = last.chars().collect();
            let needs_trim =
                chars.len() > max_with_ellipsis || chars.last().is_some_and(|c| c.is_whitespace());
            if needs_trim {
                *last = chars.into_iter().take(max_with_ellipsis).collect();
            }
            last.push('…');
        }
    }

    all
}

fn render_too_small(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    let msg = format!("Terminal too small (need at least {MIN_FRAME_WIDTH}×{MIN_FRAME_HEIGHT})");
    let lines = vec![
        Line::from(""),
        Line::from(truncate_to_width(&msg, area.width as usize)),
    ];
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(Color::Yellow)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::app::AppState;

    fn render_at(width: u16, height: u16) -> String {
        let app = AppState::fixture();
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                super::render(frame, &app);
            })
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn tiny_terminal_shows_too_small_placeholder() {
        let out = render_at(40, 8);
        assert!(
            out.contains("Terminal too small"),
            "expected placeholder, got:\n{out}"
        );
        assert!(
            !out.contains("PROJECTS"),
            "sidebar should not render at this size:\n{out}"
        );
    }

    #[test]
    fn at_minimum_size_renders_normally() {
        let out = render_at(MIN_FRAME_WIDTH, MIN_FRAME_HEIGHT);
        assert!(out.contains("PROJECTS"), "expected normal render:\n{out}");
    }

    #[test]
    fn truncate_to_width_short_passes_through() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
    }

    #[test]
    fn truncate_to_width_long_ellipsizes() {
        assert_eq!(truncate_to_width("hello world", 6), "hello…");
    }

    #[test]
    fn truncate_to_width_zero_returns_empty() {
        assert_eq!(truncate_to_width("hello", 0), "");
    }

    #[test]
    fn wrap_message_fits_in_one_line() {
        assert_eq!(wrap_message("short err", 40, 3), vec!["short err"]);
    }

    #[test]
    fn wrap_message_breaks_on_word_boundary() {
        let lines = wrap_message("the quick brown fox jumps over", 12, 3);
        assert_eq!(lines, vec!["the quick", "brown fox", "jumps over"]);
    }

    #[test]
    fn wrap_message_ellipsizes_when_over_max_lines() {
        let lines = wrap_message("aaa bbb ccc ddd eee fff ggg hhh", 4, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines.last().unwrap().ends_with('…'));
        for l in &lines {
            assert!(l.chars().count() <= 4, "line {l:?} exceeds width");
        }
    }

    #[test]
    fn wrap_message_hard_breaks_words_longer_than_width() {
        let lines = wrap_message("/an/extremely/long/absolute/path", 10, 3);
        // Expect at least one full-width chunk and a final ellipsized one.
        assert!(!lines.is_empty());
        for l in &lines {
            assert!(l.chars().count() <= 10);
        }
    }

    #[test]
    fn wrap_message_honors_embedded_newlines() {
        let lines = wrap_message("first line\nsecond line", 40, 3);
        assert_eq!(lines, vec!["first line", "second line"]);
    }

    #[test]
    fn wrap_message_zero_args_return_empty() {
        assert_eq!(wrap_message("anything", 0, 3), Vec::<String>::new());
        assert_eq!(wrap_message("anything", 10, 0), Vec::<String>::new());
    }
}
