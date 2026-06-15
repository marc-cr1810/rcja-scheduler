use crate::model::{TournamentConfig, ScheduleAssignment};
use crate::scheduler::{AssignmentConflict, ConflictSeverity};
use super::theme;
use eframe::egui::{self, Color32, FontFamily, FontId, RichText, Stroke, TextStyle};

pub(crate) fn setup_custom_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();

    // ── Type scale ──────────────────────────────────────────────────────────
    // A deliberate hierarchy so headings/body/captions stay consistent instead
    // of every call site inventing its own `.size(..)`. Headings use the bold
    // Lato family registered in `main`.
    use FontFamily::{Monospace, Proportional};
    style.text_styles = [
        (TextStyle::Heading, FontId::new(19.0, FontFamily::Name("Heading".into()))),
        (TextStyle::Body, FontId::new(14.0, Proportional)),
        (TextStyle::Button, FontId::new(14.0, Proportional)),
        (TextStyle::Small, FontId::new(11.0, Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, Monospace)),
    ]
    .into();

    // ── Spacing ─────────────────────────────────────────────────────────────
    // A little more vertical breathing room between rows and chunkier buttons,
    // without bloating the dense grids (availability / heatmap).
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(9.0, 5.0);

    // ── Colours / shape ───────────────────────────────────────────────────--
    // Start from egui's light or dark base depending on the theme's background
    // brightness, then map the core surfaces/text onto the active theme so the
    // whole UI (panels, windows, default text) follows the JSON, not just the
    // bits we paint by hand.
    let bg = theme::bg_base();
    let is_light = relative_luminance(bg) > 0.5;
    let visuals = &mut style.visuals;
    *visuals = if is_light { egui::Visuals::light() } else { egui::Visuals::dark() };

    visuals.panel_fill = bg;
    visuals.window_fill = theme::card_bg();
    visuals.extreme_bg_color = theme::surface();
    visuals.faint_bg_color = theme::row_stripe(); // Grid::striped row tint
    visuals.override_text_color = Some(theme::text()); // default (un-coloured) text
    visuals.hyperlink_color = theme::accent();

    visuals.widgets.noninteractive.bg_fill = theme::bg_base();
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme::border());
    visuals.widgets.inactive.bg_fill = theme::surface();
    visuals.widgets.hovered.bg_fill = theme::border();
    visuals.widgets.active.bg_fill = theme::accent_mid();
    visuals.widgets.open.bg_fill = theme::surface();
    visuals.selection.bg_fill = theme::accent_mid();
    visuals.selection.stroke = Stroke::new(1.0, theme::accent());

    visuals.window_rounding = egui::Rounding::same(12.0);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(6.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(6.0);
    visuals.widgets.active.rounding = egui::Rounding::same(6.0);

    ctx.set_style(style);
}

/// Perceptual-ish luminance in 0..1, used to decide light vs dark base visuals.
fn relative_luminance(c: Color32) -> f32 {
    let [r, g, b, _] = c.to_array();
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

pub(crate) fn draw_stat_card(ui: &mut egui::Ui, icon: &str, title: &str, value: &str, color: Color32) {
    egui::Frame::none()
        .fill(theme::card_bg())
        .rounding(8.0)
        .inner_margin(egui::Margin { left: 14.0, right: 16.0, top: 10.0, bottom: 14.0 })
        .show(ui, |ui| {
            ui.set_min_size(egui::vec2(140.0, 78.0));
            ui.vertical(|ui| {
                // Thin accent bar in the value's colour across the top of the card.
                let (bar_rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.0), egui::Sense::hover());
                ui.painter().rect_filled(bar_rect, egui::Rounding::same(2.0), color);
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon).size(13.0).color(color));
                    ui.label(RichText::new(title).size(10.5).color(theme::text_muted()).strong());
                });
                ui.add_space(2.0);
                ui.label(RichText::new(value).size(22.0).strong().color(color));
            });
        });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_schedule_cell(ui: &mut egui::Ui, assign: &ScheduleAssignment, config: &TournamentConfig, current_slot_id: &str, w: f32, h: f32, conflicts: &[AssignmentConflict], assign_idx: usize) -> Option<usize> {
    let mut substitution_requested = None;
    let div_id = assign.activity.division_id();
    let (base_bg, base_border) = get_competition_colors(div_id, config);
    let is_continuation = current_slot_id != assign.time_slot_id;

    // Continuation cells (the tail of a multi-slot block) are drawn fainter so
    // the eye groups them with the block they belong to.
    let (bg_color, border_color) = if is_continuation {
        (
            Color32::from_rgba_unmultiplied(base_bg.r(), base_bg.g(), base_bg.b(), 90),
            Color32::from_rgba_unmultiplied(base_border.r(), base_border.g(), base_border.b(), 120),
        )
    } else {
        (base_bg, base_border)
    };

    // Conflicts that should grab attention get a pulsing red outline.
    let is_error_cell = conflicts.iter().any(|c| {
        matches!(c.severity, ConflictSeverity::Error) || c.message.contains("NO-SHOW")
    });

    let start_time_str = config.time_slots.iter()
        .find(|s| s.id == assign.time_slot_id)
        .map(|s| s.start_time.clone())
        .unwrap_or_else(|| "09:00".to_string());
    let start_m = parse_time_to_minutes(&start_time_str);
    let end_m = start_m + assign.activity.duration_minutes();
    let end_time_str = format_minutes_to_time(end_m);

    let cell_w = w.max(10.0);
    let cell_h = h.max(10.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(cell_w, cell_h), egui::Sense::click());

    if response.clicked() {
        // Option to open substitution if there's a conflict
        if !conflicts.is_empty() {
            substitution_requested = Some(assign_idx);
        }
    }

    ui.painter().rect_filled(rect, 6.0, bg_color);

    // Left accent stripe in the full-strength division colour for fast scanning.
    let stripe = egui::Rect::from_min_max(
        rect.left_top(),
        egui::pos2(rect.left() + 4.0, rect.bottom()),
    );
    let stripe_color = if is_continuation {
        Color32::from_rgba_unmultiplied(base_border.r(), base_border.g(), base_border.b(), 140)
    } else {
        base_border
    };
    ui.painter().rect_filled(
        stripe,
        egui::Rounding { nw: 6.0, sw: 6.0, ne: 0.0, se: 0.0 },
        stripe_color,
    );

    // Border: a dashed outline reads clearly as "continued", a solid one as a
    // self-contained block.
    if is_continuation {
        let pts = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
            rect.left_top(),
        ];
        ui.painter().extend(egui::Shape::dashed_line(
            &pts,
            Stroke::new(1.0, border_color),
            4.0,
            3.0,
        ));
    } else {
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, border_color));
    }

    // Pulsing outline for error/no-show cells so they catch the eye.
    if is_error_cell {
        let t = ui.input(|i| i.time);
        let pulse = 0.5 + 0.5 * (t * 4.0).sin() as f32;
        let alpha = (70.0 + pulse * 150.0) as u8;
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(2.0, Color32::from_rgba_unmultiplied(248, 113, 113, alpha)),
        );
        ui.ctx().request_repaint();
    }

    let inner_rect = rect.shrink2(egui::vec2(8.0, 4.0));
    let mut child_ui = ui.child_ui(inner_rect, *ui.layout());
    
    let mut clip_rect = child_ui.clip_rect();
    clip_rect.max.x = clip_rect.max.x.min(inner_rect.max.x);
    clip_rect.max.y = clip_rect.max.y.min(inner_rect.max.y);
    child_ui.set_clip_rect(clip_rect);

    child_ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let label_text = if is_continuation {
                RichText::new(format!("{} (cont.)", assign.activity.label())).size(11.5).color(theme::text_muted())
            } else {
                RichText::new(assign.activity.label()).strong().size(11.5).color(Color32::WHITE)
            };
            
            // Use a vertical layout for the label to allow it to wrap within the available horizontal space
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                ui.label(label_text);
            });

            if !conflicts.is_empty() {
                let has_error = conflicts.iter().any(|c| matches!(c.severity, ConflictSeverity::Error));
                let is_no_show = conflicts.iter().any(|c| c.message.contains("NO-SHOW"));
                
                let icon = if is_no_show { "🏃" } else if has_error { "❌" } else { "⚠" };
                let color = if is_no_show || has_error { theme::danger() } else { theme::warning() };
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let resp = ui.add(egui::Button::new(RichText::new(icon).color(color).strong()).frame(false));
                    if resp.clicked() {
                        substitution_requested = Some(assign_idx);
                    }
                    resp.on_hover_ui(|ui| {
                        ui.vertical(|ui| {
                            for c in conflicts {
                                let c_icon = if c.message.contains("NO-SHOW") { "🏃" } else if matches!(c.severity, ConflictSeverity::Error) { "❌" } else { "⚠" };
                                ui.label(format!("{} {}", c_icon, c.message));
                            }
                            ui.add_space(4.0);
                            ui.label(RichText::new("Click to find substitute").italics().color(theme::text_muted()));
                        });
                    });
                });
            }
        });

        if h >= 30.0 {
            ui.label(RichText::new(format!("⏰ {} - {}", start_time_str, end_time_str)).size(9.5).color(theme::text_dim()));
        }
        
        let volunteer_names: Vec<String> = assign
            .volunteer_ids
            .iter()
            .map(|v_id| {
                config
                    .volunteers
                    .iter()
                    .find(|v| v.id == *v_id)
                    .map_or(v_id.clone(), |v| v.name.split(' ').next().unwrap_or(&v.name).to_string())
            })
            .collect();

        if h >= 40.0 {
            let vol_label = if matches!(assign.activity, crate::model::Activity::Interview { .. }) {
                if assign.volunteer_ids.len() > 1 { "Judges" } else { "Interviewer" }
            } else {
                "Refs"
            };

            let names_str = if volunteer_names.is_empty() { "None".to_string() } else { volunteer_names.join(", ") };
            let vol_text = if is_continuation {
                RichText::new(format!("{}: {}", vol_label, names_str)).size(9.5).color(Color32::from_rgb(120, 130, 140))
            } else {
                let color = if volunteer_names.is_empty() { theme::danger() } else { theme::text_dim() };
                RichText::new(format!("{}: {}", vol_label, names_str)).size(9.5).color(color)
            };
            
            ui.add(egui::Label::new(vol_text).wrap(true));
        }
    });

    response.on_hover_ui(|ui| {
        ui.vertical(|ui| {
                ui.heading(format!("{}{}", assign.activity.label(), if is_continuation { " (Continuation)" } else { "" }));
                let div_name = config.divisions.iter().find(|d| d.id == div_id).map_or(div_id, |d| &d.name);
                ui.label(format!("Division: {}", div_name));
                
                if let Some(stage_label) = assign.activity.stage_label() {
                    ui.label(RichText::new(format!("Stage: {}", stage_label)).strong().color(theme::warning()));
                }
                
                let round_label = assign.activity.round_label();
                if !round_label.is_empty() {
                    ui.label(RichText::new(round_label).strong().color(theme::accent()));
                }

                ui.label(format!("Time: {} - {} ({} min)", start_time_str, end_time_str, assign.activity.duration_minutes()));
                
                let volunteer_full_names: Vec<String> = assign
                    .volunteer_ids
                    .iter()
                    .map(|v_id| {
                        config
                            .volunteers
                            .iter()
                            .find(|v| v.id == *v_id)
                            .map_or(v_id.clone(), |v| v.name.clone())
                    })
                    .collect();

                let vol_label = if matches!(assign.activity, crate::model::Activity::Interview { .. }) {
                    if assign.volunteer_ids.len() > 1 { "Assigned Judges" } else { "Assigned Interviewer" }
                } else {
                    "Assigned Referees"
                };
                ui.label(format!("{}: {}", vol_label, volunteer_full_names.join(", ")));

                if !conflicts.is_empty() {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.label(RichText::new("Conflicts:").strong().color(theme::danger()));
                    for c in conflicts {
                        let icon = if matches!(c.severity, ConflictSeverity::Error) { "❌" } else { "⚠" };
                        ui.label(format!("{} {}", icon, c.message));
                    }
                }
            });
        });

    substitution_requested
}

pub(crate) fn get_competition_colors(div_id: &str, config: &TournamentConfig) -> (Color32, Color32) {
    // An explicit per-division colour always wins.
    if let Some(div) = config.divisions.iter().find(|d| d.id == div_id)
        && let Some(rgb) = div.color {
            return super::theme::cell_colors_from_rgb(rgb);
        }

    // Otherwise fall back to a curated categorical palette, indexed by the
    // division's position so each division gets a distinct, legible colour
    // that stays stable frame-to-frame (instead of a hash → arbitrary hue).
    if let Some(idx) = config.divisions.iter().position(|d| d.id == div_id) {
        return super::theme::division_cell_colors(idx);
    }

    // Unknown division id (e.g. stale schedule): hash to a palette slot so the
    // colour is at least deterministic.
    let mut hash: u32 = 0;
    for c in div_id.chars() {
        hash = hash.wrapping_add(c as u32).wrapping_mul(31);
    }
    super::theme::division_cell_colors(hash as usize)
}

/// Returns true if `t` is a well-formed `HH:MM` time string.
pub(crate) fn is_valid_hhmm(t: &str) -> bool {
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() != 2 {
        return false;
    }
    match (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
        (Ok(h), Ok(m)) => h < 24 && m < 60,
        _ => false,
    }
}

/// A 50px `HH:MM` text field whose text turns red while the contents are malformed,
/// giving inline feedback before the user clicks Generate.
pub(crate) fn time_edit(value: &mut String) -> egui::TextEdit<'_> {
    let valid = is_valid_hhmm(value);
    let mut edit = egui::TextEdit::singleline(value).desired_width(50.0);
    if !valid {
        edit = edit.text_color(theme::danger());
    }
    edit
}

pub(crate) fn parse_time_to_minutes(t: &str) -> u32 {
    let parts: Vec<&str> = t.split(':').collect();
    if parts.len() == 2 {
        let h = parts[0].parse::<u32>().unwrap_or(0);
        let m = parts[1].parse::<u32>().unwrap_or(0);
        h * 60 + m
    } else {
        0
    }
}

pub(crate) fn draw_card<R>(ui: &mut egui::Ui, title: &str, add_space: bool, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(theme::card_bg())
        .rounding(8.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                if !title.is_empty() {
                    ui.label(RichText::new(title).strong().color(Color32::WHITE));
                    if add_space {
                        ui.add_space(8.0);
                    }
                }
                add_contents(ui)
            }).inner
        }).inner
}

pub(crate) fn format_minutes_to_time(min: u32) -> String {
    let hr = min / 60;
    let mn = min % 60;
    format!("{:02}:{:02}", hr, mn)
}

/// A friendly centred placeholder for tabs/sections that have no data yet.
///
/// Returns `true` on the frame the optional call-to-action button is clicked.
pub(crate) fn draw_empty_state(
    ui: &mut egui::Ui,
    icon: &str,
    title: &str,
    body: &str,
    cta: Option<&str>,
) -> bool {
    let mut clicked = false;
    ui.vertical_centered(|ui| {
        ui.add_space(40.0);
        ui.label(RichText::new(icon).size(44.0).color(theme::text_faint()));
        ui.add_space(10.0);
        ui.label(RichText::new(title).size(16.0).strong().color(theme::text_muted()));
        ui.add_space(2.0);
        ui.label(RichText::new(body).color(theme::text_faint()));
        if let Some(label) = cta {
            ui.add_space(14.0);
            let btn = egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
                .fill(theme::accent_strong())
                .rounding(egui::Rounding::same(6.0))
                .min_size(egui::vec2(0.0, 30.0));
            if ui.add(btn).clicked() {
                clicked = true;
            }
        }
        ui.add_space(40.0);
    });
    clicked
}
