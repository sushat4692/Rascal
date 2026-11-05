//! Color presets for the diff viewer.
//!
//! The presets are app-global and independent of the window's light/dark mode.
//! The actual colors are resolved against the current theme at render time.

use gpui::{Global, Hsla, hsla};
use gpui_component::theme::Theme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffTheme {
    #[default]
    Standard,
    HighContrast,
    Colorblind,
}

impl Global for DiffTheme {}

impl DiffTheme {
    pub fn label(self) -> &'static str {
        match self {
            DiffTheme::Standard => "標準",
            DiffTheme::HighContrast => "高コントラスト",
            DiffTheme::Colorblind => "色覚配慮",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            DiffTheme::Standard => "standard",
            DiffTheme::HighContrast => "high-contrast",
            DiffTheme::Colorblind => "colorblind",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "standard" => Some(DiffTheme::Standard),
            "high-contrast" => Some(DiffTheme::HighContrast),
            "colorblind" => Some(DiffTheme::Colorblind),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub struct DiffPalette {
    pub added_bg: Hsla,
    pub removed_bg: Hsla,
    pub added_fg: Hsla,
    pub removed_fg: Hsla,
    pub added_mark: Hsla,
    pub removed_mark: Hsla,
    pub hunk_bg: Hsla,
    pub hunk_fg: Hsla,
    pub context_fg: Hsla,
    pub line_no: Hsla,
}

pub fn palette(theme: &Theme, diff: DiffTheme) -> DiffPalette {
    let dark = theme.is_dark();
    match diff {
        DiffTheme::Standard => DiffPalette {
            added_bg: theme.success.opacity(if dark { 0.30 } else { 0.15 }),
            removed_bg: theme.danger.opacity(if dark { 0.30 } else { 0.15 }),
            added_fg: theme.foreground,
            removed_fg: theme.foreground,
            added_mark: if dark {
                hsla(0.38, 0.72, 0.64, 1.0)
            } else {
                theme.success
            },
            removed_mark: if dark {
                hsla(0.0, 0.78, 0.68, 1.0)
            } else {
                theme.danger
            },
            hunk_bg: theme.muted.opacity(if dark { 0.8 } else { 0.6 }),
            hunk_fg: theme.link,
            context_fg: theme.foreground,
            line_no: theme.muted_foreground,
        },
        DiffTheme::HighContrast if dark => DiffPalette {
            added_bg: hsla(0.38, 0.65, 0.25, 0.62),
            removed_bg: hsla(0.0, 0.65, 0.28, 0.62),
            added_fg: hsla(0.38, 0.80, 0.88, 1.0),
            removed_fg: hsla(0.0, 0.85, 0.88, 1.0),
            added_mark: hsla(0.38, 0.85, 0.92, 1.0),
            removed_mark: hsla(0.0, 0.90, 0.92, 1.0),
            hunk_bg: hsla(0.60, 0.25, 0.22, 1.0),
            hunk_fg: hsla(0.60, 0.90, 0.85, 1.0),
            context_fg: hsla(0.0, 0.0, 0.95, 1.0),
            line_no: hsla(0.0, 0.0, 0.82, 1.0),
        },
        DiffTheme::HighContrast => DiffPalette {
            added_bg: hsla(0.38, 0.70, 0.88, 0.92),
            removed_bg: hsla(0.0, 0.70, 0.90, 0.92),
            added_fg: hsla(0.38, 0.80, 0.20, 1.0),
            removed_fg: hsla(0.0, 0.80, 0.28, 1.0),
            added_mark: hsla(0.38, 0.80, 0.24, 1.0),
            removed_mark: hsla(0.0, 0.80, 0.32, 1.0),
            hunk_bg: hsla(0.60, 0.60, 0.90, 1.0),
            hunk_fg: hsla(0.60, 0.70, 0.25, 1.0),
            context_fg: hsla(0.0, 0.0, 0.12, 1.0),
            line_no: hsla(0.0, 0.0, 0.35, 1.0),
        },
        DiffTheme::Colorblind if dark => DiffPalette {
            added_bg: hsla(0.60, 0.80, 0.35, 0.40),
            removed_bg: hsla(0.08, 0.90, 0.38, 0.42),
            added_fg: theme.foreground,
            removed_fg: theme.foreground,
            added_mark: hsla(0.60, 0.90, 0.74, 1.0),
            removed_mark: hsla(0.08, 0.95, 0.70, 1.0),
            hunk_bg: hsla(0.75, 0.20, 0.22, 1.0),
            hunk_fg: hsla(0.75, 0.80, 0.85, 1.0),
            context_fg: theme.foreground,
            line_no: hsla(0.0, 0.0, 0.74, 1.0),
        },
        DiffTheme::Colorblind => DiffPalette {
            added_bg: hsla(0.60, 0.80, 0.90, 0.92),
            removed_bg: hsla(0.08, 0.90, 0.92, 0.95),
            added_fg: hsla(0.0, 0.0, 0.12, 1.0),
            removed_fg: hsla(0.0, 0.0, 0.12, 1.0),
            added_mark: hsla(0.60, 0.80, 0.35, 1.0),
            removed_mark: hsla(0.08, 0.90, 0.40, 1.0),
            hunk_bg: hsla(0.75, 0.40, 0.92, 1.0),
            hunk_fg: hsla(0.75, 0.60, 0.28, 1.0),
            context_fg: hsla(0.0, 0.0, 0.12, 1.0),
            line_no: hsla(0.0, 0.0, 0.38, 1.0),
        },
    }
}
