//! RealtyLens typography tokens.
//!
//! From the spec: Inter 16/1.6/300 for body, slightly tightened weights
//! for nav and brand wordmark. The Warp wire-in registers Inter with the
//! font loader and reads sizing from this module.

/// CSS-style font-family stack the Warp font resolver should fall through.
pub const FONT_FAMILY_INTER: &str = "Inter, system-ui, -apple-system, sans-serif";

/// Mono fallback for code panes (mirrors typical IDE conventions). Warp
/// already configures its terminal mono font; we only override when the
/// user explicitly opts in via Settings.
pub const FONT_FAMILY_MONO: &str = "Berkeley Mono, JetBrains Mono, SF Mono, Menlo, monospace";

/// Type scale derived from the spec.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Typography {
    pub family: &'static str,
    pub family_mono: &'static str,
    pub body_size_px: u16,
    pub body_line_height_x100: u16, // 160 == 1.60
    pub body_weight: u16,
    pub brand_size_px: u16,
    pub brand_weight: u16,
    pub letter_spacing_brand_px: f32,
}

impl Typography {
    pub const fn realtylens() -> Self {
        Self {
            family: FONT_FAMILY_INTER,
            family_mono: FONT_FAMILY_MONO,
            body_size_px: 16,
            body_line_height_x100: 160,
            body_weight: 300,
            brand_size_px: 14,
            brand_weight: 500,
            letter_spacing_brand_px: -0.3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typography_matches_spec_values() {
        let t = Typography::realtylens();
        assert_eq!(t.body_size_px, 16);
        assert_eq!(t.body_weight, 300);
        assert_eq!(t.body_line_height_x100, 160);
        assert!(t.family.starts_with("Inter"));
    }
}
