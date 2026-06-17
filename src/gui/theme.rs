//! Runtime-swappable colour palette for the GUI.
//!
//! Colours are read through accessor functions (`theme::accent()`, …) that pull
//! from a single global [`Theme`]. The active theme can be replaced at runtime
//! ([`set_active`]) which is what powers the in-app theme switcher.
//!
//! A theme can be loaded from a JSON file (see [`load_file`] / [`discover`]).
//! Colours in JSON may be written as a hex string (`"#818cf8"`, `"#818cf8ff"`)
//! or an array (`[129,140,248]` / `[129,140,248,255]`). Because the struct uses
//! `#[serde(default)]`, a theme file only needs to list the colours it wants to
//! override — everything else inherits the built-in dark palette.

use eframe::egui::Color32;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, RwLock};

/// A colour as parsed from JSON (hex string or `[r,g,b(,a)]` array).
#[derive(Clone, Copy, Debug)]
pub struct ThemeColor(pub Color32);

impl<'de> Deserialize<'de> for ThemeColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Hex(String),
            Rgb([u8; 3]),
            Rgba([u8; 4]),
        }

        let color = match Repr::deserialize(deserializer)? {
            Repr::Hex(s) => parse_hex(&s).map_err(serde::de::Error::custom)?,
            Repr::Rgb([r, g, b]) => Color32::from_rgb(r, g, b),
            Repr::Rgba([r, g, b, a]) => Color32::from_rgba_unmultiplied(r, g, b, a),
        };
        Ok(ThemeColor(color))
    }
}

fn parse_hex(s: &str) -> Result<Color32, String> {
    let h = s.trim().trim_start_matches('#');
    let byte = |i: usize| -> Result<u8, String> {
        u8::from_str_radix(&h[i..i + 2], 16).map_err(|_| format!("invalid hex colour '{s}'"))
    };
    match h.len() {
        6 => Ok(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Ok(Color32::from_rgba_unmultiplied(
            byte(0)?,
            byte(2)?,
            byte(4)?,
            byte(6)?,
        )),
        _ => Err(format!("hex colour must be 6 or 8 digits, got '{s}'")),
    }
}

/// A full colour palette. Every field has a sensible dark-theme default, so a
/// JSON file can override just the colours it cares about.
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Theme {
    /// Display name shown in the switcher (falls back to the file name).
    pub name: Option<String>,

    // Surfaces
    pub bg_base: ThemeColor,
    pub card_bg: ThemeColor,
    pub card_bg_alt: ThemeColor,
    pub surface: ThemeColor,
    pub row_stripe: ThemeColor,
    // Borders
    pub border: ThemeColor,
    // Text
    pub text: ThemeColor,
    pub text_dim: ThemeColor,
    pub text_muted: ThemeColor,
    pub text_faint: ThemeColor,
    // Accent
    pub accent: ThemeColor,
    pub accent_mid: ThemeColor,
    pub accent_strong: ThemeColor,
    pub accent_alt: ThemeColor,
    /// Text/icon colour drawn on top of accent or other strongly-coloured fills
    /// (buttons, pills, active tabs). Defaults to white.
    pub on_accent: ThemeColor,
    // Status
    pub success: ThemeColor,
    pub success_border: ThemeColor,
    pub success_bg: ThemeColor,
    pub warning: ThemeColor,
    pub warning_border: ThemeColor,
    pub warning_bg: ThemeColor,
    pub danger: ThemeColor,
    pub danger_border: ThemeColor,
    pub danger_bg: ThemeColor,
    pub info: ThemeColor,
    pub info_border: ThemeColor,
    pub info_bg: ThemeColor,
    pub rose: ThemeColor,
    /// Categorical colours for divisions with no explicit colour set.
    pub division_palette: Vec<ThemeColor>,
}

impl Default for Theme {
    fn default() -> Self {
        let c = |r, g, b| ThemeColor(Color32::from_rgb(r, g, b));
        Theme {
            name: Some("Default (Dark)".to_string()),
            // Neutral (near-greyscale) dark surfaces — a faint warm tint keeps it
            // from reading as the cold blue-slate it used to be. Input surfaces
            // are pushed clearly lighter than the cards so fields/drop-downs read
            // as distinct, interactive elements.
            bg_base: c(22, 22, 26),
            card_bg: c(33, 33, 38),
            card_bg_alt: c(28, 28, 32),
            surface: c(45, 45, 52),
            row_stripe: c(31, 31, 36),
            border: c(64, 64, 72),
            text: c(231, 231, 234),
            text_dim: c(209, 209, 214),
            text_muted: c(161, 161, 170),
            text_faint: c(113, 113, 122),
            accent: c(129, 140, 248),
            accent_mid: c(99, 102, 241),
            accent_strong: c(79, 70, 229),
            accent_alt: c(167, 139, 250),
            on_accent: c(255, 255, 255),
            success: c(52, 211, 153),
            success_border: c(16, 185, 129),
            success_bg: c(6, 78, 59),
            warning: c(251, 191, 36),
            warning_border: c(245, 158, 11),
            warning_bg: c(120, 53, 4),
            danger: c(248, 113, 113),
            danger_border: c(239, 68, 68),
            danger_bg: c(127, 29, 29),
            info: c(96, 165, 250),
            info_border: c(59, 130, 246),
            info_bg: c(30, 58, 138),
            rose: c(244, 63, 94),
            division_palette: vec![
                c(99, 102, 241), // indigo
                c(16, 185, 129), // emerald
                c(245, 158, 11), // amber
                c(244, 63, 94),  // rose
                c(14, 165, 233), // sky
                c(139, 92, 246), // violet
                c(249, 115, 22), // orange
                c(20, 184, 166), // teal
                c(236, 72, 153), // pink
                c(132, 204, 22), // lime
                c(6, 182, 212),  // cyan
                c(217, 70, 239), // fuchsia
            ],
        }
    }
}

impl Theme {
    /// Display name for the switcher.
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| "Unnamed".to_string())
    }
}

// ── Active theme + registry ───────────────────────────────────────────────--
static ACTIVE: LazyLock<RwLock<Theme>> = LazyLock::new(|| RwLock::new(Theme::default()));
/// All selectable themes (built-in default + discovered files), populated by
/// [`init`].
static REGISTRY: LazyLock<RwLock<Vec<Theme>>> =
    LazyLock::new(|| RwLock::new(vec![Theme::default()]));

/// Replace the active theme. Re-run `setup_custom_style` afterwards so the
/// changes that feed egui's `Visuals` (backgrounds, selection) also update.
pub fn set_active(theme: Theme) {
    *ACTIVE.write().expect("theme lock poisoned") = theme;
}

/// Populate the registry from disk and activate the first theme. Returns the
/// display name of the activated theme. Call once at startup.
pub fn init() -> String {
    let themes = discover();
    let active_name = themes[0].display_name();
    set_active(themes[0].clone());
    *REGISTRY.write().expect("theme lock poisoned") = themes;
    active_name
}

/// Re-scan theme files from disk, keeping the current selection active if it
/// still exists (otherwise activate the default). Returns the active name.
pub fn reload(current: &str) -> String {
    let themes = discover();
    let pick = themes
        .iter()
        .find(|t| t.display_name() == current)
        .unwrap_or(&themes[0]);
    let name = pick.display_name();
    set_active(pick.clone());
    *REGISTRY.write().expect("theme lock poisoned") = themes;
    name
}

/// Display names of all registered themes, in order.
pub fn names() -> Vec<String> {
    REGISTRY
        .read()
        .expect("theme lock poisoned")
        .iter()
        .map(Theme::display_name)
        .collect()
}

/// Activate the registered theme with this display name, if present.
pub fn activate_by_name(name: &str) -> bool {
    let registry = REGISTRY.read().expect("theme lock poisoned");
    if let Some(theme) = registry.iter().find(|t| t.display_name() == name) {
        let theme = theme.clone();
        drop(registry);
        set_active(theme);
        true
    } else {
        false
    }
}

/// Generate accessor functions that read a single colour from the active theme.
macro_rules! accessors {
    ($($field:ident),* $(,)?) => {
        $(pub fn $field() -> Color32 { ACTIVE.read().expect("theme lock poisoned").$field.0 })*
    };
}

accessors!(
    bg_base,
    card_bg,
    card_bg_alt,
    surface,
    row_stripe,
    border,
    text,
    text_dim,
    text_muted,
    text_faint,
    accent,
    accent_mid,
    accent_strong,
    accent_alt,
    on_accent,
    success,
    success_border,
    success_bg,
    warning,
    warning_border,
    warning_bg,
    danger,
    danger_border,
    danger_bg,
    info,
    info_border,
    info_bg,
    rose,
);

// ── Division colours ──────────────────────────────────────────────────────--
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

/// Black or white text, whichever reads better on the given background.
/// Use for text drawn on a themed fill whose brightness varies per theme
/// (e.g. status banners).
pub fn contrast_text(bg: Color32) -> Color32 {
    let [r, g, b, _] = bg.to_array();
    let lum = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
    if lum > 0.55 {
        Color32::from_rgb(24, 24, 27)
    } else {
        Color32::WHITE
    }
}

/// (background, border) for the Nth uncoloured division, cycling the active
/// theme's division palette.
pub fn division_cell_colors(index: usize) -> (Color32, Color32) {
    let guard = ACTIVE.read().expect("theme lock poisoned");
    if guard.division_palette.is_empty() {
        return cell_colors_from_rgb([99, 102, 241]);
    }
    let base = guard.division_palette[index % guard.division_palette.len()].0;
    cell_colors_from_rgb([base.r(), base.g(), base.b()])
}

// ── Loading / discovery ───────────────────────────────────────────────────--
/// Parse a theme from a JSON file, naming it after the file if it has no `name`.
pub fn load_file(path: &Path) -> Result<Theme, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut theme: Theme =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if theme.name.is_none() {
        theme.name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
    }
    Ok(theme)
}

/// Directories scanned for user theme files (`themes/*.json`): the current
/// working directory and the folder next to the executable.
fn theme_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("themes")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.join("themes"));
    }
    dirs
}

/// All selectable themes: the built-in default first, then any valid
/// `themes/*.json` files found on disk (de-duplicated by display name).
pub fn discover() -> Vec<Theme> {
    let mut themes = vec![Theme::default()];
    let mut seen = std::collections::HashSet::new();
    seen.insert(themes[0].display_name());

    for dir in theme_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        for path in paths {
            match load_file(&path) {
                Ok(theme) if seen.insert(theme.display_name()) => themes.push(theme),
                Ok(_) => {} // duplicate name; skip
                Err(e) => eprintln!("Skipping theme {e}"),
            }
        }
    }
    themes
}
