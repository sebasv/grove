//! Status-badge formatting shared by the sidebar and any future pane that
//! needs to summarize a worktree's state.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::model::{ChecksRollup, PrState, PrStatus, WorktreeStatus};

/// Build the colored span sequence for a worktree's status, ordered by
/// priority (leftmost = most urgent).
pub fn badge_spans(status: &WorktreeStatus) -> Vec<Span<'static>> {
    if status.is_clean() {
        return vec![Span::styled("✓", Style::default().fg(Color::Green))];
    }

    let mut parts: Vec<(String, Style)> = Vec::new();
    if status.conflicts > 0 {
        parts.push(("⚠".to_string(), Style::default().fg(Color::Red)));
    }
    if status.staged > 0 {
        parts.push((
            format!("+{}", status.staged),
            Style::default().fg(Color::Green),
        ));
    }
    if status.modified > 0 {
        parts.push((
            format!("~{}", status.modified),
            Style::default().fg(Color::Yellow),
        ));
    }
    if status.deleted > 0 {
        parts.push((
            format!("-{}", status.deleted),
            Style::default().fg(Color::Red),
        ));
    }
    if status.ahead > 0 {
        parts.push((
            format!("↑{}", status.ahead),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if status.behind > 0 {
        parts.push((
            format!("↓{}", status.behind),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }

    let mut spans = Vec::with_capacity(parts.len() * 2);
    for (i, (text, style)) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(text, style));
    }
    spans
}

/// Column width the badges will consume when rendered. Counts characters as
/// one column each; accurate for our ASCII + Box-Drawing glyphs on a standard
/// terminal.
pub fn badge_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// Append PR + CI spans to the output of `badge_spans`.  No-op when `pr` is
/// None.  Call this after `badge_spans` to keep the status badges stable.
pub fn append_pr_spans(pr: Option<&PrStatus>, spans: &mut Vec<Span<'static>>) {
    let Some(pr) = pr else { return };
    if !spans.is_empty() {
        spans.push(Span::raw(" "));
    }
    let (glyph, style) = match pr.state {
        PrState::Open => ("●", Style::default().fg(Color::Green)),
        PrState::Draft => ("◐", Style::default().fg(Color::Yellow)),
        PrState::Merged => (
            "✓●",
            Style::default().fg(Color::Green).add_modifier(Modifier::DIM),
        ),
        PrState::Closed => ("●", Style::default().add_modifier(Modifier::DIM)),
    };
    spans.push(Span::styled(glyph.to_string(), style));
    match pr.checks {
        ChecksRollup::Failing => {
            spans.push(Span::styled(
                "✗".to_string(),
                Style::default().fg(Color::Red),
            ));
        }
        ChecksRollup::Pending => {
            spans.push(Span::styled(
                "⟳".to_string(),
                Style::default().fg(Color::Yellow),
            ));
        }
        ChecksRollup::Passing | ChecksRollup::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flatten(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn clean_renders_check_mark() {
        let s = WorktreeStatus::default();
        let spans = badge_spans(&s);
        assert_eq!(flatten(&spans), "✓");
    }

    #[test]
    fn staged_and_modified_combine() {
        let s = WorktreeStatus {
            staged: 3,
            modified: 2,
            ..WorktreeStatus::default()
        };
        assert_eq!(flatten(&badge_spans(&s)), "+3 ~2");
    }

    #[test]
    fn conflict_comes_first() {
        let s = WorktreeStatus {
            conflicts: 1,
            modified: 2,
            ahead: 1,
            ..WorktreeStatus::default()
        };
        assert_eq!(flatten(&badge_spans(&s)), "⚠ ~2 ↑1");
    }

    #[test]
    fn ahead_behind_only() {
        let s = WorktreeStatus {
            ahead: 1,
            behind: 2,
            ..WorktreeStatus::default()
        };
        assert_eq!(flatten(&badge_spans(&s)), "↑1 ↓2");
    }

    #[test]
    fn badge_width_sums_characters() {
        let spans = badge_spans(&WorktreeStatus {
            staged: 10,
            ahead: 2,
            ..WorktreeStatus::default()
        });
        // "+10 ↑2" = 6 chars
        assert_eq!(badge_width(&spans), 6);
    }
}
