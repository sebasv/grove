//! Embedded terminal backed by a PTY, a vt100 parser, and a reader thread.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::async_evt::{Event, EventSender, WorktreeId};

/// Lightweight activity signals captured by the vt100 parser as it
/// processes PTY output.  Sidebar reads this aggregated across a
/// worktree's terminals to show whether something in there is doing
/// work or asking for attention.
#[derive(Debug, Default, Clone)]
pub struct TerminalActivity {
    /// True after the shell emitted a BEL (0x07) and false once the user
    /// focuses this terminal again.  High-confidence "needs attention"
    /// signal — Claude Code, gh CLI prompts, and most readline tools
    /// ring the bell when waiting on input.
    pub bell_pending: bool,
    /// Most recent OSC 0 / OSC 2 window title set by the shell.  Many
    /// TUIs (Claude Code, oh-my-zsh, gum, …) prefix it with a spinner
    /// glyph while working and a static glyph while idle, which gives
    /// us a precise "thinking" signal — see [`title_is_thinking`].
    pub title: String,
    /// Wall-clock instant of the most recent PTY byte we observed.
    /// Used as a low-precision fallback for TUIs that don't announce
    /// state via the window title.
    pub last_output_at: Option<Instant>,
}

/// `vt100::Callbacks` impl that funnels relevant events into the shared
/// activity slot.  Held by the parser; mutated by `Parser::process`
/// without grove having to scan bytes itself.
pub(crate) struct GroveCallbacks {
    activity: Arc<Mutex<TerminalActivity>>,
}

impl vt100::Callbacks for GroveCallbacks {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        if let Ok(mut a) = self.activity.lock() {
            a.title = String::from_utf8_lossy(title).into_owned();
        }
    }

    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        if let Ok(mut a) = self.activity.lock() {
            a.bell_pending = true;
        }
    }
}

pub struct Terminal {
    pub parser: Arc<Mutex<vt100::Parser<GroveCallbacks>>>,
    pub activity: Arc<Mutex<TerminalActivity>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child_killer: Box<dyn ChildKiller + Send + Sync>,
    last_size: PtySize,
}

/// Does the shell's most recent window title look like a "thinking" cue?
/// Claude Code's title cycles through braille pattern glyphs (U+2800–
/// U+28FF) while the assistant is producing output, and switches to a
/// static `✳` star when idle.  We accept any leading braille character,
/// which also catches oh-my-zsh, gum, and any other TUI spinner that
/// uses the same code-point block.
pub fn title_is_thinking(title: &str) -> bool {
    title
        .chars()
        .next()
        .is_some_and(|c| ('\u{2800}'..='\u{28FF}').contains(&c))
}

impl Terminal {
    pub fn spawn(cwd: &Path, size: PtySize, wt_id: WorktreeId, tx: EventSender) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(size).context("opening pty")?;

        let shell = std::env::var_os("SHELL")
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/bin/sh".to_string());

        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("-l");
        cmd.cwd(cwd);
        if let Some(term) = std::env::var_os("TERM") {
            cmd.env("TERM", term);
        } else {
            cmd.env("TERM", "xterm-256color");
        }

        let child = pair.slave.spawn_command(cmd).context("spawning shell")?;
        let child_killer = child.clone_killer();
        // Drop the slave side so the child can exit cleanly on EOF.
        drop(pair.slave);
        // We don't need to await the child for v0.4 — dropping the killer will
        // SIGKILL it on exit.  The `_child` holder must stay alive or the OS
        // reaps it immediately; we keep the killer which internally holds the
        // handle.
        std::mem::forget(child);

        let master = pair.master;
        let writer = Arc::new(Mutex::new(
            master.take_writer().context("taking pty writer")?,
        ));
        let reader = master.try_clone_reader().context("cloning pty reader")?;

        let activity = Arc::new(Mutex::new(TerminalActivity::default()));
        let callbacks = GroveCallbacks {
            activity: Arc::clone(&activity),
        };
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            size.rows, size.cols, 10_000, callbacks,
        )));

        spawn_reader_thread(
            reader,
            parser.clone(),
            Arc::clone(&writer),
            Arc::clone(&activity),
            wt_id,
            tx,
        );

        Ok(Self {
            parser,
            activity,
            writer,
            master,
            child_killer,
            last_size: size,
        })
    }

    /// Clear the bell-pending flag — called when the user focuses this
    /// terminal so the "needs attention" indicator goes away.
    pub fn clear_bell(&self) {
        if let Ok(mut a) = self.activity.lock() {
            a.bell_pending = false;
        }
    }

    pub fn activity_snapshot(&self) -> TerminalActivity {
        self.activity.lock().map(|a| a.clone()).unwrap_or_default()
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()
    }

    pub fn resize(&mut self, size: PtySize) -> Result<()> {
        if size == self.last_size {
            return Ok(());
        }
        self.master.resize(size).context("resizing pty")?;
        self.parser
            .lock()
            .unwrap()
            .screen_mut()
            .set_size(size.rows, size.cols);
        self.last_size = size;
        Ok(())
    }
}

/// Extract the text under a rectangular region of the rendered terminal,
/// at the given scrollback offset.  `start` and `end` are `(col, row)`
/// pairs in row-major order (start <= end).  Returns None on lock failure.
///
/// This temporarily moves the parser's scrollback window so the read sees
/// the same rows the user had highlighted; the next render call resets the
/// offset, so this side effect is transparent.
pub fn read_selection(
    term: &Terminal,
    start: (u16, u16),
    end: (u16, u16),
    scrollback: usize,
) -> Option<String> {
    let mut parser = term.parser.lock().ok()?;
    parser.screen_mut().set_scrollback(scrollback);
    let screen = parser.screen();
    // vt100 takes rows first, then cols.  We extend `end_col` by 1 so the
    // user-visible end column is included (contents_between's end is
    // exclusive).
    let (start_row, start_col) = (start.1, start.0);
    let (end_row, mut end_col) = (end.1, end.0);
    let (_rows, cols) = screen.size();
    if end_col < cols {
        end_col += 1;
    }
    Some(screen.contents_between(start_row, start_col, end_row, end_col))
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Best-effort kill — ignore errors (child may already be dead).
        let _ = self.child_killer.kill();
    }
}

fn spawn_reader_thread(
    mut reader: Box<dyn std::io::Read + Send>,
    parser: Arc<Mutex<vt100::Parser<GroveCallbacks>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    activity: Arc<Mutex<TerminalActivity>>,
    wt_id: WorktreeId,
    tx: EventSender,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    // Detect ESC[6n (Device Status Report — cursor position query).
                    // Applications like atuin send this to learn where they are on
                    // screen.  The vt100 parser consumes it silently, so we must
                    // synthesise the CPR response (ESC[row;colR) and inject it back
                    // into the PTY before the application times out waiting.
                    let has_cpr = buf[..n].windows(4).any(|w| w == b"\x1b[6n");
                    if let Ok(mut p) = parser.lock() {
                        // Bell + window-title updates are picked up by
                        // GroveCallbacks during parser.process(); no
                        // byte-scanning needed on our side.
                        p.process(&buf[..n]);
                        if has_cpr {
                            let pos = p.screen().cursor_position();
                            let row = pos.0 + 1;
                            let col = pos.1 + 1;
                            let resp = format!("\x1b[{row};{col}R");
                            if let Ok(mut w) = writer.lock() {
                                let _ = w.write_all(resp.as_bytes());
                                let _ = w.flush();
                            }
                        }
                    }
                    if let Ok(mut a) = activity.lock() {
                        a.last_output_at = Some(Instant::now());
                    }
                    let _ = wt_id;
                    if tx.send(Event::TerminalOutput).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

/// Build PTY input bytes for a crossterm KeyEvent. Returns None when the key
/// has no useful PTY representation (e.g., unmodified function keys we don't
/// map yet).
pub fn key_to_pty_bytes(key: crossterm::event::KeyEvent) -> Option<Vec<u8>> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    let mut out = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                let upper = c.to_ascii_uppercase();
                if (b'@'..=b'_').contains(&(upper as u8)) {
                    vec![(upper as u8) - 0x40]
                } else if upper == ' ' {
                    // Ctrl+Space is reserved at the grove layer.
                    return None;
                } else {
                    let mut bytes = [0u8; 4];
                    let s = c.encode_utf8(&mut bytes);
                    s.as_bytes().to_vec()
                }
            } else {
                let mut bytes = [0u8; 4];
                let s = c.encode_utf8(&mut bytes);
                s.as_bytes().to_vec()
            }
        }
        KeyCode::Enter => {
            // Shift+Enter sends LF; TUIs like Claude Code use this as
            // newline-in-input while plain Enter (CR) is "submit".  The
            // kitty u-style escape was correct but only kitty-aware apps
            // decode it, so most users got nothing.
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                b"\n".to_vec()
            } else {
                b"\r".to_vec()
            }
        }
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => b"\x7f".to_vec(),
        KeyCode::Esc => b"\x1b".to_vec(),
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => return None,
        },
        _ => return None,
    };

    if alt && !out.is_empty() && out[0] != 0x1b {
        out.insert(0, 0x1b);
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        }
    }

    fn key_with(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: mods,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::empty(),
        }
    }

    #[test]
    fn plain_char_encodes_as_utf8() {
        assert_eq!(key_to_pty_bytes(key(KeyCode::Char('a'))), Some(vec![b'a']));
        assert_eq!(
            key_to_pty_bytes(key(KeyCode::Char('ñ'))),
            Some("ñ".as_bytes().to_vec())
        );
    }

    #[test]
    fn ctrl_c_encodes_as_0x03() {
        let k = key_with(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(key_to_pty_bytes(k), Some(vec![0x03]));
    }

    #[test]
    fn ctrl_d_encodes_as_0x04() {
        let k = key_with(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(key_to_pty_bytes(k), Some(vec![0x04]));
    }

    #[test]
    fn ctrl_space_is_consumed_by_grove() {
        let k = key_with(KeyCode::Char(' '), KeyModifiers::CONTROL);
        assert_eq!(key_to_pty_bytes(k), None);
    }

    #[test]
    fn arrow_keys_use_vt_sequences() {
        assert_eq!(key_to_pty_bytes(key(KeyCode::Up)), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            key_to_pty_bytes(key(KeyCode::Left)),
            Some(b"\x1b[D".to_vec())
        );
    }

    #[test]
    fn enter_backspace_tab_have_classic_bytes() {
        assert_eq!(key_to_pty_bytes(key(KeyCode::Enter)), Some(b"\r".to_vec()));
        assert_eq!(
            key_to_pty_bytes(key(KeyCode::Backspace)),
            Some(b"\x7f".to_vec())
        );
        assert_eq!(key_to_pty_bytes(key(KeyCode::Tab)), Some(b"\t".to_vec()));
    }

    #[test]
    fn shift_enter_emits_lf_for_newline_in_input() {
        let k = key_with(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(key_to_pty_bytes(k), Some(b"\n".to_vec()));
    }

    #[test]
    fn alt_prefix_emits_escape() {
        let k = key_with(KeyCode::Char('b'), KeyModifiers::ALT);
        assert_eq!(key_to_pty_bytes(k), Some(vec![0x1b, b'b']));
    }

    #[test]
    fn f_keys_have_standard_sequences() {
        assert_eq!(
            key_to_pty_bytes(key(KeyCode::F(1))),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            key_to_pty_bytes(key(KeyCode::F(5))),
            Some(b"\x1b[15~".to_vec())
        );
    }

    #[test]
    fn title_thinking_recognises_braille_spinner_glyphs() {
        // Real samples captured from Claude Code while streaming.
        assert!(title_is_thinking("⠂ Claude Code"));
        assert!(title_is_thinking("⠐ Run ls command in root directory"));
        assert!(title_is_thinking("⡀ anything"));
        // U+28FF is the last code point in the braille block.
        assert!(title_is_thinking("\u{28FF} edge"));
    }

    #[test]
    fn title_thinking_rejects_idle_and_unrelated_glyphs() {
        // `✳` (U+2733) is Claude Code's static "idle" prefix.
        assert!(!title_is_thinking("✳ Claude Code"));
        assert!(!title_is_thinking("✳ Say hi"));
        // Plain text titles (a normal shell, vim, etc.) are not thinking.
        assert!(!title_is_thinking("~/dev/grove — zsh"));
        assert!(!title_is_thinking(""));
        // U+2800 is the start of braille; U+27FF is one before — must
        // not be treated as a spinner glyph.
        assert!(!title_is_thinking("\u{27FF} not braille"));
    }
}
