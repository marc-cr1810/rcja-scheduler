use crate::gui::AppState;
use crate::gui::theme;
use crate::model::Team;
use eframe::egui::{self, RichText};

impl AppState {
    pub(super) fn draw_teams(&mut self, ui: &mut egui::Ui) {
        ui.heading("Teams Management");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.active_team_sub_tab,
                super::TeamSubTab::List,
                "👥 Team List",
            );
            ui.selectable_value(
                &mut self.active_team_sub_tab,
                super::TeamSubTab::GapAnalysis,
                "⚖ Gap Analysis",
            );
        });
        ui.add_space(10.0);

        match self.active_team_sub_tab {
            super::TeamSubTab::List => self.draw_team_list(ui),
            super::TeamSubTab::GapAnalysis => self.draw_team_gap_analysis(ui),
        }
    }

    fn draw_team_list(&mut self, ui: &mut egui::Ui) {
        // CSV Import Frame (Always accessible)
        egui::Frame::none()
            .fill(theme::card_bg())
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Bulk Import Teams from CSV").strong().color(theme::text()));
                    ui.label(RichText::new("Import teams from a RoboCup Junior registration CSV file. This will automatically create divisions if they don't exist.").size(11.0).color(theme::text_muted()));
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
            if crate::gui::helpers::draw_empty_state(
                ui,
                "👥",
                "No divisions defined",
                "Teams belong to a division. Create one first, then add teams here (or import a CSV above).",
                Some("Go to Divisions"),
            ) {
                self.active_tab = super::Tab::Divisions;
            }
            return;
        }

        // Add Individual Team Frame
        egui::Frame::none()
            .fill(theme::card_bg())
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Add Individual Team")
                            .strong()
                            .color(theme::text()),
                    );
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Team Name:");
                            ui.add_sized(
                                [220.0, 20.0],
                                egui::TextEdit::singleline(&mut self.new_team_name),
                            );
                        });

                        ui.vertical(|ui| {
                            ui.label("Organization:");
                            ui.add_sized(
                                [200.0, 20.0],
                                egui::TextEdit::singleline(&mut self.new_team_org),
                            );
                        });

                        ui.vertical(|ui| {
                            ui.label("Division:");
                            egui::ComboBox::from_id_source("team_div_combo")
                                .selected_text(
                                    self.config
                                        .divisions
                                        .iter()
                                        .find(|d| d.id == self.new_team_div_id)
                                        .map(|d| d.name.as_str())
                                        .unwrap_or("Select Division"),
                                )
                                .show_ui(ui, |ui| {
                                    for div in &self.config.divisions {
                                        ui.selectable_value(
                                            &mut self.new_team_div_id,
                                            div.id.clone(),
                                            &div.name,
                                        );
                                    }
                                });
                        });

                        ui.vertical(|ui| {
                            ui.label(""); // Spacer
                            if ui.button("+ Add Team").clicked()
                                && !self.new_team_name.trim().is_empty()
                                && !self.new_team_div_id.is_empty()
                            {
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
            ui.label(
                RichText::new("No teams registered yet.")
                    .italics()
                    .color(theme::text_muted()),
            );
        } else {
            let mut to_remove = None;
            let mut teams_changed = false;
            let divisions_list = &self.config.divisions;
            let teams_list = &mut self.config.teams;
            egui::Grid::new("teams_grid")
                .num_columns(4)
                .spacing(egui::vec2(20.0, 10.0))
                .striped(true)
                .show(ui, |ui| {
                    ui.label(RichText::new("Team Name").strong());
                    ui.label(RichText::new("Organization").strong());
                    ui.label(RichText::new("Division").strong());
                    ui.label(RichText::new("Actions").strong());
                    ui.end_row();

                    for (idx, team) in teams_list.iter_mut().enumerate() {
                        if ui
                            .add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut team.name))
                            .changed()
                        {
                            teams_changed = true;
                        }
                        if ui
                            .add_sized(
                                [200.0, 20.0],
                                egui::TextEdit::singleline(&mut team.organization),
                            )
                            .changed()
                        {
                            teams_changed = true;
                        }
                        let current_div_id = team.division_id.clone();
                        let div_exists = divisions_list.iter().any(|d| d.id == current_div_id);

                        egui::ComboBox::from_id_source(format!("team_div_edit_{}", idx))
                            .selected_text(if div_exists {
                                divisions_list
                                    .iter()
                                    .find(|d| d.id == current_div_id)
                                    .map(|d| d.name.as_str())
                                    .unwrap_or("Unknown")
                            } else {
                                "⚠ Select Division"
                            })
                            .show_ui(ui, |ui| {
                                for div in divisions_list {
                                    if ui
                                        .selectable_value(
                                            &mut team.division_id,
                                            div.id.clone(),
                                            &div.name,
                                        )
                                        .clicked()
                                    {
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

    fn draw_team_gap_analysis(&mut self, ui: &mut egui::Ui) {
        if self.schedule.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new("No Schedule Generated").size(16.0).color(theme::text_faint()).strong());
                ui.label("Gap analysis requires a generated schedule to calculate time between activities.");
            });
            return;
        }

        ui.label(
            RichText::new("TEAM ACTIVITY GAP ANALYSIS")
                .strong()
                .color(theme::text_muted()),
        );
        ui.label("Review wait times and busy periods for each team.");
        ui.add_space(10.0);

        let sched = self.schedule.as_ref().unwrap();
        let slots = &self.config.time_slots;
        let teams = &self.config.teams;

        if teams.is_empty() || slots.is_empty() {
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("gap_analysis_grid")
                .num_columns(6)
                .spacing([15.0, 8.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(RichText::new("Team").strong());
                    ui.label(RichText::new("Division").strong());
                    ui.label(RichText::new("Activities").strong());
                    ui.label(RichText::new("Day Range").strong());
                    ui.label(RichText::new("Min Gap").strong());
                    ui.label(RichText::new("Max Gap").strong());
                    ui.end_row();

                    for team in teams {
                        let mut team_activities: Vec<_> = sched
                            .assignments
                            .iter()
                            .filter(|a| a.activity.teams().contains(&team.name.as_str()))
                            .filter_map(|a| {
                                let slot = slots.iter().find(|s| s.id == a.time_slot_id)?;
                                let start_m =
                                    crate::gui::helpers::parse_time_to_minutes(&slot.start_time);
                                Some((slot.day.clone(), start_m, a.activity.duration_minutes()))
                            })
                            .collect();

                        team_activities.sort_by(|a, b| {
                            if a.0 != b.0 {
                                // Try to sort by day index in config
                                let pos_a = slots.iter().position(|s| s.day == a.0).unwrap_or(0);
                                let pos_b = slots.iter().position(|s| s.day == b.0).unwrap_or(0);
                                pos_a.cmp(&pos_b)
                            } else {
                                a.1.cmp(&b.1)
                            }
                        });

                        ui.label(&team.name);
                        ui.label(
                            self.config
                                .divisions
                                .iter()
                                .find(|d| d.id == team.division_id)
                                .map(|d| d.name.as_str())
                                .unwrap_or("Unknown"),
                        );
                        ui.label(team_activities.len().to_string());

                        if let (Some(first), Some(last)) =
                            (team_activities.first(), team_activities.last())
                        {
                            let day_range = if first.0 == last.0 {
                                format!(
                                    "{} ({} - {})",
                                    first.0,
                                    crate::gui::helpers::format_minutes_to_time(first.1),
                                    crate::gui::helpers::format_minutes_to_time(last.1 + last.2)
                                )
                            } else {
                                format!("{} - {}", first.0, last.0)
                            };
                            ui.label(day_range);

                            let mut min_gap = u32::MAX;
                            let mut max_gap = 0;

                            for i in 0..team_activities.len().saturating_sub(1) {
                                let a1 = &team_activities[i];
                                let a2 = &team_activities[i + 1];
                                if a1.0 == a2.0 {
                                    // Same-team activities can overlap after manual drag-and-drop
                                    // editing, which would make `start2 - end1` underflow. Skip
                                    // overlapping pairs and clamp with saturating_sub for safety.
                                    let end1 = a1.1 + a1.2;
                                    if a2.1 >= end1 {
                                        let gap = a2.1 - end1;
                                        if gap < min_gap {
                                            min_gap = gap;
                                        }
                                        if gap > max_gap {
                                            max_gap = gap;
                                        }
                                    }
                                }
                            }

                            if min_gap == u32::MAX {
                                ui.label("-");
                                ui.label("-");
                            } else {
                                ui.label(RichText::new(format!("{}m", min_gap)).color(
                                    if min_gap < 15 {
                                        theme::danger()
                                    } else {
                                        theme::text()
                                    },
                                ));
                                ui.label(RichText::new(format!("{}m", max_gap)).color(
                                    if max_gap > 180 {
                                        theme::warning()
                                    } else {
                                        theme::text()
                                    },
                                ));
                            }
                        } else {
                            ui.label("No activities");
                            ui.label("-");
                            ui.label("-");
                        }
                        ui.end_row();
                    }
                });
        });
    }
}
