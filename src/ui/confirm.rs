use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::ui::{centered_rect, truncate_to_width, wrap_message};

const ERROR_MAX_LINES: usize = 3;

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
    let modal_width: u16 = 66;
    // Inner content width is modal_width - 2 (borders); error rows have a
    // 4-col gutter ("  ! " / "    "), other lines a 2-col left pad.
    let inner_width = (modal_width as usize).saturating_sub(2);
    let err_width = inner_width.saturating_sub(4);

    let err_lines: Vec<String> = error
        .map(|m| wrap_message(m, err_width.max(1), ERROR_MAX_LINES))
        .unwrap_or_default();

    // Body lines fixed: blank, repo, worktree, blank,
    //   [unmerged: warn (1 or 2) + blank],
    //   [k] option, [d|D] option, blank,
    //   [error: lines + blank],
    //   hint.
    let mut body_rows: u16 = 4 // blank + repo + worktree + blank
        + 2 // two option lines
        + 1 // blank before hint/error
        + 1; // hint
    if unmerged {
        body_rows += if pr_number.is_some() { 3 } else { 2 };
    }
    if !err_lines.is_empty() {
        body_rows += err_lines.len() as u16 + 1;
    }
    let height = body_rows + 2; // borders

    let modal_area = centered_rect(modal_width, height, area);
    frame.render_widget(Clear, modal_area);

    // Title is rendered inside the top border; truncate so a long branch
    // can't eat the closing border corner.
    let title_budget = (modal_width as usize).saturating_sub(4);
    let title = truncate_to_width(&format!(" Remove worktree for {branch} "), title_budget);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let dim = Style::default().fg(Color::DarkGray);
    let warn = Style::default().fg(Color::Yellow);
    let err_style = Style::default().fg(Color::Red);

    // Truncate path/branch in body lines to the inner content width minus
    // the labels — long absolute paths are common and silent clipping
    // hides the meaningful tail of the path.
    let repo_budget = inner_width.saturating_sub("  Repository: ".chars().count());
    let wt_budget = inner_width.saturating_sub("  Worktree:   ".chars().count());
    let branch_budget =
        inner_width.saturating_sub("  ⚠ \"\" has unmerged commits.".chars().count());

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(format!(
            "  Repository: {}",
            truncate_to_width(repo_name, repo_budget)
        )),
        Line::styled(
            format!(
                "  Worktree:   {}",
                truncate_to_width(&worktree_path.display().to_string(), wt_budget)
            ),
            dim,
        ),
        Line::from(""),
    ];

    if unmerged {
        let branch_shown = truncate_to_width(branch, branch_budget);
        lines.push(Line::styled(
            format!("  ⚠ \"{branch_shown}\" has unmerged commits."),
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

    lines.push(Line::from("  [k] Remove worktree, keep branch  (default)"));
    if unmerged {
        lines.push(Line::from(
            "  [D] Remove worktree and force-delete branch (-D)",
        ));
    } else {
        lines.push(Line::from("  [d] Remove worktree and delete branch (-d)"));
    }
    lines.push(Line::from(""));

    for (i, l) in err_lines.iter().enumerate() {
        let prefix = if i == 0 { "  ! " } else { "    " };
        lines.push(Line::styled(format!("{prefix}{l}"), err_style));
    }
    if !err_lines.is_empty() {
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
