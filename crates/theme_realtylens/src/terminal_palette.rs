//! 16-slot ANSI palette derived from RealtyLens tokens.
//!
//! Claude Code's TUI emits ANSI color codes; rendering them on the warm
//! off-white surface needs a palette tuned for that backdrop instead of
//! the usual deep-dark terminal default. The bright variants stay close
//! to the regular ones — RealtyLens prefers low-contrast harmony over
//! 8-color punch.

use crate::palette::{Color, ThemeMode};

/// 16 ANSI colors plus background / foreground / cursor / selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnsiPalette {
    pub background: Color,
    pub foreground: Color,
    pub cursor: Color,
    pub selection_bg: Color,

    pub black: Color,
    pub red: Color,
    pub green: Color,
    pub yellow: Color,
    pub blue: Color,
    pub magenta: Color,
    pub cyan: Color,
    pub white: Color,

    pub bright_black: Color,
    pub bright_red: Color,
    pub bright_green: Color,
    pub bright_yellow: Color,
    pub bright_blue: Color,
    pub bright_magenta: Color,
    pub bright_cyan: Color,
    pub bright_white: Color,
}

impl AnsiPalette {
    /// Light: warm off-white background, charcoal foreground.
    pub const fn realtylens_light() -> Self {
        Self {
            background: Color::hex(0xFFFFFF),
            foreground: Color::hex(0x2C2C2C),
            cursor: Color::hex(0x171717),
            selection_bg: Color::hex(0xE0F1F8), // chip-sky

            black: Color::hex(0x2C2C2C),
            red: Color::hex(0xEF4444),
            green: Color::hex(0x10A37F),
            yellow: Color::hex(0xB58B00),
            blue: Color::hex(0x3B82F6),
            magenta: Color::hex(0xA855F7),
            cyan: Color::hex(0x0072CC),
            white: Color::hex(0x525252),

            bright_black: Color::hex(0x7A7A7A),
            bright_red: Color::hex(0xF87171),
            bright_green: Color::hex(0x1EC99F),
            bright_yellow: Color::hex(0xD4A017),
            bright_blue: Color::hex(0x60A5FA),
            bright_magenta: Color::hex(0xC084FC),
            bright_cyan: Color::hex(0x22B5E0),
            bright_white: Color::hex(0x2C2C2C),
        }
    }

    /// Dark: charcoal background, off-white foreground.
    pub const fn realtylens_dark() -> Self {
        Self {
            background: Color::hex(0x171717),
            foreground: Color::hex(0xFEFFFC),
            cursor: Color::hex(0xFEFFFC),
            selection_bg: Color::hex(0x2C2C2C),

            black: Color::hex(0x171717),
            red: Color::hex(0xF87171),
            green: Color::hex(0x1EC99F),
            yellow: Color::hex(0xD4A017),
            blue: Color::hex(0x60A5FA),
            magenta: Color::hex(0xC084FC),
            cyan: Color::hex(0x22B5E0),
            white: Color::hex(0xB4B4B4),

            bright_black: Color::hex(0x4A4A4A),
            bright_red: Color::hex(0xFCA5A5),
            bright_green: Color::hex(0x39E5BD),
            bright_yellow: Color::hex(0xE9C04A),
            bright_blue: Color::hex(0x93C5FD),
            bright_magenta: Color::hex(0xD8B4FE),
            bright_cyan: Color::hex(0x67D6F0),
            bright_white: Color::hex(0xFEFFFC),
        }
    }

    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self::realtylens_light(),
            ThemeMode::Dark => Self::realtylens_dark(),
        }
    }

    /// Convenience iterator over the eight base ANSI colors as
    /// `(name, color)` pairs — handy when generating warp_terminal config
    /// or rendering a swatch.
    pub fn base_pairs(&self) -> [(&'static str, Color); 8] {
        [
            ("black", self.black),
            ("red", self.red),
            ("green", self.green),
            ("yellow", self.yellow),
            ("blue", self.blue),
            ("magenta", self.magenta),
            ("cyan", self.cyan),
            ("white", self.white),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_palette_has_high_enough_contrast_between_bg_and_fg() {
        // Sanity: warm off-white bg should not collide with charcoal fg.
        let p = AnsiPalette::realtylens_light();
        assert_ne!(p.background, p.foreground);
        // Quick perceived-luminance check: bg should be much lighter.
        let lum = |c: Color| 0.2126 * c.r as f32 + 0.7152 * c.g as f32 + 0.0722 * c.b as f32;
        assert!(lum(p.background) - lum(p.foreground) > 100.0);
    }

    #[test]
    fn for_mode_picks_correct_variant() {
        assert_eq!(
            AnsiPalette::for_mode(ThemeMode::Light),
            AnsiPalette::realtylens_light()
        );
        assert_eq!(
            AnsiPalette::for_mode(ThemeMode::Dark),
            AnsiPalette::realtylens_dark()
        );
    }
}
