//! RealtyLens palette tokens.
//!
//! Hex values are taken verbatim from `docs/realtylens-theme.txt`; do not
//! drift from that file without updating both. Lights and darks live in
//! the same struct so `app/src/themes/palette.rs` can pick at runtime
//! based on [`ThemeMode`].

use serde::{Deserialize, Serialize};

/// 24-bit RGB color with optional alpha (0–255).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 0xFF }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Hex constructor accepting `0xRRGGBB` integers — handy for `pub const`.
    pub const fn hex(rgb: u32) -> Self {
        Self::rgb(
            ((rgb >> 16) & 0xFF) as u8,
            ((rgb >> 8) & 0xFF) as u8,
            (rgb & 0xFF) as u8,
        )
    }

    /// Format as `#RRGGBB` — useful for handing tokens to a CSS-like
    /// downstream (the editor block markup uses this).
    pub fn to_hex_string(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    /// Mix `self` toward `other` by `t` (clamped to 0..=1).
    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| -> u8 {
            (a as f32 * (1.0 - t) + b as f32 * t)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        Color {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }
}

/// Light / dark variant selector. The crate ships both in [`Palette`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    Light,
    Dark,
}

/// The seven editorial chip accents from the RealtyLens spec.
///
/// `IDE semantic` column matches the plan in
/// `~/.claude/plans/i-want-to-create-linear-taco.md` — keep in sync if the
/// chip → semantic mapping changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChipPalette {
    /// `--chip-sage` — claude-ex / search / indexing surfaces.
    pub sage: Color,
    /// `--chip-sky` — focused agent halo, terminal selection.
    pub sky: Color,
    /// `--chip-lavender` — palette accents, shortcut chips.
    pub lavender: Color,
    /// `--chip-blush` — diff "removed" backgrounds.
    pub blush: Color,
    /// `--chip-butter` — diff "modified" backgrounds.
    pub butter: Color,
    /// `--chip-orchid` — pane lineage edges + pane-id badges.
    pub orchid: Color,
    /// `--chip-peach` — empty-state halos.
    pub peach: Color,
}

impl ChipPalette {
    pub const fn realtylens() -> Self {
        Self {
            sage: Color::hex(0xE9EFD0),
            sky: Color::hex(0xE0F1F8),
            lavender: Color::hex(0xEADFFD),
            blush: Color::hex(0xF4DADA),
            butter: Color::hex(0xF4F1DA),
            orchid: Color::hex(0xF6E5F3),
            peach: Color::hex(0xFFE2D4),
        }
    }
}

/// Full palette for one [`ThemeMode`]. Copy semantics so we can stash it
/// in app state without a lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub mode: ThemeMode,

    // Surfaces — see realtylens-theme.txt §2.
    pub bg: Color,
    pub bg_alt: Color,
    pub bg_card: Color,
    pub bg_elevated: Color,
    pub bg_dark: Color,
    pub bg_dark_alt: Color,

    // Borders.
    pub border: Color,
    pub border_dark: Color,
    pub border_subtle: Color,

    // Accent (pure ink — primary CTA is black).
    pub accent: Color,
    pub accent_light: Color,
    pub accent_dark: Color,

    // Status.
    pub green: Color,
    pub green_light: Color,
    pub warm: Color,
    pub warm_light: Color,
    pub teal: Color,
    pub teal_dark: Color,

    // Text.
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_muted: Color,
    pub text_on_dark: Color,

    // Editorial chip accents.
    pub chips: ChipPalette,
}

impl Palette {
    /// Light palette (default first-run mode). Values from §2 of the
    /// RealtyLens spec.
    pub const fn realtylens_light() -> Self {
        Self {
            mode: ThemeMode::Light,

            bg: Color::hex(0xFEFFFC),
            bg_alt: Color::hex(0xF9FAF7),
            bg_card: Color::hex(0xFFFFFF),
            bg_elevated: Color::hex(0xEEF1ED),
            bg_dark: Color::hex(0x171717),
            bg_dark_alt: Color::hex(0x000000),

            border: Color::hex(0xDEE2DE),
            border_dark: Color::hex(0xB4B8B4),
            border_subtle: Color::hex(0xEEF1ED),

            accent: Color::hex(0x171717),
            accent_light: Color::hex(0x2C2C2C),
            accent_dark: Color::hex(0x000000),

            green: Color::hex(0x10A37F),
            green_light: Color::hex(0x1EC99F),
            warm: Color::hex(0xEF4444),
            warm_light: Color::hex(0xF87171),
            teal: Color::hex(0x3B82F6),
            teal_dark: Color::hex(0x0072CC),

            text_primary: Color::hex(0x2C2C2C),
            text_secondary: Color::hex(0x525252),
            text_tertiary: Color::hex(0x7A7A7A),
            text_muted: Color::hex(0xA8A8A8),
            text_on_dark: Color::hex(0xFEFFFC),

            chips: ChipPalette::realtylens(),
        }
    }

    /// Dark companion: surfaces flipped to charcoal, accents brightened
    /// just enough to read on the dark surfaces while still feeling warm.
    /// Chip palette unchanged — they read as soft accents on dark too.
    pub const fn realtylens_dark() -> Self {
        Self {
            mode: ThemeMode::Dark,

            bg: Color::hex(0x171717),
            bg_alt: Color::hex(0x1F1F1E),
            bg_card: Color::hex(0x232322),
            bg_elevated: Color::hex(0x2C2C2C),
            bg_dark: Color::hex(0x000000),
            bg_dark_alt: Color::hex(0x000000),

            border: Color::hex(0x2C2C2C),
            border_dark: Color::hex(0x4A4A4A),
            border_subtle: Color::hex(0x232322),

            // On dark, the CTA flips to off-white ink.
            accent: Color::hex(0xFEFFFC),
            accent_light: Color::hex(0xE6E7E2),
            accent_dark: Color::hex(0xFFFFFF),

            green: Color::hex(0x1EC99F),
            green_light: Color::hex(0x39E5BD),
            warm: Color::hex(0xF87171),
            warm_light: Color::hex(0xFCA5A5),
            teal: Color::hex(0x60A5FA),
            teal_dark: Color::hex(0x3B82F6),

            text_primary: Color::hex(0xFEFFFC),
            text_secondary: Color::hex(0xB4B4B4),
            text_tertiary: Color::hex(0x8B8B8B),
            text_muted: Color::hex(0x6A6A6A),
            text_on_dark: Color::hex(0xFEFFFC),

            chips: ChipPalette::realtylens(),
        }
    }

    pub const fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self::realtylens_light(),
            ThemeMode::Dark => Self::realtylens_dark(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_palette_uses_pure_black_cta() {
        // RealtyLens §2: "primary CTA is black, NOT blue". Catch any
        // accidental drift to the legacy cobalt accent here.
        let p = Palette::realtylens_light();
        assert_eq!(p.accent, Color::hex(0x171717));
        assert_eq!(p.accent_dark, Color::hex(0x000000));
        assert_ne!(p.accent, p.teal, "CTA must not equal info blue");
    }

    #[test]
    fn dark_palette_inverts_text_and_surfaces() {
        let l = Palette::realtylens_light();
        let d = Palette::realtylens_dark();
        assert_ne!(l.bg, d.bg);
        assert_ne!(l.text_primary, d.text_primary);
    }

    #[test]
    fn chip_palette_has_seven_distinct_colors() {
        let c = ChipPalette::realtylens();
        let all = [
            c.sage, c.sky, c.lavender, c.blush, c.butter, c.orchid, c.peach,
        ];
        for i in 0..all.len() {
            for j in (i + 1)..all.len() {
                assert_ne!(all[i], all[j], "chips must all differ");
            }
        }
    }

    #[test]
    fn hex_string_roundtrips() {
        assert_eq!(Color::hex(0x10A37F).to_hex_string(), "#10A37F");
    }

    #[test]
    fn mix_is_linear() {
        let c = Color::hex(0x000000).mix(Color::hex(0xFFFFFF), 0.5);
        assert!((c.r as i32 - 128).abs() <= 1);
    }
}
