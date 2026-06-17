use crate::gui::AppState;
use crate::gui::helpers::draw_stat_card;
use crate::gui::theme;
use crate::validator::DiagnosticSeverity;
use eframe::egui::{self, RichText, Stroke};

impl AppState {
    pub(super) fn draw_dashboard(&mut self, ui: &mut egui::Ui) {
        ui.heading("Workspace Dashboard");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Competition/Workspace Name:").strong());
            if ui
                .text_edit_singleline(&mut self.config.competition_name)
                .changed()
            {
                self.update_diagnostics();
            }
        });
        ui.add_space(15.0);

        ui.horizontal_wrapped(|ui| {
            draw_stat_card(
                ui,
                "🏆",
                "Divisions",
                &self.config.divisions.len().to_string(),
                theme::accent(),
            );
            draw_stat_card(
                ui,
                "👥",
                "Teams",
                &self.config.teams.len().to_string(),
                theme::accent_alt(),
            );
            draw_stat_card(
                ui,
                "🏟",
                "Fields / Arenas",
                &self.config.fields.len().to_string(),
                theme::success(),
            );
            draw_stat_card(
                ui,
                "📅",
                "Time Slots",
                &self.config.time_slots.len().to_string(),
                theme::rose(),
            );
            draw_stat_card(
                ui,
                "👤",
                "Volunteers",
                &self.config.volunteers.len().to_string(),
                theme::warning(),
            );
        });

        ui.add_space(20.0);

        ui.label(
            RichText::new("REAL-TIME DIAGNOSTICS")
                .strong()
                .size(12.0)
                .color(theme::text_muted()),
        );
        ui.add_space(5.0);

        let error_count = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count();
        let warn_count = self
            .diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count();

        if error_count == 0 && warn_count == 0 {
            egui::Frame::none()
                .fill(theme::success_bg())
                .stroke(Stroke::new(1.0, theme::success_border()))
                .rounding(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    let fg = theme::contrast_text(theme::success_bg());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("✅ READY").strong().color(fg));
                        ui.label(RichText::new("No configuration issues found! Your workspace setup is compatible. You can generate the schedule under the 'Schedule Solver' tab.").color(fg));
                    });
                });
        } else {
            for diag in &self.diagnostics {
                let bg_color = match diag.severity {
                    DiagnosticSeverity::Error => theme::danger_bg(),
                    DiagnosticSeverity::Warning => theme::warning_bg(),
                    DiagnosticSeverity::Info => theme::info_bg(),
                };
                let border_color = match diag.severity {
                    DiagnosticSeverity::Error => theme::danger_border(),
                    DiagnosticSeverity::Warning => theme::warning_border(),
                    DiagnosticSeverity::Info => theme::info_border(),
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
                        let fg = theme::contrast_text(bg_color);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(header_text).strong().color(fg));
                                ui.label(RichText::new(&diag.message).color(fg).strong());
                            });
                            if let Some(ref rec) = diag.recommendation {
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new(format!("💡 Recommended: {}", rec))
                                        .color(fg)
                                        .size(11.5),
                                );
                            }
                        });
                    });
                ui.add_space(8.0);
            }
        }
    }
}
