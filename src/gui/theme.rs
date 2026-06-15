//! Central palette for the GUI.
//!
//! Every colour the app draws should come from here rather than from inline
//! `Color32::from_rgb(...)` literals, so the whole look can be retuned (or a
//! light theme added) in one place. The names are semantic — `ACCENT`,
//! `DANGER`, `TEXT_MUTED` — not "indigo400", so call sites read by intent.
//!
//! Values track the Tailwind palette the app was originally built against.

use eframe::egui::Color32;

// ── Surfaces ────────────────────────────────────────────────────────────────
/// App background / non-interactive base (gray-900).
pub const BG_BASE: Color32 = Color32::from_rgb(17, 24, 39);
/// Primary card / panel fill.
pub const CARD_BG: Color32 = Color32::from_rgb(30, 37, 50);
/// Slightly cooler card fill used for list/division rows (gray-800-ish).
pub const CARD_BG_ALT: Color32 = Color32::from_rgb(26, 32, 44);
/// Inset/secondary surface (gray-800).
pub const SURFACE: Color32 = Color32::from_rgb(31, 41, 55);
/// Zebra-striping highlight row — one notch lighter than [`CARD_BG_ALT`].
pub const ROW_STRIPE: Color32 = Color32::from_rgb(33, 40, 54);

// ── Borders ───────────────────────────────────────────────────────────────--
/// Default hairline border (gray-700).
pub const BORDER: Color32 = Color32::from_rgb(55, 65, 81);

// ── Text ──────────────────────────────────────────────────────────────────--
/// Primary text on dark surfaces.
pub const TEXT: Color32 = Color32::from_rgb(229, 231, 235);
/// Slightly dimmed body text (gray-300).
pub const TEXT_DIM: Color32 = Color32::from_rgb(209, 213, 219);
/// Muted captions / secondary labels (gray-400).
pub const TEXT_MUTED: Color32 = Color32::from_rgb(156, 163, 175);
/// Faint hints / ids / placeholders (gray-500).
pub const TEXT_FAINT: Color32 = Color32::from_rgb(107, 114, 128);

// ── Accent (indigo) ─────────────────────────────────────────────────────────
/// Primary accent for headings, links, highlights (indigo-400).
pub const ACCENT: Color32 = Color32::from_rgb(129, 140, 248);
/// Mid accent, used for selection fills (indigo-500).
pub const ACCENT_MID: Color32 = Color32::from_rgb(99, 102, 241);
/// Strong accent for active buttons / primary CTAs (indigo-600).
pub const ACCENT_STRONG: Color32 = Color32::from_rgb(79, 70, 229);
/// Secondary accent (violet-400) for variety in stat cards etc.
pub const ACCENT_ALT: Color32 = Color32::from_rgb(167, 139, 250);

// ── Status colours ────────────────────────────────────────────────────────--
/// Success foreground (emerald-400).
pub const SUCCESS: Color32 = Color32::from_rgb(52, 211, 153);
/// Success border (emerald-500).
pub const SUCCESS_BORDER: Color32 = Color32::from_rgb(16, 185, 129);
/// Success surface fill (emerald-900).
pub const SUCCESS_BG: Color32 = Color32::from_rgb(6, 78, 59);

/// Warning foreground (amber-400).
pub const WARNING: Color32 = Color32::from_rgb(251, 191, 36);
/// Warning border (amber-500).
pub const WARNING_BORDER: Color32 = Color32::from_rgb(245, 158, 11);
/// Warning surface fill.
pub const WARNING_BG: Color32 = Color32::from_rgb(120, 53, 4);

/// Danger foreground (red-400).
pub const DANGER: Color32 = Color32::from_rgb(248, 113, 113);
/// Danger border (red-500).
pub const DANGER_BORDER: Color32 = Color32::from_rgb(239, 68, 68);
/// Danger surface fill (red-900).
pub const DANGER_BG: Color32 = Color32::from_rgb(127, 29, 29);

/// Info foreground (blue-500).
pub const INFO_BORDER: Color32 = Color32::from_rgb(59, 130, 246);
/// Info surface fill.
pub const INFO_BG: Color32 = Color32::from_rgb(30, 58, 138);

/// Rose accent used by the "time slots" stat card.
pub const ROSE: Color32 = Color32::from_rgb(244, 63, 94);

// ── Division category palette ─────────────────────────────────────────────--
/// A curated, evenly-spread set of base hues for divisions that have no
/// explicit colour assigned. Chosen for distinctness and legible contrast on
/// the dark schedule grid (Tailwind-500 family). Indexed by division order so
/// colours are stable frame-to-frame.
pub const DIVISION_PALETTE: [Color32; 12] = [
    Color32::from_rgb(99, 102, 241),  // indigo
    Color32::from_rgb(16, 185, 129),  // emerald
    Color32::from_rgb(245, 158, 11),  // amber
    Color32::from_rgb(244, 63, 94),   // rose
    Color32::from_rgb(14, 165, 233),  // sky
    Color32::from_rgb(139, 92, 246),  // violet
    Color32::from_rgb(249, 115, 22),  // orange
    Color32::from_rgb(20, 184, 166),  // teal
    Color32::from_rgb(236, 72, 153),  // pink
    Color32::from_rgb(132, 204, 22),  // lime
    Color32::from_rgb(6, 182, 212),   // cyan
    Color32::from_rgb(217, 70, 239),  // fuchsia
];

/// Derive a (background, border) pair for a schedule cell from a base RGB.
///
/// Border keeps the base hue at near-full strength; the fill is the same hue
/// darkened so white text stays readable on top.
pub fn cell_colors_from_rgb(rgb: [u8; 3]) -> (Color32, Color32) {
    let hsv = eframe::egui::ecolor::Hsva::from_srgb(rgb);
    let bg = eframe::egui::ecolor::Hsva::new(hsv.h, hsv.s * 1.2, hsv.v * 0.4, 1.0);
    let border = eframe::egui::ecolor::Hsva::new(hsv.h, hsv.s, hsv.v * 0.8, 1.0);
    (Color32::from(bg), Color32::from(border))
}

/// (background, border) for the Nth uncoloured division, cycling the palette.
pub fn division_cell_colors(index: usize) -> (Color32, Color32) {
    let base = DIVISION_PALETTE[index % DIVISION_PALETTE.len()];
    cell_colors_from_rgb([base.r(), base.g(), base.b()])
}
