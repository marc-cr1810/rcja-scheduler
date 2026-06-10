use crate::gui::AppState;
use crate::validator::DiagnosticSeverity;
use crate::gui::helpers::draw_stat_card;
use eframe::egui::{self, Color32, RichText, Stroke};

impl AppState {
    pub(super) fn draw_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.heading("Workspace Dashboard");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Competition/Workspace Name:").strong());
            if ui.text_edit_singleline(&mut self.config.competition_name).changed() {
                self.update_diagnostics();
            }
        });
        ui.add_space(15.0);

        ui.horizontal(|ui| {
            draw_stat_card(ui, "Divisions", &self.config.divisions.len().to_string(), Color32::from_rgb(129, 140, 248));
            draw_stat_card(ui, "Teams", &self.config.teams.len().to_string(), Color32::from_rgb(167, 139, 250));
            draw_stat_card(ui, "Fields / Arenas", &self.config.fields.len().to_string(), Color32::from_rgb(52, 211, 153));
            draw_stat_card(ui, "Time Slots", &self.config.time_slots.len().to_string(), Color32::from_rgb(244, 63, 94));
            draw_stat_card(ui, "Volunteers", &self.config.volunteers.len().to_string(), Color32::from_rgb(251, 191, 36));
        });

        ui.add_space(20.0);

        ui.label(RichText::new("REAL-TIME DIAGNOSTICS").strong().size(12.0).color(Color32::from_rgb(156, 163, 175)));
        ui.add_space(5.0);

        let error_count = self.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Error).count();
        let warn_count = self.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Warning).count();

        if error_count == 0 && warn_count == 0 {
            egui::Frame::none()
                .fill(Color32::from_rgb(6, 78, 59))
                .stroke(Stroke::new(1.0, Color32::from_rgb(16, 185, 129)))
                .rounding(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("✅ READY").strong().color(Color32::WHITE));
                        ui.label(RichText::new("No configuration issues found! Your workspace setup is compatible. You can generate the schedule under the 'Schedule Solver' tab.").color(Color32::from_rgb(167, 243, 208)));
                    });
                });
        } else {
            for diag in &self.diagnostics {
                let bg_color = match diag.severity {
                    DiagnosticSeverity::Error => Color32::from_rgb(127, 29, 29),
                    DiagnosticSeverity::Warning => Color32::from_rgb(120, 53, 4),
                    DiagnosticSeverity::Info => Color32::from_rgb(30, 58, 138),
                };
                let border_color = match diag.severity {
                    DiagnosticSeverity::Error => Color32::from_rgb(239, 68, 68),
                    DiagnosticSeverity::Warning => Color32::from_rgb(245, 158, 11),
                    DiagnosticSeverity::Info => Color32::from_rgb(59, 130, 246),
                };
                let header_text = match diag.severity {
                    DiagnosticSeverity::Error => "❌ ERROR",
                    DiagnosticSeverity::Warning => "⚠ WARNING",
                    DiagnosticSeverity::Info => "ℹ INFO",
                };

                egui::Frame::none()
                    .fill(bg_color)
                    .stroke(Stroke::new(1.0, border_color))
                    .rounding(8.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(header_text).strong().color(Color32::WHITE));
                                ui.label(RichText::new(&diag.message).color(Color32::WHITE).strong());
                            });
                            if let Some(ref rec) = diag.recommendation {
                                ui.add_space(4.0);
                                ui.label(RichText::new(format!("💡 Recommended: {}", rec)).color(Color32::from_rgb(229, 231, 235)).size(11.5));
                            }
                        });
                    });
                ui.add_space(8.0);
            }
        }
    }
}
