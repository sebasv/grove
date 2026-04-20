// Integration tests for top-level UI rendering. Internal modules are tested
// per-module; this file exercises the public render entry point via TestBackend.

use ratatui::{backend::TestBackend, Terminal};

#[test]
fn empty_frame_renders_cleanly() {
    let backend = TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|_frame| {}).unwrap();
    insta::assert_snapshot!(terminal.backend());
}
