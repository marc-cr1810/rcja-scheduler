use crate::gui::AppState;
use crate::gui::theme;
use crate::model::{SchedulingMode, TimeSlot};
use eframe::egui::{self, Color32, RichText};

impl AppState {
    pub(super) fn draw_time_slots(&mut self, ui: &mut egui::Ui) {
        ui.heading("Time Slots Manager");
        ui.add_space(10.0);

        // Ensure we always have at least one day configuration
        if self.config.day_configs.is_empty() {
            self.config.day_configs.push(crate::model::DayGenConfig::default());
        }

        // 1. Group Auto-Calculate & Generate in a nice frame
        egui::Frame::none()
            .fill(theme::card_bg())
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                // Global settings
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Match Slot Duration (min):").strong());
                    ui.add(egui::DragValue::new(&mut self.gen_slot_duration).clamp_range(5..=180));
                    ui.add_space(15.0);
                    ui.label(RichText::new("Interview Slot Duration (min):").strong());
                    ui.add(egui::DragValue::new(&mut self.gen_interview_slot_duration).clamp_range(5..=60));
                    ui.add_space(15.0);
                    ui.label(RichText::new("Match Break (min):").strong());
                    ui.add(egui::DragValue::new(&mut self.gen_match_slot_break).clamp_range(0..=60));
                    ui.add_space(15.0);
                    ui.label(RichText::new("Interview Break (min):").strong());
                    ui.add(egui::DragValue::new(&mut self.gen_interview_slot_break).clamp_range(0..=60));
                });

                ui.add_space(8.0);
                ui.label(RichText::new("💡 Tip: Click the button below to regenerate slots after changing durations or breaks.").italics().color(theme::text_muted()));

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                ui.label(RichText::new("Day Configurations:").strong().color(theme::text()));
                ui.add_space(4.0);

                // Grid of day configurations
                let mut to_remove_day = None;
                
                let day_choices = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

                for (idx, day_cfg) in self.config.day_configs.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        // Dropdown for day
                        egui::ComboBox::from_id_source(format!("day_combo_{}", idx))
                            .selected_text(&day_cfg.day)
                            .width(100.0)
                            .show_ui(ui, |ui| {
                                for d in &day_choices {
                                    ui.selectable_value(&mut day_cfg.day, d.to_string(), *d);
                                }
                            });

                        ui.label("Start:");
                        ui.add(crate::gui::helpers::time_edit(&mut day_cfg.start_time))
                            .on_hover_text("Format: HH:MM, e.g. 09:00");

                        ui.label("End:");
                        ui.add(crate::gui::helpers::time_edit(&mut day_cfg.end_time))
                            .on_hover_text("Format: HH:MM, e.g. 17:00");

                        ui.checkbox(&mut day_cfg.lunch_enabled, "Lunch Break");
                        ui.checkbox(&mut day_cfg.interviews_enabled, "Interviews");

                        if day_cfg.lunch_enabled {
                            ui.label("Lunch Start:");
                            ui.add(crate::gui::helpers::time_edit(&mut day_cfg.lunch_start))
                                .on_hover_text("Format: HH:MM, e.g. 12:00");

                            ui.label("Lunch Duration (min):");
                            ui.add(egui::DragValue::new(&mut day_cfg.lunch_duration).clamp_range(15..=120));
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // Only allow deletion if there's more than one config, or always allow but we auto-add default if empty
                            let delete_btn = egui::Button::new(RichText::new("🗑").color(theme::danger()))
                                .fill(Color32::TRANSPARENT);
                            if ui.add(delete_btn).on_hover_text("Delete this day configuration").clicked() {
                                to_remove_day = Some(idx);
                            }
                        });
                    });
                    ui.add_space(4.0);
                }

                if let Some(idx) = to_remove_day {
                    self.config.day_configs.remove(idx);
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("➕ Add Day Configuration").strong().color(theme::accent())).clicked() {
                        let week = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
                        let used: std::collections::HashSet<String> = self.config.day_configs.iter().map(|c| c.day.clone()).collect();
                        let last_day = self.config.day_configs.last().map(|c| c.day.clone()).unwrap_or_else(|| "Friday".to_string());
                        let last_idx = week.iter().position(|d| *d == last_day).unwrap_or(4);
                        // Pick the next weekday after the last one that isn't already configured (wrapping
                        // through the week). Only falls back to a duplicate if all seven days are in use.
                        let next_day = (1..=7)
                            .map(|offset| week[(last_idx + offset) % 7])
                            .find(|d| !used.contains(*d))
                            .unwrap_or(week[(last_idx + 1) % 7])
                            .to_string();
                        self.config.day_configs.push(crate::model::DayGenConfig {
                            day: next_day,
                            start_time: "09:00".to_string(),
                            end_time: "17:00".to_string(),
                            lunch_enabled: true,
                            lunch_start: "12:00".to_string(),
                            lunch_duration: 60,
                            interviews_enabled: true,
                        });
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let gen_btn = egui::Button::new(RichText::new("⚡ Auto-Calculate & Generate").strong().color(theme::on_accent()))
                            .fill(theme::accent_strong())
                            .rounding(6.0)
                            .min_size(egui::vec2(180.0, 30.0));
                        if ui.add(gen_btn).clicked() {
                            self.auto_generate_time_slots();
                        }
                    });
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                // Summary calculations
                let mut total_games = 0;
                for div in &self.config.divisions {
                    let div_teams_count = self.config.teams.iter().filter(|t| t.division_id == div.id).count();
                    if div_teams_count < 2 && div.mode == crate::model::SchedulingMode::HeadToHead {
                        continue;
                    }
                    let games = match div.mode {
                        crate::model::SchedulingMode::HeadToHead => {
                            let rr_matches = crate::scheduler::activity::compute_rr_match_count(div_teams_count, div.games_per_team);
                            let mut finals_matches = if div.finals_enabled {
                                match div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand) {
                                    crate::model::FinalsRounds::Grand => 1,
                                    crate::model::FinalsRounds::Semis => 3,
                                    crate::model::FinalsRounds::Quarter => 7,
                                    crate::model::FinalsRounds::Eighths => 15,
                                }
                            } else {
                                0
                            };
                            if div.finals_enabled && div.finals_third_place_playoff && div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand) != crate::model::FinalsRounds::Grand {
                                finals_matches += 1;
                            }
                            rr_matches + finals_matches
                        }
                        crate::model::SchedulingMode::IndividualRun => {
                            div_teams_count * div.games_per_team
                        }
                    };
                    total_games += games;
                    if div.interviews_enabled {
                        total_games += div_teams_count;
                    }
                }
                let total_fields = self.config.fields.len();
                let required_slots = if total_fields > 0 { total_games.div_ceil(total_fields) } else { 0 };
                let total_generated = self.config.time_slots.len();

                ui.horizontal(|ui| {
                    ui.label(RichText::new("Summary Info:").strong().color(theme::text_muted()));
                    ui.label(format!("Total matches & runs to schedule: {}", total_games));
                    ui.label("|");
                    ui.label(format!("Fields: {}", total_fields));
                    ui.label("|");
                    ui.label(RichText::new(format!("Required slots: {}", required_slots)).strong().color(theme::accent()));
                    ui.label("|");
                    let status_color = if total_generated >= required_slots {
                        theme::success() // Green
                    } else {
                        theme::danger() // Red
                    };
                    ui.label(RichText::new(format!("Generated slots: {}", total_generated)).strong().color(status_color));
                });
            });

        ui.add_space(15.0);

        if ui.button("⚠ Clear All Time Slots").clicked() {
            self.config.time_slots.clear();
            self.clear_schedule();
            self.update_diagnostics();
            self.status_message = "All time slots cleared.".to_string();
        }

        ui.add_space(10.0);

        // Slots Grid
        let mut to_remove = None;
        egui::Grid::new("slots_grid").num_columns(6).spacing(egui::vec2(20.0, 10.0)).show(ui, |ui| {
            ui.label(RichText::new("Day").strong());
            ui.label(RichText::new("Start Time").strong());
            ui.label(RichText::new("End Time").strong());
            ui.label(RichText::new("Duration (min)").strong());
            ui.label(RichText::new("Kind").strong());
            ui.label(RichText::new("Actions").strong());
            ui.end_row();

            for (idx, slot) in self.config.time_slots.iter().enumerate() {
                ui.label(&slot.day);
                ui.label(RichText::new(&slot.start_time).strong().color(theme::text()));
                ui.label(RichText::new(&slot.end_time).strong().color(theme::text()));
                ui.label(format!("{} mins", slot.duration_minutes()));
                
                let (kind_icon, kind_color) = match slot.kind {
                    crate::model::FieldKind::Competition => ("⚔", Color32::from_rgb(147, 197, 253)), // Blue
                    crate::model::FieldKind::Interview => ("💬", Color32::from_rgb(253, 186, 116)), // Yellow/Orange
                };
                ui.label(RichText::new(format!("{} {:?}", kind_icon, slot.kind)).color(kind_color));
                
                if ui.button("🗑 Delete").clicked() {
                    to_remove = Some(idx);
                }
                ui.end_row();
            }
        });

        if let Some(idx) = to_remove {
            self.config.time_slots.remove(idx);
            self.clear_schedule();
            self.update_diagnostics();
            self.status_message = "Time slot deleted.".to_string();
        }
    }

    pub fn sort_time_slots(&mut self) {
        self.config.time_slots.sort_by(|a, b| {
            let day_order = |d: &str| -> usize {
                match d.to_lowercase().as_str() {
                    "monday" => 1,
                    "tuesday" => 2,
                    "wednesday" => 3,
                    "thursday" => 4,
                    "friday" => 5,
                    "saturday" => 6,
                    "sunday" => 7,
                    _ => 8,
                }
            };
            
            let a_day = day_order(&a.day);
            let b_day = day_order(&b.day);
            
            if a_day != b_day {
                a_day.cmp(&b_day)
            } else {
                a.start_minutes().cmp(&b.start_minutes())
            }
        });
    }

    pub fn auto_generate_time_slots(&mut self) {
        self.clear_schedule();
        let slot_duration = self.gen_slot_duration;

        let mut total_games = 0;
        for div in &self.config.divisions {
            let div_teams_count = self.config.teams.iter()
                .filter(|t| t.division_id == div.id)
                .count();
            if div_teams_count < 2 && div.mode == SchedulingMode::HeadToHead {
                continue;
            }
            let games = match div.mode {
                SchedulingMode::HeadToHead => {
                    let rr_matches = crate::scheduler::activity::compute_rr_match_count(div_teams_count, div.games_per_team);
                    let mut finals_matches = if div.finals_enabled {
                        match div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand) {
                            crate::model::FinalsRounds::Grand => 1,
                            crate::model::FinalsRounds::Semis => 3,
                            crate::model::FinalsRounds::Quarter => 7,
                            crate::model::FinalsRounds::Eighths => 15,
                        }
                    } else {
                        0
                    };
                    if div.finals_enabled && div.finals_third_place_playoff && div.finals_rounds.unwrap_or(crate::model::FinalsRounds::Grand) != crate::model::FinalsRounds::Grand {
                        finals_matches += 1;
                    }
                    rr_matches + finals_matches
                }
                SchedulingMode::IndividualRun => {
                    div_teams_count * div.games_per_team
                }
            };
            total_games += games;
            
            if div.interviews_enabled {
                total_games += div_teams_count;
            }
        }

        let total_fields = self.config.fields.len();
        if total_fields == 0 {
            self.status_message = "Error: Please add at least one field first to allocate slots!".to_string();
            return;
        }

        let required_slots = total_games.div_ceil(total_fields);

        if required_slots == 0 {
            self.status_message = "Error: No teams or divisions defined to calculate matches!".to_string();
            return;
        }

        if self.config.day_configs.is_empty() {
            self.status_message = "Error: No day configurations specified for auto-generation!".to_string();
            return;
        }

        let mut old_day_slots = Vec::new();
        type VolunteerRanges = Vec<(String, Vec<(u32, u32)>)>;
        let mut volunteer_available_ranges: Vec<VolunteerRanges> = vec![Vec::new(); self.config.volunteers.len()];

        for day_config in &self.config.day_configs {
            let target_day_lower = day_config.day.to_lowercase();
            let day_slots: Vec<(String, u32, u32)> = self.config.time_slots.iter()
                .filter(|s| s.day.to_lowercase() == target_day_lower)
                .map(|s| (s.id.clone(), s.start_minutes(), s.start_minutes() + s.duration_minutes()))
                .collect();
            
            for (vol_idx, vol) in self.config.volunteers.iter().enumerate() {
                let ranges: Vec<(u32, u32)> = day_slots.iter()
                    .filter(|(slot_id, _, _)| vol.availabilities.contains(slot_id))
                    .map(|(_, start, end)| (*start, *end))
                    .collect();
                volunteer_available_ranges[vol_idx].push((day_config.day.clone(), ranges));
            }
            
            old_day_slots.extend(day_slots);
        }

        for day_config in &self.config.day_configs {
            let target_day_lower = day_config.day.to_lowercase();
            self.config.time_slots.retain(|s| s.day.to_lowercase() != target_day_lower);
        }

        let mut total_generated = 0;

        for day_config in &self.config.day_configs {
            let start_parts: Vec<&str> = day_config.start_time.split(':').collect();
            if start_parts.len() != 2 {
                self.status_message = format!("Error: Invalid start time format for {}. Use HH:MM.", day_config.day);
                return;
            }

            let mut h = match start_parts[0].parse::<u32>() {
                Ok(val) => val,
                Err(_) => { self.status_message = format!("Error parsing start hour for {}.", day_config.day); return; }
            };
            let mut m = match start_parts[1].parse::<u32>() {
                Ok(val) => val,
                Err(_) => { self.status_message = format!("Error parsing start minute for {}.", day_config.day); return; }
            };

            let end_parts: Vec<&str> = day_config.end_time.split(':').collect();
            if end_parts.len() != 2 {
                self.status_message = format!("Error: Invalid end time format for {}. Use HH:MM.", day_config.day);
                return;
            }

            let end_h = match end_parts[0].parse::<u32>() {
                Ok(val) => val,
                Err(_) => { self.status_message = format!("Error parsing end hour for {}.", day_config.day); return; }
            };
            let end_m = match end_parts[1].parse::<u32>() {
                Ok(val) => val,
                Err(_) => { self.status_message = format!("Error parsing end minute for {}.", day_config.day); return; }
            };
            let limit_minutes = end_h * 60 + end_m;

            let mut lunch_start_minutes = 0;
            let mut lunch_end_minutes = 0;
            if day_config.lunch_enabled {
                let lunch_parts: Vec<&str> = day_config.lunch_start.split(':').collect();
                if lunch_parts.len() == 2
                    && let (Ok(lh), Ok(lm)) = (lunch_parts[0].parse::<u32>(), lunch_parts[1].parse::<u32>()) {
                        lunch_start_minutes = lh * 60 + lm;
                        lunch_end_minutes = lunch_start_minutes + day_config.lunch_duration;
                    }
            }

            let mut slot_index = 1;
            loop {
                let current_start_minutes = h * 60 + m;
                let current_end_minutes = current_start_minutes + slot_duration;

                if current_end_minutes > limit_minutes {
                    break;
                }

                if day_config.lunch_enabled
                    && current_start_minutes < lunch_end_minutes && current_end_minutes > lunch_start_minutes {
                        h = lunch_end_minutes / 60;
                        m = lunch_end_minutes % 60;
                        continue;
                    }

                let start_str = format!("{:02}:{:02}", h, m);
                let slot_end_h = current_end_minutes / 60;
                let slot_end_m = current_end_minutes % 60;
                let end_str = format!("{:02}:{:02}", slot_end_h, slot_end_m);

                self.config.time_slots.push(TimeSlot {
                    id: format!("{}_auto_slot_comp_{}", day_config.day.to_lowercase(), slot_index),
                    day: day_config.day.clone(),
                    start_time: start_str,
                    end_time: end_str,
                    kind: crate::model::FieldKind::Competition,
                });

                slot_index += 1;
                total_generated += 1;

                let next_start_minutes = current_start_minutes + slot_duration + self.gen_match_slot_break;
                h = next_start_minutes / 60;
                m = next_start_minutes % 60;
            }

            // Generate Interview Slots
            if day_config.interviews_enabled {
                let mut int_h = start_parts[0].parse::<u32>().unwrap_or_default();
                let mut int_m = start_parts[1].parse::<u32>().unwrap_or_default();
                let int_slot_duration = self.gen_interview_slot_duration;
                let mut int_slot_index = 1;

                loop {
                    let current_start_minutes = int_h * 60 + int_m;
                    let current_end_minutes = current_start_minutes + int_slot_duration;

                    if current_end_minutes > limit_minutes {
                        break;
                    }

                    if day_config.lunch_enabled
                        && current_start_minutes < lunch_end_minutes && current_end_minutes > lunch_start_minutes {
                            int_h = lunch_end_minutes / 60;
                            int_m = lunch_end_minutes % 60;
                            continue;
                        }

                    let start_str = format!("{:02}:{:02}", int_h, int_m);
                    let slot_end_h = current_end_minutes / 60;
                    let slot_end_m = current_end_minutes % 60;
                    let end_str = format!("{:02}:{:02}", slot_end_h, slot_end_m);

                    self.config.time_slots.push(TimeSlot {
                        id: format!("{}_auto_slot_int_{}", day_config.day.to_lowercase(), int_slot_index),
                        day: day_config.day.clone(),
                        start_time: start_str,
                        end_time: end_str,
                        kind: crate::model::FieldKind::Interview,
                    });

                    int_slot_index += 1;

                    let next_start_minutes = current_start_minutes + int_slot_duration + self.gen_interview_slot_break;
                    int_h = next_start_minutes / 60;
                    int_m = next_start_minutes % 60;
                }
            }
        }

        self.sort_time_slots();

        for (vol_idx, vol) in self.config.volunteers.iter_mut().enumerate() {
            let old_slot_ids: std::collections::HashSet<&str> = old_day_slots.iter()
                .map(|(id, _, _)| id.as_str())
                .collect();
            vol.availabilities.retain(|id| !old_slot_ids.contains(id.as_str()));

            for (day_name, old_ranges) in &volunteer_available_ranges[vol_idx] {
                if old_ranges.is_empty() {
                    continue;
                }
                
                let day_lower = day_name.to_lowercase();
                let day_new_slots: Vec<(String, u32, u32)> = self.config.time_slots.iter()
                    .filter(|s| s.day.to_lowercase() == day_lower)
                    .map(|s| (s.id.clone(), s.start_minutes(), s.start_minutes() + s.duration_minutes()))
                    .collect();

                for (new_id, new_start, new_end) in &day_new_slots {
                    let overlaps = old_ranges.iter().any(|(old_start, old_end)| {
                        new_start < old_end && new_end > old_start
                    });
                    if overlaps && !vol.availabilities.contains(new_id) {
                        vol.availabilities.push(new_id.clone());
                    }
                }
            }
        }

        self.clear_schedule();
        self.update_diagnostics();
        self.status_message = format!(
            "Success! Generated {} slots of duration {}m across {} day(s).",
            total_generated, slot_duration, self.config.day_configs.len()
        );
    }
}
