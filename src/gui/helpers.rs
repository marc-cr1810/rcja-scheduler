use crate::model::{TournamentConfig, ScheduleAssignment};
use crate::scheduler::{AssignmentConflict, ConflictSeverity};
use eframe::egui::{self, Color32, RichText, Stroke};

pub(crate) fn setup_custom_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(17, 24, 39);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(31, 41, 55);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(55, 65, 81);
    visuals.widgets.active.bg_fill = Color32::from_rgb(75, 85, 99);
    visuals.widgets.open.bg_fill = Color32::from_rgb(75, 85, 99);
    visuals.selection.bg_fill = Color32::from_rgb(99, 102, 241);
    visuals.window_rounding = egui::Rounding::same(12.0);
    visuals.widgets.noninteractive.rounding = egui::Rounding::same(8.0);
    visuals.widgets.inactive.rounding = egui::Rounding::same(6.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(6.0);
    visuals.widgets.active.rounding = egui::Rounding::same(6.0);
    ctx.set_visuals(visuals);
}

pub(crate) fn draw_stat_card(ui: &mut egui::Ui, title: &str, value: &str, color: Color32) {
    egui::Frame::none()
        .fill(Color32::from_rgb(30, 37, 50))
        .rounding(8.0)
        .inner_margin(16.0)
        .show(ui, |ui| {
            ui.set_min_size(egui::vec2(130.0, 75.0));
            ui.vertical(|ui| {
                ui.label(RichText::new(title).size(10.5).color(Color32::from_rgb(156, 163, 175)).strong());
                ui.add_space(4.0);
                ui.label(RichText::new(value).size(22.0).strong().color(color));
            });
        });
}

pub(crate) fn draw_schedule_cell(ui: &mut egui::Ui, assign: &ScheduleAssignment, config: &TournamentConfig, current_slot_id: &str, w: f32, h: f32, conflicts: &[AssignmentConflict]) -> bool {
    let mut conflict_clicked = false;
    let div_id = assign.activity.division_id();
    let (mut bg_color, mut border_color) = get_competition_colors(div_id, config);
    let is_continuation = current_slot_id != assign.time_slot_id;

    if is_continuation {
        bg_color = Color32::from_rgba_unmultiplied(
            bg_color.r(),
            bg_color.g(),
            bg_color.b(),
            90,
        );
        border_color = Color32::from_rgba_unmultiplied(
            border_color.r(),
            border_color.g(),
            border_color.b(),
            120,
        );
    }

    let start_time_str = config.time_slots.iter()
        .find(|s| s.id == assign.time_slot_id)
        .map(|s| s.start_time.clone())
        .unwrap_or_else(|| "09:00".to_string());
    let start_m = parse_time_to_minutes(&start_time_str);
    let end_m = start_m + assign.activity.duration_minutes();
    let end_time_str = format_minutes_to_time(end_m);

    let cell_w = w.max(10.0);
    let cell_h = h.max(10.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(cell_w, cell_h), egui::Sense::hover());

    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(if is_continuation { 0.7 } else { 1.0 }, border_color),
    );
    ui.painter().rect_filled(rect, 6.0, bg_color);

    let inner_rect = rect.shrink2(egui::vec2(8.0, 4.0));
    let mut child_ui = ui.child_ui(inner_rect, *ui.layout());
    
    let mut clip_rect = child_ui.clip_rect();
    clip_rect.max.x = clip_rect.max.x.min(inner_rect.max.x);
    clip_rect.max.y = clip_rect.max.y.min(inner_rect.max.y);
    child_ui.set_clip_rect(clip_rect);

    child_ui.vertical(|ui| {
        ui.horizontal(|ui| {
            let label_text = if is_continuation {
                RichText::new(format!("{} (cont.)", assign.activity.label())).size(11.5).color(Color32::from_rgb(156, 163, 175))
            } else {
                RichText::new(assign.activity.label()).strong().size(11.5).color(Color32::WHITE)
            };
            
            // Use a vertical layout for the label to allow it to wrap within the available horizontal space
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
                ui.label(label_text);
            });

            if !conflicts.is_empty() {
                let has_error = conflicts.iter().any(|c| matches!(c.severity, ConflictSeverity::Error));
                let icon = if has_error { "❌" } else { "⚠" };
                let color = if has_error { Color32::from_rgb(248, 113, 113) } else { Color32::from_rgb(251, 191, 36) };
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let resp = ui.add(egui::Button::new(RichText::new(icon).color(color).strong()).frame(false));
                    if resp.clicked() {
                        conflict_clicked = true;
                    }
                    resp.on_hover_ui(|ui| {
                        ui.vertical(|ui| {
                            for c in conflicts {
                                let c_icon = if matches!(c.severity, ConflictSeverity::Error) { "❌" } else { "⚠" };
                                ui.label(format!("{} {}", c_icon, c.message));
                            }
                        });
                    });
                });
            }
        });

        if h >= 30.0 {
            ui.label(RichText::new(format!("⏰ {} - {}", start_time_str, end_time_str)).size(9.5).color(Color32::from_rgb(209, 213, 219)));
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
                let color = if volunteer_names.is_empty() { Color32::from_rgb(248, 113, 113) } else { Color32::from_rgb(209, 213, 219) };
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
                    ui.label(RichText::new(format!("Stage: {}", stage_label)).strong().color(Color32::from_rgb(251, 191, 36)));
                }
                
                let round_label = assign.activity.round_label();
                if !round_label.is_empty() {
                    ui.label(RichText::new(round_label).strong().color(Color32::from_rgb(129, 140, 248)));
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
                    ui.label(RichText::new("Conflicts:").strong().color(Color32::from_rgb(248, 113, 113)));
                    for c in conflicts {
                        let icon = if matches!(c.severity, ConflictSeverity::Error) { "❌" } else { "⚠" };
                        ui.label(format!("{} {}", icon, c.message));
                    }
                }
            });
        });

    conflict_clicked
}

pub(crate) fn get_competition_colors(div_id: &str, config: &TournamentConfig) -> (Color32, Color32) {
    if let Some(div) = config.divisions.iter().find(|d| d.id == div_id)
        && let Some(rgb) = div.color {
            // Generate a slightly darker background version for the fill
            // and use the main color for the border.
            // Using a fixed opacity/darkness logic similar to the HSL version.
            let hsv = egui::ecolor::Hsva::from_srgb(rgb);
            let bg_hsv = egui::ecolor::Hsva::new(hsv.h, hsv.s * 1.2, hsv.v * 0.4, 1.0);
            let border_hsv = egui::ecolor::Hsva::new(hsv.h, hsv.s, hsv.v * 0.8, 1.0);
            
            return (Color32::from(bg_hsv), Color32::from(border_hsv));
        }

    let mut hash: u32 = 0;
    for c in div_id.chars() {
        hash = hash.wrapping_add(c as u32).wrapping_mul(31);
    }

    let hue = (hash % 360) as f32;
    let rgb_bg = hsl_to_rgb(hue, 0.45, 0.16);
    let rgb_border = hsl_to_rgb(hue, 0.65, 0.45);
    (rgb_bg, rgb_border)
}

pub(crate) fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;

    let (r_temp, g_temp, b_temp) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    Color32::from_rgb(
        ((r_temp + m) * 255.0) as u8,
        ((g_temp + m) * 255.0) as u8,
        ((b_temp + m) * 255.0) as u8,
    )
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
        .fill(Color32::from_rgb(30, 37, 50))
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
