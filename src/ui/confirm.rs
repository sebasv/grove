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

pub fn render_remove_worktree(frame: &mut Frame, area: Rect, branch: &str) {
    let modal_area = centered_rect(58, 8, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Remove worktree? ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(""),
        Line::from(format!("  Remove worktree for \"{branch}\"?")),
        Line::from("  (runs `git worktree remove`; branch is kept)"),
        Line::from(""),
        Line::from("  y/Enter confirm  ·  n/Esc cancel"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

pub fn render_delete_branch(frame: &mut Frame, area: Rect, branch: &str, pr_number: Option<u32>) {
    let height = if pr_number.is_some() { 10u16 } else { 8 };
    let modal_area = centered_rect(64, height, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Delete branch? ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut lines = vec![
        Line::from(""),
        Line::from(format!("  Also delete branch \"{branch}\" locally?")),
        Line::from(""),
    ];
    if let Some(n) = pr_number {
        lines.push(Line::styled(
            format!("  Warning: PR #{n} is open — deleting will close it."),
            Style::default().fg(Color::Yellow),
        ));
        lines.push(Line::from(""));
    }
    lines.push(Line::from("  y/Enter confirm  ·  n/Esc skip"));
    frame.render_widget(Paragraph::new(lines), inner);
}

pub fn render_force_delete_branch(frame: &mut Frame, area: Rect, branch: &str) {
    let modal_area = centered_rect(64, 8, area);
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Branch has unmerged commits ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(""),
        Line::from(format!("  \"{branch}\" has unmerged commits.")),
        Line::from("  Force delete (data loss)?"),
        Line::from(""),
        Line::from("  y/Enter force delete  ·  n/Esc cancel"),
    ];
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
