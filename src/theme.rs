use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Runtime color palette.  A small set of semantic slots — widgets pull from
/// these rather than hard-coding colors so future additions stay consistent.
#[allow(dead_code)] // dim/success/warn/danger are read from widgets added later in 1.x
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub accent: Color,
    pub dim: Color,
    pub success: Color,
    pub warn: Color,
    pub danger: Color,
    /// Background for a focused text input — lets the user see where
    /// keystrokes will land without reading hint text.
    pub input_bg_focused: Color,
    /// Background for an unfocused text input (e.g. the NewWorktree
    /// modal's input row while the branch list is active).
    pub input_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // Matches the hard-coded choices used in v0.x so the 1.0 default
        // looks identical to what users have today.
        Self {
            accent: Color::Cyan,
            dim: Color::DarkGray,
            success: Color::Green,
            warn: Color::Yellow,
            danger: Color::Red,
            input_bg_focused: Color::Rgb(40, 44, 52),
            input_bg: Color::Rgb(28, 30, 36),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeName {
    #[default]
    Default,
    Tokyonight,
    Gruvbox,
}

impl ThemeName {
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::Tokyonight,
            Self::Tokyonight => Self::Gruvbox,
            Self::Gruvbox => Self::Default,
        }
    }
}

pub fn resolve(name: ThemeName) -> Theme {
    match name {
        ThemeName::Default => Theme::default(),
        ThemeName::Tokyonight => Theme {
            accent: Color::Rgb(122, 162, 247),
            dim: Color::Rgb(86, 95, 137),
            success: Color::Rgb(158, 206, 106),
            warn: Color::Rgb(224, 175, 104),
            danger: Color::Rgb(247, 118, 142),
            input_bg_focused: Color::Rgb(36, 40, 59),
            input_bg: Color::Rgb(26, 29, 43),
        },
        ThemeName::Gruvbox => Theme {
            accent: Color::Rgb(250, 189, 47),
            dim: Color::Rgb(146, 131, 116),
            success: Color::Rgb(184, 187, 38),
            warn: Color::Rgb(254, 128, 25),
            danger: Color::Rgb(251, 73, 52),
            input_bg_focused: Color::Rgb(60, 56, 54),
            input_bg: Color::Rgb(40, 40, 40),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_maps_each_name() {
        assert_eq!(resolve(ThemeName::Default).accent, Color::Cyan);
        assert!(matches!(
            resolve(ThemeName::Tokyonight).accent,
            Color::Rgb(..)
        ));
        assert!(matches!(resolve(ThemeName::Gruvbox).accent, Color::Rgb(..)));
    }
}
