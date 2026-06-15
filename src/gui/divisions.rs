use crate::gui::AppState;
use crate::gui::theme;
use crate::model::{Division, SchedulingMode, FinalsRounds};
use crate::scheduler::sanitize_name;
use crate::gui::helpers::draw_card;
use eframe::egui::{self, RichText, Stroke};

impl AppState {
    pub(super) fn draw_divisions(&mut self, ui: &mut egui::Ui) {
        ui.heading("Divisions Management");
        ui.add_space(10.0);

        // Add Division Card
        draw_card(ui, "Add New Division", true, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Name:");
                            ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut self.new_div_name));
                        });

                        ui.vertical(|ui| {
                            ui.label("Format:");
                            ui.horizontal(|ui| {
                                ui.radio_value(&mut self.new_div_mode, SchedulingMode::HeadToHead, "Soccer (Head-to-Head)");
                                ui.radio_value(&mut self.new_div_mode, SchedulingMode::IndividualRun, "Rescue/OnStage (Runs)");
                            });
                        });
                    });

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            match self.new_div_mode {
                                SchedulingMode::HeadToHead => {
                                    ui.label("Games per Team:").on_hover_text(
                                        "Total number of round-robin games each team should play.\n\
                                         If this is less than (teams - 1), it will be a partial round robin.\n\
                                         If it is a multiple of (teams - 1), it will be multiple full round robins."
                                    );
                                }
                                SchedulingMode::IndividualRun => {
                                    ui.label("Runs per Team:");
                                }
                            }
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut self.new_div_games).clamp_range(1..=10));
                                
                                if self.new_div_mode == SchedulingMode::HeadToHead {
                                    let n = self.config.teams.iter().filter(|t| t.division_id == sanitize_name(&self.new_div_name)).count();
                                    if n >= 2 {
                                        let full_rr = n - 1;
                                        let is_partial = !self.new_div_games.is_multiple_of(full_rr);
                                        if is_partial {
                                            ui.label(RichText::new("⚠").color(theme::warning()))
                                                .on_hover_text(format!(
                                                    "Partial Round Robin: Each team plays {} games.\n\
                                                     A full round robin requires {} games per team (or multiples: {}, {}, {}, etc.).\n\
                                                     In this mode, not every team will play every other team equally.",
                                                    self.new_div_games, full_rr, full_rr, full_rr * 2, full_rr * 3
                                                ));
                                        }
                                    }
                                }
                            });
                        });

                        ui.vertical(|ui| {
                            ui.label("Match Duration (min):");
                            ui.add(egui::DragValue::new(&mut self.new_div_duration).clamp_range(5..=60));
                        });

                        ui.vertical(|ui| {
                            ui.label("Referees Required:");
                            ui.add(egui::DragValue::new(&mut self.new_div_volunteers).clamp_range(1..=4));
                        });

                        ui.vertical(|ui| {
                            ui.label("Color:");
                            ui.color_edit_button_srgb(&mut self.new_div_color);
                        });
                    });

                    ui.add_space(5.0);
                    if self.new_div_mode == SchedulingMode::HeadToHead {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.new_div_finals_enabled, "Finals Enabled");
                            if !self.new_div_finals_enabled {
                                self.new_div_finals_third_place_playoff = false;
                            }
                            if self.new_div_finals_enabled {
                                ui.label("Finals Format:");
                                egui::ComboBox::from_id_source("new_div_finals_rounds")
                                    .selected_text(match self.new_div_finals_rounds {
                                        FinalsRounds::Grand => "Grand Final (Top 2)",
                                        FinalsRounds::Semis => "Semi-Finals (Top 4)",
                                        FinalsRounds::Quarter => "Quarter-Finals (Top 8)",
                                        FinalsRounds::Eighths => "Eighth-Finals (Top 16)",
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.new_div_finals_rounds, FinalsRounds::Grand, "Grand Final (Top 2)");
                                        ui.selectable_value(&mut self.new_div_finals_rounds, FinalsRounds::Semis, "Semi-Finals (Top 4)");
                                        ui.selectable_value(&mut self.new_div_finals_rounds, FinalsRounds::Quarter, "Quarter-Finals (Top 8)");
                                        ui.selectable_value(&mut self.new_div_finals_rounds, FinalsRounds::Eighths, "Eighth-Finals (Top 16)");
                                    });

                                if self.new_div_finals_enabled {
                                    let n = self.config.teams.iter().filter(|t| t.division_id == sanitize_name(&self.new_div_name)).count();
                                    let required = match self.new_div_finals_rounds {
                                        FinalsRounds::Grand => 2,
                                        FinalsRounds::Semis => 4,
                                        FinalsRounds::Quarter => 8,
                                        FinalsRounds::Eighths => 16,
                                    };
                                    if n < required {
                                        ui.label(RichText::new("⚠").color(theme::danger()))
                                            .on_hover_text(format!(
                                                "Insufficient teams: {} teams in division, but '{}' requires at least {}.",
                                                n, match self.new_div_finals_rounds {
                                                    FinalsRounds::Grand => "Grand Final",
                                                    FinalsRounds::Semis => "Semi-Finals",
                                                    FinalsRounds::Quarter => "Quarter-Finals",
                                                    FinalsRounds::Eighths => "Eighth-Finals",
                                                }, required
                                            ));
                                    }
                                }

                                if self.new_div_finals_rounds == FinalsRounds::Grand {
                                    self.new_div_finals_third_place_playoff = false;
                                }

                                if self.new_div_finals_rounds != FinalsRounds::Grand {
                                    ui.checkbox(&mut self.new_div_finals_third_place_playoff, "3rd Place Playoff");
                                }

                                ui.checkbox(&mut self.new_div_custom_finals_duration, "Custom Duration");
                                if self.new_div_custom_finals_duration {
                                    ui.add(egui::DragValue::new(&mut self.new_div_finals_duration).clamp_range(5..=60));
                                    ui.label("min");
                                }
                            }
                        });
                        ui.add_space(5.0);
                    }

                    ui.checkbox(&mut self.new_div_interviews, "Enable Interviews");
                    if self.new_div_interviews {
                        ui.horizontal(|ui| {
                            ui.label("Interview Duration (min):");
                            ui.add(egui::DragValue::new(&mut self.new_div_int_dur).clamp_range(5..=30));
                            ui.label("Interview Judges:");
                            ui.add(egui::DragValue::new(&mut self.new_div_int_vols).clamp_range(1..=4));
                        });
                    }

                    ui.add_space(8.0);
                    if ui.button(RichText::new("+ Create Division").strong().color(theme::text())).clicked()
                        && !self.new_div_name.trim().is_empty() {
                            let existing_ids: Vec<String> = self.config.divisions.iter().map(|d| d.id.clone()).collect();
                            let id = crate::scheduler::unique_id(&sanitize_name(&self.new_div_name), &existing_ids);
                            let finals_dur = if self.new_div_finals_enabled && self.new_div_custom_finals_duration {
                                Some(self.new_div_finals_duration)
                            } else {
                                None
                            };

                            self.config.divisions.push(Division {
                                id: id.clone(),
                                name: self.new_div_name.clone(),
                                mode: self.new_div_mode,
                                games_per_team: self.new_div_games,
                                volunteers_required: self.new_div_volunteers,
                                duration_minutes: self.new_div_duration,
                                allowed_fields: None,
                                interviews_enabled: self.new_div_interviews,
                                interview_volunteers_required: self.new_div_int_vols,
                                interview_duration_minutes: self.new_div_int_dur,
                                finals_enabled: self.new_div_finals_enabled,
                                finals_rounds: if self.new_div_finals_enabled { Some(self.new_div_finals_rounds) } else { None },
                                finals_duration_minutes: finals_dur,
                                finals_third_place_playoff: self.new_div_finals_third_place_playoff,
                                color: Some(self.new_div_color),
                                min_match_break_minutes: None,
                            });

                            self.new_div_name.clear();
                            self.new_div_finals_enabled = false;
                            self.new_div_custom_finals_duration = false;
                            self.new_div_finals_third_place_playoff = false;
                            
                            use rand::Rng;
                            let mut rng = rand::thread_rng();
                            self.new_div_color = [
                                rng.gen_range(50..=200),
                                rng.gen_range(50..=200),
                                rng.gen_range(50..=200),
                            ];
                            
                            self.update_diagnostics();
                        }
                });
            });

        ui.add_space(15.0);

        if self.config.divisions.is_empty() {
            crate::gui::helpers::draw_empty_state(
                ui,
                "🏆",
                "No divisions yet",
                "Create your first division above to start building the competition.",
                None,
            );
            return;
        }

        ui.label(RichText::new("EXISTING DIVISIONS").strong().color(theme::text_muted()));
        ui.add_space(5.0);

        let mut division_to_delete = None;

        for (idx, div) in self.config.divisions.clone().into_iter().enumerate() {
            let mut div = div;
            let mut divisions_changed = false;
            let icon = match div.mode {
                SchedulingMode::HeadToHead => "⚽",
                SchedulingMode::IndividualRun => "🤖",
            };
            
            egui::Frame::none()
                .fill(theme::card_bg_alt())
                .rounding(8.0)
                .stroke(Stroke::new(1.0, theme::border()))
                .inner_margin(egui::Margin::symmetric(12.0, 10.0))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(icon).strong().size(15.0).color(theme::text()));
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&div.name).strong().size(14.0).color(theme::text()));
                                ui.label(RichText::new(format!("ID: {}", div.id)).size(10.0).color(theme::text_faint()));
                            });
                            
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button(RichText::new("🗑 Delete").color(theme::danger())).clicked() {
                                    division_to_delete = Some(idx);
                                }
                            });
                        });
                        
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(8.0);

                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            if ui.text_edit_singleline(&mut div.name).changed() {
                                divisions_changed = true;
                            }

                            ui.label("Format:");
                            if ui.radio_value(&mut div.mode, SchedulingMode::HeadToHead, "Soccer").changed() {
                                divisions_changed = true;
                            }
                            if ui.radio_value(&mut div.mode, SchedulingMode::IndividualRun, "Rescue").changed() {
                                divisions_changed = true;
                            }
                        });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            match div.mode {
                                SchedulingMode::HeadToHead => {
                                    ui.label("Games per Team:");
                                }
                                SchedulingMode::IndividualRun => {
                                    ui.label("Runs per Team:");
                                }
                            }
                            if ui.add(egui::DragValue::new(&mut div.games_per_team).clamp_range(1..=10)).changed() {
                                divisions_changed = true;
                            }
                            
                            if div.mode == SchedulingMode::HeadToHead {
                                let n = self.config.teams.iter().filter(|t| t.division_id == div.id).count();
                                if n >= 2 {
                                    let full_rr = n - 1;
                                    let is_partial = div.games_per_team % full_rr != 0;
                                    if is_partial {
                                        ui.label(RichText::new("⚠").color(theme::warning()))
                                            .on_hover_text(format!(
                                                "Partial Round Robin: Each team plays {} games.\n\
                                                 A full round robin requires {} games per team (or multiples: {}, {}, {}, etc.).\n\
                                                 In this mode, not every team will play every other team equally.",
                                                div.games_per_team, full_rr, full_rr, full_rr * 2, full_rr * 3
                                            ));
                                    }
                                } else {
                                    ui.label(RichText::new("⚠").color(theme::danger()))
                                        .on_hover_text("Need at least 2 teams in this division to play matches.");
                                }
                            }

                            ui.label("Match Duration (min):");
                            if ui.add(egui::DragValue::new(&mut div.duration_minutes).clamp_range(5..=60)).changed() {
                                divisions_changed = true;
                            }

                            ui.label("Referees Required:");
                            if ui.add(egui::DragValue::new(&mut div.volunteers_required).clamp_range(1..=4)).changed() {
                                divisions_changed = true;
                            }

                            ui.label("Color:");
                            let mut color = div.color.unwrap_or([79, 70, 229]);
                            if ui.color_edit_button_srgb(&mut color).changed() {
                                div.color = Some(color);
                                divisions_changed = true;
                            }
                        });

                        if div.mode == SchedulingMode::HeadToHead {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui.checkbox(&mut div.finals_enabled, "Finals Enabled").changed() {
                                    if !div.finals_enabled {
                                        div.finals_third_place_playoff = false;
                                    }
                                    divisions_changed = true;
                                }
                                if div.finals_enabled {
                                    let mut rounds = div.finals_rounds.unwrap_or(FinalsRounds::Grand);
                                    ui.label("Finals Format:");
                                    let combobox_id = format!("finals_rounds_{}", div.id);
                                    egui::ComboBox::from_id_source(combobox_id)
                                        .selected_text(match rounds {
                                            FinalsRounds::Grand => "Grand Final (Top 2)",
                                            FinalsRounds::Semis => "Semi-Finals (Top 4)",
                                            FinalsRounds::Quarter => "Quarter-Finals (Top 8)",
                                            FinalsRounds::Eighths => "Eighth-Finals (Top 16)",
                                        })
                                        .show_ui(ui, |ui| {
                                            let mut changed = false;
                                            changed |= ui.selectable_value(&mut rounds, FinalsRounds::Grand, "Grand Final (Top 2)").changed();
                                            changed |= ui.selectable_value(&mut rounds, FinalsRounds::Semis, "Semi-Finals (Top 4)").changed();
                                            changed |= ui.selectable_value(&mut rounds, FinalsRounds::Quarter, "Quarter-Finals (Top 8)").changed();
                                            changed |= ui.selectable_value(&mut rounds, FinalsRounds::Eighths, "Eighth-Finals (Top 16)").changed();
                                            if changed {
                                                div.finals_rounds = Some(rounds);
                                                if rounds == FinalsRounds::Grand {
                                                    div.finals_third_place_playoff = false;
                                                }
                                                divisions_changed = true;
                                            }
                                        });

                                    let n = self.config.teams.iter().filter(|t| t.division_id == div.id).count();
                                    let required = match rounds {
                                        FinalsRounds::Grand => 2,
                                        FinalsRounds::Semis => 4,
                                        FinalsRounds::Quarter => 8,
                                        FinalsRounds::Eighths => 16,
                                    };
                                    if n < required {
                                        ui.label(RichText::new("⚠").color(theme::danger()))
                                            .on_hover_text(format!(
                                                "Insufficient teams: {} teams in division, but '{}' requires at least {}.",
                                                n, match rounds {
                                                    FinalsRounds::Grand => "Grand Final",
                                                    FinalsRounds::Semis => "Semi-Finals",
                                                    FinalsRounds::Quarter => "Quarter-Finals",
                                                    FinalsRounds::Eighths => "Eighth-Finals",
                                                }, required
                                            ));
                                    }

                                    if rounds != FinalsRounds::Grand
                                        && ui.checkbox(&mut div.finals_third_place_playoff, "3rd Place Playoff").changed() {
                                            divisions_changed = true;
                                        }

                                    let mut has_custom_dur = div.finals_duration_minutes.is_some();
                                    if ui.checkbox(&mut has_custom_dur, "Custom Duration").changed() {
                                        if has_custom_dur {
                                            div.finals_duration_minutes = Some(div.duration_minutes);
                                        } else {
                                            div.finals_duration_minutes = None;
                                        }
                                        divisions_changed = true;
                                    }
                                    if let Some(ref mut dur) = div.finals_duration_minutes {
                                        if ui.add(egui::DragValue::new(dur).clamp_range(5..=60)).changed() {
                                            divisions_changed = true;
                                        }
                                        ui.label("min");
                                    }
                                }
                            });
                        }

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut div.interviews_enabled, "Enable Interviews").changed() {
                                divisions_changed = true;
                            }
                            if div.interviews_enabled {
                                ui.label("Interview Duration (min):");
                                if ui.add(egui::DragValue::new(&mut div.interview_duration_minutes).clamp_range(5..=30)).changed() {
                                    divisions_changed = true;
                                }
                                ui.label("Interview Judges:");
                                if ui.add(egui::DragValue::new(&mut div.interview_volunteers_required).clamp_range(1..=4)).changed() {
                                    divisions_changed = true;
                                }
                            }
                        });

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let mut has_override = div.min_match_break_minutes.is_some();
                            if ui.checkbox(&mut has_override, "Override recharge break")
                                .on_hover_text("Set a division-specific minimum break between this division's consecutive matches/runs, overriding the global solver default.")
                                .changed()
                            {
                                div.min_match_break_minutes = if has_override {
                                    Some(self.solver_team_match_min_break_minutes)
                                } else {
                                    None
                                };
                                divisions_changed = true;
                            }
                            if let Some(ref mut mins) = div.min_match_break_minutes {
                                if ui.add(egui::DragValue::new(mins).clamp_range(0..=120).suffix(" min")).changed() {
                                    divisions_changed = true;
                                }
                                ui.label(RichText::new("(0 = no recharge break for this division)").size(10.0).color(theme::text_faint()));
                            } else {
                                ui.label(RichText::new(format!("Inheriting global default ({} min)", self.solver_team_match_min_break_minutes))
                                    .size(10.0).color(theme::text_faint()));
                            }
                        });

                        if divisions_changed {
                            self.config.divisions[idx] = div;
                            self.clear_schedule();
                            self.update_diagnostics();
                        }
                    });
                });
            ui.add_space(10.0);
        }

        if let Some(idx) = division_to_delete {
            self.config.divisions.remove(idx);
            self.update_diagnostics();
        }
    }
}
