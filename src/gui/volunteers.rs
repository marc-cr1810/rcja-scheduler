use crate::gui::AppState;
use crate::model::Volunteer;
use crate::scheduler::sanitize_name;
use eframe::egui::{self, Color32, RichText};
use std::collections::HashMap;

impl AppState {
    pub(super) fn draw_volunteers(&mut self, ui: &mut egui::Ui) {
        ui.heading("Volunteers Management");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.active_volunteer_sub_tab, super::VolunteerSubTab::Availability, "📅 Availability Grid");
            ui.selectable_value(&mut self.active_volunteer_sub_tab, super::VolunteerSubTab::WorkloadHeatmap, "🔥 Workload Heatmap");
        });
        ui.add_space(10.0);

        match self.active_volunteer_sub_tab {
            super::VolunteerSubTab::Availability => self.draw_volunteer_availability(ui),
            super::VolunteerSubTab::WorkloadHeatmap => self.draw_volunteer_heatmap(ui),
        }
    }

    fn draw_volunteer_availability(&mut self, ui: &mut egui::Ui) {
        let divisions_list: Vec<(String, String)> = self.config.divisions.iter()
            .map(|d| (d.id.clone(), d.name.clone()))
            .collect();

        let mut all_orgs: Vec<String> = self.config.teams.iter()
            .map(|t| t.organization.clone())
            .filter(|s| !s.is_empty())
            .collect();
        all_orgs.sort();
        all_orgs.dedup();

        // Add Volunteer
        egui::Frame::none()
            .fill(Color32::from_rgb(30, 37, 50))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Register Volunteer").strong().color(Color32::WHITE));
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut self.new_vol_name);
                        });
                    });

                    ui.add_space(5.0);
                    ui.label(RichText::new("Conflict Organizations:").strong().color(Color32::WHITE));
                    ui.label(RichText::new("Select any organizations this volunteer belongs to (e.g. their school or club)").size(11.0).color(Color32::from_rgb(156, 163, 175)));
                    
                    ui.horizontal_wrapped(|ui| {
                        for org in &all_orgs {
                            let mut is_conflict = self.new_vol_conflicts_list.contains(org);
                            if ui.checkbox(&mut is_conflict, org).changed() {
                                if is_conflict {
                                    self.new_vol_conflicts_list.push(org.clone());
                                } else {
                                    self.new_vol_conflicts_list.retain(|x| x != org);
                                }
                            }
                        }
                    });

                    ui.add_space(5.0);
                    ui.label("Qualified Divisions (Capabilities):");
                    ui.horizontal(|ui| {
                        for (div_id, div_name) in &divisions_list {
                            let mut checked = self.new_vol_caps.contains(div_id);
                            if ui.checkbox(&mut checked, div_name).changed() {
                                if checked {
                                    self.new_vol_caps.push(div_id.clone());
                                } else {
                                    self.new_vol_caps.retain(|x| x != div_id);
                                }
                            }
                        }
                        let mut has_interview = self.new_vol_caps.contains(&"Interview".to_string());
                        if ui.checkbox(&mut has_interview, "🎤 Interviews").changed() {
                            if has_interview {
                                self.new_vol_caps.push("Interview".to_string());
                            } else {
                                self.new_vol_caps.retain(|x| x != "Interview");
                            }
                        }
                    });

                    ui.add_space(8.0);
                    if ui.button(RichText::new("+ Register Volunteer").strong().color(Color32::WHITE)).clicked()
                        && !self.new_vol_name.trim().is_empty() {
                            let id = sanitize_name(&self.new_vol_name);

                            self.config.volunteers.push(Volunteer {
                                id,
                                name: self.new_vol_name.clone(),
                                availabilities: Vec::new(),
                                capabilities: if self.new_vol_caps.is_empty() { None } else { Some(self.new_vol_caps.clone()) },
                                conflict_organizations: self.new_vol_conflicts_list.clone(),
                                attendance_status: std::collections::HashMap::new(),
                            });

                            self.new_vol_name.clear();
                            self.new_vol_conflicts_list.clear();
                            self.new_vol_caps.clear();
                            self.clear_schedule();
                            self.update_diagnostics();
                            self.status_message = "Volunteer registered!".to_string();
                        }
                });
            });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Capability Mode:").strong());
            if ui.checkbox(&mut self.config.strict_capabilities, "Enforce strict qualifications (must match 'Qualified for' division)").changed() {
                self.clear_schedule();
                self.update_diagnostics();
            }
        });
        ui.add_space(8.0);

        ui.label(RichText::new("INTERACTIVE AVAILABILITY GRID").strong().color(Color32::from_rgb(156, 163, 175)));
        ui.label("Click cells to toggle volunteer availability for each time slot.");
        ui.add_space(5.0);

        if self.config.time_slots.is_empty() {
            ui.label(RichText::new("Please add time slots first!").italics().color(Color32::from_rgb(244, 63, 94)));
            return;
        }

        // Get sorted unique days present in the time slots
        let mut unique_days = Vec::new();
        for slot in &self.config.time_slots {
            if !unique_days.contains(&slot.day) {
                unique_days.push(slot.day.clone());
            }
        }

        if !unique_days.contains(&self.active_vol_day)
            && let Some(first_day) = unique_days.first() {
                self.active_vol_day = first_day.clone();
            }

        // Day selection tabs
        ui.horizontal(|ui| {
            ui.label(RichText::new("Show Availability for Day:").strong());
            for day in &unique_days {
                let is_selected = self.active_vol_day == *day;
                if ui.selectable_label(is_selected, day).clicked() {
                    self.active_vol_day = day.clone();
                }
            }
        });
        ui.add_space(8.0);

        let active_slots: Vec<crate::model::TimeSlot> = self.config.time_slots.iter()
            .filter(|s| s.day == self.active_vol_day)
            .cloned()
            .collect();

        let num_cols = active_slots.len() + 4;
        let mut to_delete = None;

        // Clone division mapping to avoid double borrows on config
        let division_names_map: HashMap<String, String> = divisions_list.into_iter().collect();

        egui::ScrollArea::horizontal()
            .id_source("volunteers_avail_scroll")
            .show(ui, |ui| {
                egui::Grid::new("vol_avail_grid").num_columns(num_cols).spacing(egui::vec2(10.0, 10.0)).show(ui, |ui| {
                ui.label(RichText::new("Volunteer").strong());
                ui.label(RichText::new("Qualified for").strong());
                ui.label(RichText::new("Conflicts").strong());

                for slot in &active_slots {
                    ui.label(RichText::new(format!("{}-{}", slot.start_time, slot.end_time)).size(10.0).strong());
                }
                ui.label(RichText::new("Actions").strong());
                ui.end_row();

                let mut config_changed = false;

                for (v_idx, vol) in self.config.volunteers.iter_mut().enumerate() {
                    // 1. Volunteer Name edit
                    ui.horizontal(|ui| {
                        if vol.availabilities.is_empty() {
                            ui.label(RichText::new("⚠").color(Color32::from_rgb(251, 191, 36)))
                                .on_hover_text("Volunteer has no available time slots.");
                        }
                        if ui.add_sized([180.0, 20.0], egui::TextEdit::singleline(&mut vol.name)).changed() {
                            config_changed = true;
                        }
                    });

                    // 2. Capabilities drop-down edit
                    let mut cap_changed = false;
                    let display_caps = match &vol.capabilities {
                        Some(caps) => {
                            if caps.is_empty() {
                                "None (No capability)".to_string()
                            } else {
                                let names: Vec<String> = caps
                                    .iter()
                                    .map(|d_id| {
                                        if d_id == "Interview" {
                                            "🎤 Interviews".to_string()
                                        } else {
                                            division_names_map
                                                .get(d_id).cloned()
                                                .unwrap_or_else(|| d_id.clone())
                                        }
                                    })
                                    .collect();
                                names.join(", ")
                            }
                        }
                        None => "Any Division".to_string(),
                    };

                    egui::ComboBox::from_id_source(format!("vol_cap_edit_{}", v_idx))
                        .selected_text(display_caps)
                        .show_ui(ui, |ui| {
                            let mut is_any = vol.capabilities.is_none();
                            if ui.checkbox(&mut is_any, "Any Division").changed() {
                                if is_any {
                                    vol.capabilities = None;
                                } else {
                                    vol.capabilities = Some(Vec::new());
                                }
                                cap_changed = true;
                            }

                            if let Some(ref mut caps) = vol.capabilities {
                                for (div_id, div_name) in &division_names_map {
                                    let mut has_div = caps.contains(div_id);
                                    if ui.checkbox(&mut has_div, div_name).changed() {
                                        if has_div {
                                            caps.push(div_id.clone());
                                        } else {
                                            caps.retain(|x| x != div_id);
                                        }
                                        cap_changed = true;
                                    }
                                }
                                let mut has_interview = caps.contains(&"Interview".to_string());
                                if ui.checkbox(&mut has_interview, "🎤 Interviews").changed() {
                                    if has_interview {
                                        caps.push("Interview".to_string());
                                    } else {
                                        caps.retain(|x| x != "Interview");
                                    }
                                    cap_changed = true;
                                }
                            }
                        });

                    if cap_changed {
                        config_changed = true;
                    }

                    // 3. Organization Conflicts (Checkboxes)
                    let mut org_changed = false;
                    egui::ComboBox::from_id_source(format!("vol_org_edit_{}", v_idx))
                        .selected_text(if vol.conflict_organizations.is_empty() {
                            "No Conflicts".to_string()
                        } else {
                            vol.conflict_organizations.join(", ")
                        })
                        .show_ui(ui, |ui| {
                            for org in &all_orgs {
                                let mut has_org = vol.conflict_organizations.contains(org);
                                if ui.checkbox(&mut has_org, org).changed() {
                                    if has_org {
                                        vol.conflict_organizations.push(org.clone());
                                    } else {
                                        vol.conflict_organizations.retain(|x| x != org);
                                    }
                                    org_changed = true;
                                }
                            }
                        });

                    if org_changed {
                        config_changed = true;
                    }

                    // 4. Availability checkboxes for active day slots
                    for slot in &active_slots {
                        let mut available = vol.availabilities.contains(&slot.id);
                        if ui.checkbox(&mut available, "").changed() {
                            if available {
                                vol.availabilities.push(slot.id.clone());
                            } else {
                                vol.availabilities.retain(|id| id != &slot.id);
                            }
                            config_changed = true;
                        }
                    }

                    // 5. Action buttons (All for active day, Clear active day, Delete)
                    ui.horizontal(|ui| {
                        if ui.button(format!("✔ All {}", self.active_vol_day)).clicked() {
                            for slot in &active_slots {
                                if !vol.availabilities.contains(&slot.id) {
                                    vol.availabilities.push(slot.id.clone());
                                }
                            }
                            config_changed = true;
                        }
                        if ui.button(format!("❌ Clear {}", self.active_vol_day)).clicked() {
                            let active_ids: std::collections::HashSet<&str> = active_slots.iter()
                                .map(|s| s.id.as_str())
                                .collect();
                            vol.availabilities.retain(|id| !active_ids.contains(id.as_str()));
                            config_changed = true;
                        }
                        if ui.button("🗑 Delete").clicked() {
                            to_delete = Some(v_idx);
                        }
                    });

                    ui.end_row();
                }

                if config_changed {
                    self.clear_schedule();
                    self.update_diagnostics();
                }
            });
        });

        if let Some(idx) = to_delete {
            self.config.volunteers.remove(idx);
            self.clear_schedule();
            self.update_diagnostics();
            self.status_message = "Volunteer deleted.".to_string();
        }
    }

    fn draw_volunteer_heatmap(&mut self, ui: &mut egui::Ui) {
        if self.schedule.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new("No Schedule Generated").size(16.0).color(Color32::from_rgb(107, 114, 128)).strong());
                ui.label("The workload heatmap requires a generated schedule to visualize assignments.");
            });
            return;
        }

        ui.label(RichText::new("VOLUNTEER WORKLOAD HEATMAP").strong().color(Color32::from_rgb(156, 163, 175)));
        ui.label("Visualization of shift density across the tournament.");
        ui.add_space(10.0);

        let sched = self.schedule.as_ref().unwrap();
        let slots = &self.config.time_slots;
        let volunteers = &self.config.volunteers;

        if volunteers.is_empty() || slots.is_empty() { return; }

        let mut unique_days = Vec::new();
        for slot in slots {
            if !unique_days.contains(&slot.day) {
                unique_days.push(slot.day.clone());
            }
        }

        for day in unique_days {
            ui.add_space(15.0);
            ui.label(RichText::new(&day).strong().size(14.0).color(Color32::from_rgb(129, 140, 248)));
            ui.add_space(5.0);

            let day_slots: Vec<_> = slots.iter().filter(|s| s.day == day).collect();
            if day_slots.is_empty() { continue; }

            egui::ScrollArea::horizontal()
                .id_source(format!("heatmap_scroll_{}", day))
                .show(ui, |ui| {
                    egui::Grid::new(format!("heatmap_grid_{}", day))
                        .spacing([2.0, 2.0])
                        .show(ui, |ui| {
                            // Header row
                            ui.label("");
                            for slot in &day_slots {
                                ui.label(RichText::new(&slot.start_time).size(9.0).strong());
                            }
                            ui.label(RichText::new("Total").size(9.0).strong());
                            ui.end_row();

                            for vol in volunteers {
                                ui.label(RichText::new(&vol.name).size(11.0));
                                
                                let mut day_total = 0;
                                for slot in &day_slots {
                                    let is_assigned = sched.assignments.iter().any(|a| a.time_slot_id == slot.id && a.volunteer_ids.contains(&vol.id));
                                    let is_available = vol.availabilities.contains(&slot.id);
                                    
                                    let (bg, text) = if is_assigned {
                                        day_total += 1;
                                        (Color32::from_rgb(79, 70, 229), Color32::WHITE)
                                    } else if is_available {
                                        (Color32::from_rgb(31, 41, 55), Color32::from_rgb(107, 114, 128))
                                    } else {
                                        (Color32::from_rgb(17, 24, 39), Color32::from_rgb(55, 65, 81))
                                    };

                                    let (rect, _resp) = ui.allocate_at_least(egui::vec2(35.0, 18.0), egui::Sense::hover());
                                    ui.painter().rect_filled(rect, 2.0, bg);
                                    if is_assigned {
                                        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "ON", egui::FontId::proportional(9.0), text);
                                    }
                                }
                                
                                ui.label(RichText::new(day_total.to_string()).strong().color(if day_total > 5 { Color32::from_rgb(248, 113, 113) } else { Color32::WHITE }));
                                ui.end_row();
                            }
                        });
                });
        }
    }
}
