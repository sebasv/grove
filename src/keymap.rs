use std::fmt;
use std::str::FromStr;

use crossterm::event::{KeyCode, KeyModifiers};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde::de;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl KeyChord {
    pub const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            mods: KeyModifiers::empty(),
        }
    }
    pub const fn ctrl(c: char) -> Self {
        Self {
            code: KeyCode::Char(c),
            mods: KeyModifiers::CONTROL,
        }
    }

    pub fn matches(&self, key: &crossterm::event::KeyEvent) -> bool {
        let key_mods = key
            .modifiers
            .intersection(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        let want_mods = self
            .mods
            .intersection(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        self.code == key.code && key_mods == want_mods
    }
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_string());
        }
        if self.mods.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_string());
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_string());
        }
        parts.push(format_keycode(self.code));
        write!(f, "{}", parts.join("+"))
    }
}

fn format_keycode(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::F(n) => format!("F{n}"),
        other => format!("{other:?}"),
    }
}

impl FromStr for KeyChord {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mods = KeyModifiers::empty();
        let mut parts = s.split('+').map(str::trim).collect::<Vec<_>>();
        if parts.is_empty() {
            return Err("empty key spec".into());
        }
        let key_str = parts.pop().unwrap();
        for p in parts {
            match p.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
                "alt" | "option" | "opt" | "meta" => mods |= KeyModifiers::ALT,
                "shift" => mods |= KeyModifiers::SHIFT,
                other => return Err(format!("unknown modifier: {other}")),
            }
        }
        let code = match key_str {
            "enter" | "Enter" | "Return" | "return" => KeyCode::Enter,
            "esc" | "Esc" | "escape" | "Escape" => KeyCode::Esc,
            "up" | "Up" => KeyCode::Up,
            "down" | "Down" => KeyCode::Down,
            "left" | "Left" => KeyCode::Left,
            "right" | "Right" => KeyCode::Right,
            "home" | "Home" => KeyCode::Home,
            "end" | "End" => KeyCode::End,
            "pageup" | "PageUp" | "pgup" => KeyCode::PageUp,
            "pagedown" | "PageDown" | "pgdn" => KeyCode::PageDown,
            "backspace" | "Backspace" => KeyCode::Backspace,
            "delete" | "Delete" => KeyCode::Delete,
            "tab" | "Tab" => KeyCode::Tab,
            "space" | "Space" => KeyCode::Char(' '),
            fk if (fk.starts_with('F') || fk.starts_with('f')) && fk.len() > 1 => {
                let n: u8 = fk[1..]
                    .parse()
                    .map_err(|_| format!("bad function-key spec: {fk}"))?;
                KeyCode::F(n)
            }
            one if one.chars().count() == 1 => {
                let c = one.chars().next().unwrap();
                KeyCode::Char(c)
            }
            other => return Err(format!("unknown key: {other}")),
        };
        Ok(KeyChord { code, mods })
    }
}

impl<'de> Deserialize<'de> for KeyChord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(de::Error::custom)
    }
}

impl Serialize for KeyChord {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Keymap {
    #[serde(default = "default_quit")]
    pub quit: KeyChord,
    #[serde(default = "default_help")]
    pub help: KeyChord,
    #[serde(default = "default_refresh")]
    pub refresh: KeyChord,
    #[serde(default = "default_add_repo")]
    pub add_repo: KeyChord,
    #[serde(default = "default_remove_repo")]
    pub remove_repo: KeyChord,
    #[serde(default = "default_new_worktree")]
    pub new_worktree: KeyChord,
    #[serde(default = "default_remove_worktree")]
    pub remove_worktree: KeyChord,
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            quit: default_quit(),
            help: default_help(),
            refresh: default_refresh(),
            add_repo: default_add_repo(),
            remove_repo: default_remove_repo(),
            new_worktree: default_new_worktree(),
            remove_worktree: default_remove_worktree(),
        }
    }
}

fn default_quit() -> KeyChord {
    KeyChord::plain(KeyCode::Char('q'))
}
fn default_help() -> KeyChord {
    KeyChord::plain(KeyCode::Char('?'))
}
fn default_refresh() -> KeyChord {
    KeyChord::plain(KeyCode::Char('r'))
}
fn default_add_repo() -> KeyChord {
    KeyChord::plain(KeyCode::Char('a'))
}
fn default_remove_repo() -> KeyChord {
    KeyChord::plain(KeyCode::Char('R'))
}
fn default_new_worktree() -> KeyChord {
    KeyChord::plain(KeyCode::Char('w'))
}
fn default_remove_worktree() -> KeyChord {
    KeyChord::plain(KeyCode::Char('W'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_char() {
        let c: KeyChord = "q".parse().unwrap();
        assert_eq!(c, KeyChord::plain(KeyCode::Char('q')));
    }

    #[test]
    fn parses_ctrl_modifier() {
        let c: KeyChord = "Ctrl+q".parse().unwrap();
        assert_eq!(c, KeyChord::ctrl('q'));
    }

    #[test]
    fn parses_alt_and_enter() {
        let c: KeyChord = "Alt+Enter".parse().unwrap();
        assert_eq!(c.code, KeyCode::Enter);
        assert!(c.mods.contains(KeyModifiers::ALT));
    }

    #[test]
    fn parses_function_key() {
        let c: KeyChord = "F5".parse().unwrap();
        assert_eq!(c.code, KeyCode::F(5));
    }

    #[test]
    fn rejects_unknown_modifier() {
        assert!("Hyper+q".parse::<KeyChord>().is_err());
    }

    #[test]
    fn roundtrips_through_display() {
        let c = KeyChord::ctrl('a');
        assert_eq!(c.to_string(), "Ctrl+a");
        let reparsed: KeyChord = c.to_string().parse().unwrap();
        assert_eq!(c, reparsed);
    }

    #[test]
    fn matches_same_modifiers() {
        let c = KeyChord::ctrl('c');
        let key = crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        );
        assert!(c.matches(&key));
    }

    #[test]
    fn deserialise_from_toml() {
        let config = r#"
            quit = "Ctrl+q"
            help = "F1"
        "#;
        let km: Keymap = toml::from_str(config).unwrap();
        assert_eq!(km.quit, KeyChord::ctrl('q'));
        assert_eq!(km.help.code, KeyCode::F(1));
        assert_eq!(km.refresh, default_refresh());
    }
}
