use crate::gui::AppState;
use crate::gui::theme;
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

        ui.horizontal_wrapped(|ui| {
            draw_stat_card(ui, "🏆", "Divisions", &self.config.divisions.len().to_string(), theme::ACCENT);
            draw_stat_card(ui, "👥", "Teams", &self.config.teams.len().to_string(), theme::ACCENT_ALT);
            draw_stat_card(ui, "🏟", "Fields / Arenas", &self.config.fields.len().to_string(), theme::SUCCESS);
            draw_stat_card(ui, "📅", "Time Slots", &self.config.time_slots.len().to_string(), theme::ROSE);
            draw_stat_card(ui, "👤", "Volunteers", &self.config.volunteers.len().to_string(), theme::WARNING);
        });

        ui.add_space(20.0);

        ui.label(RichText::new("REAL-TIME DIAGNOSTICS").strong().size(12.0).color(theme::TEXT_MUTED));
        ui.add_space(5.0);

        let error_count = self.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Error).count();
        let warn_count = self.diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Warning).count();

        if error_count == 0 && warn_count == 0 {
            egui::Frame::none()
                .fill(theme::SUCCESS_BG)
                .stroke(Stroke::new(1.0, theme::SUCCESS_BORDER))
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
                    DiagnosticSeverity::Error => theme::DANGER_BG,
                    DiagnosticSeverity::Warning => theme::WARNING_BG,
                    DiagnosticSeverity::Info => theme::INFO_BG,
                };
                let border_color = match diag.severity {
                    DiagnosticSeverity::Error => theme::DANGER_BORDER,
                    DiagnosticSeverity::Warning => theme::WARNING_BORDER,
                    DiagnosticSeverity::Info => theme::INFO_BORDER,
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
                                ui.label(RichText::new(format!("💡 Recommended: {}", rec)).color(theme::TEXT).size(11.5));
                            }
                        });
                    });
                ui.add_space(8.0);
            }
        }
    }
}
