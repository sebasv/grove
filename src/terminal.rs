//! Embedded terminal backed by a PTY, a vt100 parser, and a reader thread.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::async_evt::{Event, EventSender, WorktreeId};

pub struct Terminal {
    pub parser: Arc<Mutex<vt100::Parser>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child_killer: Box<dyn ChildKiller + Send + Sync>,
    last_size: PtySize,
}

impl Terminal {
    pub fn spawn(
        cwd: &Path,
        size: PtySize,
        wt_id: WorktreeId,
        tx: EventSender,
    ) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(size)
            .context("opening pty")?;

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

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("spawning shell")?;
        let child_killer = child.clone_killer();
        // Drop the slave side so the child can exit cleanly on EOF.
        drop(pair.slave);
        // We don't need to await the child for v0.4 — dropping the killer will
        // SIGKILL it on exit.  The `_child` holder must stay alive or the OS
        // reaps it immediately; we keep the killer which internally holds the
        // handle.
        std::mem::forget(child);

        let master = pair.master;
        let writer = master
            .take_writer()
            .context("taking pty writer")?;
        let reader = master
            .try_clone_reader()
            .context("cloning pty reader")?;

        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            size.rows,
            size.cols,
            0,
        )));

        spawn_reader_thread(reader, parser.clone(), wt_id, tx);

        Ok(Self {
            parser,
            writer,
            master,
            child_killer,
            last_size: size,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&mut self, size: PtySize) -> Result<()> {
        if size == self.last_size {
            return Ok(());
        }
        self.master
            .resize(size)
            .context("resizing pty")?;
        self.parser
            .lock()
            .unwrap()
            .set_size(size.rows, size.cols);
        self.last_size = size;
        Ok(())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Best-effort kill — ignore errors (child may already be dead).
        let _ = self.child_killer.kill();
    }
}

fn spawn_reader_thread(
    mut reader: Box<dyn std::io::Read + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    wt_id: WorktreeId,
    tx: EventSender,
) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF
                Ok(n) => {
                    if let Ok(mut p) = parser.lock() {
                        p.process(&buf[..n]);
                    }
                    let _ = wt_id; // reserved for v0.5 per-terminal routing
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
        KeyCode::Enter => b"\r".to_vec(),
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
        assert_eq!(
            key_to_pty_bytes(key(KeyCode::Up)),
            Some(b"\x1b[A".to_vec())
        );
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
}
