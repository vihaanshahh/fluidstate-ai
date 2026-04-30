//! Linear Taco — RealtyLens theme tokens.
//!
//! This crate is intentionally view-framework-agnostic. It exposes the
//! RealtyLens design tokens (color palette, typography, motion) as plain
//! Rust constants and small data types. The Warp-side wire-in lives in
//! `app/src/themes/` and turns these tokens into the engine's runtime
//! palette structures.
//!
//! Source of truth: `docs/realtylens-theme.txt` in this fork. Token names
//! mirror the CSS variable names from that document, just translated to
//! Rust idioms.
//!
//! # Surfaces
//!
//! - [`palette::Palette`] — every named color we'll need.
//! - [`palette::ChipPalette`] — the seven editorial chip accents and the
//!   IDE semantics they're assigned to.
//! - [`terminal_palette::AnsiPalette`] — the 16-slot ANSI palette derived
//!   from RealtyLens tokens for the terminal pane.
//! - [`motion`] — easing + duration constants for the reduce-motion-aware
//!   animation system.
//! - [`typography`] — Inter font family, weights, and sizing.
//!
//! All values are `pub const` so consumers can `match` on them, plug them
//! into static config, or expose them through Settings.

#![deny(rust_2018_idioms)]

pub mod motion;
pub mod palette;
pub mod terminal_palette;
pub mod typography;

pub use motion::{Easing, MOTION_DURATION_LONG, MOTION_DURATION_SHORT, MOTION_EASE_RL};
pub use palette::{ChipPalette, Color, Palette, ThemeMode};
pub use terminal_palette::AnsiPalette;
pub use typography::{FONT_FAMILY_INTER, Typography};
