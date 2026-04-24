use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::centered_rect;

pub fn render_remove_repo(frame: &mut Frame, area: Rect, repo_name: &str) {
    let modal_area = centered_rect(58, 8, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Remove repository? ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(""),
        Line::from(format!("  Remove \"{repo_name}\" from grove?")),
        Line::from("  (config.toml will be rewritten;"),
        Line::from("   the repository on disk is not touched)"),
        Line::from(""),
        Line::from("  y/Enter confirm  ·  n/Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

#[allow(clippy::too_many_arguments)]
pub fn render_remove_worktree(
    frame: &mut Frame,
    area: Rect,
    branch: &str,
    worktree_path: &std::path::Path,
    repo_name: &str,
    variant: crate::app::DeleteVariant,
    pr_number: Option<u32>,
    error: Option<&str>,
) {
    let unmerged = matches!(variant, crate::app::DeleteVariant::Unmerged);
    let height = if pr_number.is_some() && unmerged {
        14u16
    } else if unmerged {
        13
    } else {
        12
    };
    let modal_area = centered_rect(66, height, area);
    frame.render_widget(Clear, modal_area);

    let title = format!(" Remove worktree for {branch} ");
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let dim = Style::default().fg(Color::DarkGray);
    let warn = Style::default().fg(Color::Yellow);
    let err_style = Style::default().fg(Color::Red);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(format!("  Repository: {repo_name}")),
        Line::styled(format!("  Worktree:   {}", worktree_path.display()), dim),
        Line::from(""),
    ];

    // Warning block — only for the unmerged (force-delete) case.
    if unmerged {
        lines.push(Line::styled(
            format!("  ⚠ \"{branch}\" has unmerged commits."),
            warn,
        ));
        if let Some(n) = pr_number {
            lines.push(Line::styled(
                format!("  ⚠ PR #{n} is open — force-delete will close it."),
                warn,
            ));
        }
        lines.push(Line::from(""));
    }

    // Options.  Default action is always keep.
    lines.push(Line::from("  [k] Remove worktree, keep branch  (default)"));
    if unmerged {
        lines.push(Line::from(
            "  [D] Remove worktree and force-delete branch (-D)",
        ));
    } else {
        lines.push(Line::from("  [d] Remove worktree and delete branch (-d)"));
    }
    lines.push(Line::from(""));

    if let Some(msg) = error {
        lines.push(Line::styled(format!("  ! {msg}"), err_style));
        lines.push(Line::from(""));
    }

    let hint = if unmerged {
        "  k keep · D force-delete · Esc cancel"
    } else {
        "  k keep · d delete · Esc cancel"
    };
    lines.push(Line::from(hint));

    frame.render_widget(Paragraph::new(lines), inner);
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
    fn confirm_remove_modal_names_the_repo() {
        let mut app = AppState::fixture();
        app.update(AppMessage::OpenConfirmRemoveRepo);
        insta::assert_snapshot!(render_to_string(&app));
    }
}
