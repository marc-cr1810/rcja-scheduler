use crate::gui::AppState;
use crate::model::Team;
use eframe::egui::{self, Color32, RichText};

impl AppState {
    pub(super) fn draw_teams(&mut self, ui: &mut egui::Ui) {
        ui.heading("Teams Management");
        ui.add_space(10.0);

        // CSV Import Frame (Always accessible)
        egui::Frame::none()
            .fill(Color32::from_rgb(30, 37, 50))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Bulk Import Teams from CSV").strong().color(Color32::WHITE));
                    ui.label(RichText::new("Import teams from a RoboCup Junior registration CSV file. This will automatically create divisions if they don't exist.").size(11.0).color(Color32::from_rgb(156, 163, 175)));
                    ui.add_space(5.0);
                    
                    if self.csv_import.is_none() {
                        if ui.button("📂 Select CSV File...").clicked()
                            && let Some(path) = rfd::FileDialog::new()
                                .add_filter("CSV Files", &["csv"])
                                .pick_file()
                                && let Ok(content) = std::fs::read_to_string(path) {
                                    self.prepare_csv_import(&content);
                                }
                    } else if self.csv_import.is_some() {
                        let mut finalize = false;
                        let mut cancel = false;
                        
                        ui.group(|ui| {
                            ui.label(RichText::new("Select Divisions to Import:").strong());
                            
                            if let Some(import) = &mut self.csv_import {
                                egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                                    for div_name in &import.divisions {
                                        let mut is_selected = import.selected_divisions.contains(div_name);
                                        if ui.checkbox(&mut is_selected, div_name).changed() {
                                            if is_selected {
                                                import.selected_divisions.insert(div_name.clone());
                                            } else {
                                                import.selected_divisions.remove(div_name);
                                            }
                                        }
                                    }
                                });
                            }
                            
                            ui.add_space(5.0);
                            ui.horizontal(|ui| {
                                if ui.button("✅ Confirm Import").clicked() {
                                    finalize = true;
                                }
                                if ui.button("❌ Cancel").clicked() {
                                    cancel = true;
                                }
                            });
                        });

                        if finalize {
                            self.finalize_csv_import();
                        }
                        if cancel {
                            self.csv_import = None;
                        }
                    }
                });
            });

        ui.add_space(15.0);

        if self.config.divisions.is_empty() {
            ui.label(RichText::new("⚠ To add individual teams manually, please define divisions first.").strong().color(Color32::from_rgb(245, 158, 11)));
            return;
        }

        // Add Individual Team Frame
        egui::Frame::none()
            .fill(Color32::from_rgb(30, 37, 50))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Add Individual Team").strong().color(Color32::WHITE));
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Team Name:");
                            ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut self.new_team_name));
                        });

                        ui.vertical(|ui| {
                            ui.label("Organization:");
                            ui.add_sized([200.0, 20.0], egui::TextEdit::singleline(&mut self.new_team_org));
                        });
                        
                        ui.vertical(|ui| {
                            ui.label("Division:");
                            egui::ComboBox::from_id_source("team_div_combo")
                                .selected_text(
                                    self.config.divisions.iter()
                                        .find(|d| d.id == self.new_team_div_id)
                                        .map(|d| d.name.as_str())
                                        .unwrap_or("Select Division")
                                )
                                .show_ui(ui, |ui| {
                                    for div in &self.config.divisions {
                                        ui.selectable_value(&mut self.new_team_div_id, div.id.clone(), &div.name);
                                    }
                                });
                        });

                        ui.vertical(|ui| {
                            ui.label(""); // Spacer
                            if ui.button("+ Add Team").clicked()
                                && !self.new_team_name.trim().is_empty() && !self.new_team_div_id.is_empty() {
                                    self.config.teams.push(Team {
                                        name: self.new_team_name.trim().to_string(),
                                        division_id: self.new_team_div_id.clone(),
                                        organization: self.new_team_org.trim().to_string(),
                                    });
                                    self.new_team_name.clear();
                                    self.new_team_org.clear();
                                    self.clear_schedule();
                                    self.update_diagnostics();
                                    self.status_message = "Team added!".to_string();
                                }
                        });
                    });
                });
            });

        ui.add_space(15.0);

        // Teams list
        if self.config.teams.is_empty() {
            ui.label(RichText::new("No teams registered yet.").italics().color(Color32::from_rgb(156, 163, 175)));
        } else {
            let mut to_remove = None;
            let mut teams_changed = false;
            let divisions_list = &self.config.divisions;
            let teams_list = &mut self.config.teams;
            egui::Grid::new("teams_grid").num_columns(4).spacing(egui::vec2(20.0, 10.0)).show(ui, |ui| {
                ui.label(RichText::new("Team Name").strong());
                ui.label(RichText::new("Organization").strong());
                ui.label(RichText::new("Division").strong());
                ui.label(RichText::new("Actions").strong());
                ui.end_row();

                for (idx, team) in teams_list.iter_mut().enumerate() {
                    if ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut team.name)).changed() {
                        teams_changed = true;
                    }
                    if ui.add_sized([200.0, 20.0], egui::TextEdit::singleline(&mut team.organization)).changed() {
                        teams_changed = true;
                    }
                    let current_div_id = team.division_id.clone();
                    let div_exists = divisions_list.iter().any(|d| d.id == current_div_id);
                    
                    egui::ComboBox::from_id_source(format!("team_div_edit_{}", idx))
                        .selected_text(if div_exists {
                            divisions_list.iter()
                                .find(|d| d.id == current_div_id)
                                .map(|d| d.name.as_str())
                                .unwrap_or("Unknown")
                        } else {
                            "⚠ Select Division"
                        })
                        .show_ui(ui, |ui| {
                            for div in divisions_list {
                                if ui.selectable_value(&mut team.division_id, div.id.clone(), &div.name).clicked() {
                                    teams_changed = true;
                                }
                            }
                        });
                    
                    if ui.button("🗑 Delete").clicked() {
                        to_remove = Some(idx);
                    }
                    ui.end_row();
                }
            });

            if teams_changed {
                self.clear_schedule();
                self.update_diagnostics();
            }

            if let Some(idx) = to_remove {
                self.config.teams.remove(idx);
                self.clear_schedule();
                self.update_diagnostics();
                self.status_message = "Team deleted.".to_string();
            }
        }
    }
}
