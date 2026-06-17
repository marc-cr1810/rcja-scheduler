use crate::gui::AppState;
use crate::gui::AvailEditor;
use crate::gui::theme;
use crate::model::{TimeSlot, Volunteer, parse_time_minutes};
use crate::scheduler::sanitize_name;
use eframe::egui::{self, RichText};
use std::collections::{HashMap, HashSet};

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

        // (id, display name) for every field / interview table, used by the
        // per-volunteer field-lock selectors.
        let fields_list: Vec<(String, String)> = self.config.fields.iter()
            .map(|f| {
                let label = match f.kind {
                    crate::model::FieldKind::Interview => format!("🎤 {}", f.name),
                    crate::model::FieldKind::Competition => f.name.clone(),
                };
                (f.id.clone(), label)
            })
            .collect();

        let mut all_orgs: Vec<String> = self.config.teams.iter()
            .map(|t| t.organization.clone())
            .filter(|s| !s.is_empty())
            .collect();
        all_orgs.sort();
        all_orgs.dedup();

        // Add Volunteer
        egui::Frame::none()
            .fill(theme::card_bg())
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Register Volunteer").strong().color(theme::text()));
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label("Name:");
                            ui.text_edit_singleline(&mut self.new_vol_name);
                        });
                    });

                    ui.add_space(5.0);
                    ui.label(RichText::new("Conflict Organizations:").strong().color(theme::text()));
                    ui.label(RichText::new("Select any organizations this volunteer belongs to (e.g. their school or club)").size(11.0).color(theme::text_muted()));
                    
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

                    if !fields_list.is_empty() {
                        ui.add_space(5.0);
                        ui.label("Lock to Fields / Interview Tables (optional):");
                        ui.label(RichText::new("If any are selected, this volunteer will only be rostered on those fields.").size(11.0).color(theme::text_muted()));
                        ui.horizontal_wrapped(|ui| {
                            for (fid, fname) in &fields_list {
                                let mut locked = self.new_vol_locked_fields.contains(fid);
                                if ui.checkbox(&mut locked, fname).changed() {
                                    if locked {
                                        self.new_vol_locked_fields.push(fid.clone());
                                    } else {
                                        self.new_vol_locked_fields.retain(|x| x != fid);
                                    }
                                }
                            }
                        });
                    }

                    ui.add_space(8.0);
                    if ui.button(RichText::new("+ Register Volunteer").strong().color(theme::text())).clicked()
                        && !self.new_vol_name.trim().is_empty() {
                            let id = sanitize_name(&self.new_vol_name);

                            self.config.volunteers.push(Volunteer {
                                id,
                                name: self.new_vol_name.clone(),
                                availabilities: Vec::new(),
                                capabilities: if self.new_vol_caps.is_empty() { None } else { Some(self.new_vol_caps.clone()) },
                                conflict_organizations: self.new_vol_conflicts_list.clone(),
                                attendance_status: std::collections::HashMap::new(),
                                locked_field_ids: if self.new_vol_locked_fields.is_empty() { None } else { Some(self.new_vol_locked_fields.clone()) },
                            });

                            self.new_vol_name.clear();
                            self.new_vol_conflicts_list.clear();
                            self.new_vol_caps.clear();
                            self.new_vol_locked_fields.clear();
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

        ui.label(RichText::new("INTERACTIVE AVAILABILITY GRID").strong().color(theme::text_muted()));
        ui.label("Click cells to toggle volunteer availability for each time slot.");
        ui.add_space(5.0);

        if self.config.time_slots.is_empty() {
            if crate::gui::helpers::draw_empty_state(
                ui,
                "📅",
                "No time slots yet",
                "The availability grid needs time slots to fill in. Generate them first.",
                Some("Go to Time Slots"),
            ) {
                self.active_tab = super::Tab::TimeSlots;
            }
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

        let num_cols = 6;
        let active_day = self.active_vol_day.clone();
        let day_bounds = day_boundaries(&active_slots);
        let day_start = day_bounds.first().copied().unwrap_or(0);
        let day_end = day_bounds.last().copied().unwrap_or(day_start);
        let mut to_delete = None;
        let mut editor_to_open: Option<AvailEditor> = None;

        // Lookup map for display names; iteration for UI uses `divisions_list`
        // (a Vec) so checkbox order stays stable across frames. Iterating a
        // freshly-built HashMap each frame reshuffles entries (per-instance hash
        // seed) and makes the capability dropdown flicker.
        let division_names_map: HashMap<String, String> = divisions_list.iter().cloned().collect();

        egui::ScrollArea::horizontal()
            .id_source("volunteers_avail_scroll")
            .show(ui, |ui| {
                egui::Grid::new("vol_avail_grid").num_columns(num_cols).spacing(egui::vec2(10.0, 10.0)).striped(true).show(ui, |ui| {
                ui.label(RichText::new("Volunteer").strong());
                ui.label(RichText::new("Qualified for").strong());
                ui.label(RichText::new("Conflicts").strong());
                ui.label(RichText::new("Locked To").strong());
                ui.label(RichText::new("Availability").strong());
                ui.label(RichText::new("Actions").strong());
                ui.end_row();

                let mut config_changed = false;

                for (v_idx, vol) in self.config.volunteers.iter_mut().enumerate() {
                    // 1. Volunteer Name edit
                    ui.horizontal(|ui| {
                        if vol.availabilities.is_empty() {
                            ui.label(RichText::new("⚠").color(theme::warning()))
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
                                for (div_id, div_name) in &divisions_list {
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

                    // 3b. Field lock (Locked To) — restrict this volunteer to a set
                    // of fields / interview tables. Empty means no restriction.
                    let mut lock_changed = false;
                    let lock_label = match &vol.locked_field_ids {
                        Some(ids) if !ids.is_empty() => {
                            let names: Vec<String> = ids.iter()
                                .map(|fid| fields_list.iter().find(|(id, _)| id == fid)
                                    .map(|(_, n)| n.clone())
                                    .unwrap_or_else(|| fid.clone()))
                                .collect();
                            names.join(", ")
                        }
                        _ => "Any Field".to_string(),
                    };
                    egui::ComboBox::from_id_source(format!("vol_lock_edit_{}", v_idx))
                        .selected_text(lock_label)
                        .show_ui(ui, |ui| {
                            if fields_list.is_empty() {
                                ui.label("No fields defined");
                            }
                            for (fid, fname) in &fields_list {
                                let mut locked = vol.locked_field_ids.as_ref().is_some_and(|ids| ids.contains(fid));
                                if ui.checkbox(&mut locked, fname).changed() {
                                    let ids = vol.locked_field_ids.get_or_insert_with(Vec::new);
                                    if locked {
                                        ids.push(fid.clone());
                                    } else {
                                        ids.retain(|x| x != fid);
                                    }
                                    if ids.is_empty() {
                                        vol.locked_field_ids = None;
                                    }
                                    lock_changed = true;
                                }
                            }
                        });
                    if lock_changed {
                        config_changed = true;
                    }

                    // 4. Availability summary (timeline bar + ranges + edit).
                    // The per-slot checkbox grid was replaced by a range editor:
                    // overlapping, varying-duration slots made one checkbox per
                    // slot unmanageable. Ranges are derived from the stored slot
                    // IDs for display and converted back on apply.
                    ui.horizontal(|ui| {
                        let ranges = covered_ranges(vol, &active_slots);
                        let all_day = !active_slots.is_empty()
                            && active_slots.iter().all(|s| vol.availabilities.contains(&s.id));
                        draw_timeline(ui, &ranges, day_start, day_end);

                        let summary = if active_slots.is_empty() {
                            "—".to_string()
                        } else if ranges.is_empty() {
                            "None".to_string()
                        } else if all_day {
                            "All day".to_string()
                        } else {
                            ranges.iter()
                                .map(|(s, e)| format!("{}–{}", fmt_minutes(*s), fmt_minutes(*e)))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let color = if ranges.is_empty() { theme::warning() } else { theme::text() };
                        ui.label(RichText::new(summary).size(11.0).color(color));

                        if ui.button("✎ Edit").clicked() {
                            let ed_ranges = ranges.iter()
                                .map(|(s, e)| (fmt_minutes(*s), fmt_minutes(*e)))
                                .collect();
                            editor_to_open = Some(AvailEditor {
                                vol_idx: v_idx,
                                day: active_day.clone(),
                                ranges: ed_ranges,
                                all_day,
                            });
                        }
                    });

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

        if editor_to_open.is_some() {
            self.vol_avail_editor = editor_to_open;
        }
        self.draw_avail_editor(ui);
    }

    /// Modal-ish popup that edits one volunteer's availability for one day as a
    /// list of time ranges (or "all day"), writing the result back as slot IDs.
    fn draw_avail_editor(&mut self, ui: &mut egui::Ui) {
        let Some(mut ed) = self.vol_avail_editor.take() else { return };

        let day_slots: Vec<TimeSlot> = self.config.time_slots.iter()
            .filter(|s| s.day == ed.day)
            .cloned()
            .collect();
        let boundaries = day_boundaries(&day_slots);
        let vol_name = self.config.volunteers.get(ed.vol_idx)
            .map(|v| v.name.clone())
            .unwrap_or_default();

        let mut window_open = true;
        let mut apply = false;
        let mut cancel = false;

        egui::Window::new(format!("Availability — {} ({})", vol_name, ed.day))
            .collapsible(false)
            .resizable(false)
            .open(&mut window_open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.checkbox(&mut ed.all_day, "Available all day");
                ui.add_space(8.0);

                // Flag reversed/overlapping ranges so we can highlight them and
                // block Apply. The end picker is also constrained to be after the
                // start, so within a single range it can't be reversed at all.
                let (range_flags, range_error) = if ed.all_day {
                    (Vec::new(), None)
                } else {
                    validate_ranges(&ed.ranges)
                };

                if !ed.all_day {
                    if ed.ranges.is_empty() {
                        ui.label(RichText::new("No ranges — volunteer is unavailable this day.")
                            .size(11.0).color(theme::text_muted()));
                    }

                    // For each range, the end of the previous range and the start
                    // of the next (in time order) bound what its pickers may
                    // offer, so overlapping times are never selectable.
                    let neighbours = range_neighbour_bounds(&ed.ranges);

                    let mut remove = None;
                    for (i, (start, end)) in ed.ranges.iter_mut().enumerate() {
                        let start_min = parse_time_minutes(start);
                        let end_min = parse_time_minutes(end);
                        let (prev_end, next_start) = neighbours[i];
                        // Top-align the row: egui's default centre alignment lets
                        // items drift downward across a horizontal row (the end
                        // picker ended up lower than the start). With equal-height
                        // items, top alignment keeps the comboboxes on one line.
                        ui.horizontal_top(|ui| {
                            // Match the combobox height (text + button padding,
                            // floored at interact_size) so the labels' centred
                            // text lines up with the text inside the comboboxes.
                            let row_h = ui.spacing().interact_size.y.max(
                                ui.text_style_height(&egui::TextStyle::Button)
                                    + 2.0 * ui.spacing().button_padding.y,
                            );

                            let label = RichText::new(format!("Range {}:", i + 1));
                            let label = if range_flags.get(i).copied().unwrap_or(false) {
                                label.color(theme::danger())
                            } else {
                                label
                            };
                            ui.add_sized([56.0, row_h], egui::Label::new(label).selectable(false));
                            // start: at/after the previous range, before its own end.
                            boundary_combo(ui, format!("avail_start_{i}"), start, &boundaries, |b| {
                                prev_end.is_none_or(|m| b >= m) && end_min.is_none_or(|m| b < m)
                            });
                            ui.add_sized([12.0, row_h], egui::Label::new("–").selectable(false));
                            // end: after its own start, at/before the next range.
                            boundary_combo(ui, format!("avail_end_{i}"), end, &boundaries, |b| {
                                start_min.is_none_or(|m| b > m) && next_start.is_none_or(|m| b <= m)
                            });
                            if ui.button("✕").on_hover_text("Remove range").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        ed.ranges.remove(i);
                    }
                    ui.add_space(4.0);
                    if ui.button("+ Add range").clicked() {
                        // Default to the next free slot after the existing ranges,
                        // running to the end of the day — so it never overlaps and
                        // the start picker has room to narrow it down.
                        let after = ed.ranges.iter()
                            .filter_map(|(_, e)| parse_time_minutes(e))
                            .max();
                        let s = match after {
                            Some(end) => boundaries.iter().copied()
                                .find(|b| *b >= end)
                                .unwrap_or_else(|| boundaries.last().copied().unwrap_or(0)),
                            None => boundaries.first().copied().unwrap_or(0),
                        };
                        let e = boundaries.last().copied().filter(|e| *e > s)
                            .or_else(|| boundaries.iter().copied().find(|b| *b > s))
                            .unwrap_or(s);
                        ed.ranges.push((fmt_minutes(s), fmt_minutes(e)));
                    }
                }

                if let Some(err) = &range_error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("⚠ {err}")).size(11.0).color(theme::danger()));
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(range_error.is_none(), |ui| {
                        if ui.button(RichText::new("Apply").strong().color(theme::text())).clicked() {
                            apply = true;
                        }
                    });
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if apply {
            if let Some(vol) = self.config.volunteers.get_mut(ed.vol_idx) {
                let day_ids: HashSet<&str> = day_slots.iter().map(|s| s.id.as_str()).collect();
                vol.availabilities.retain(|id| !day_ids.contains(id.as_str()));

                let new_ids: Vec<String> = if ed.all_day {
                    day_slots.iter().map(|s| s.id.clone()).collect()
                } else {
                    let ranges_min: Vec<(u32, u32)> = ed.ranges.iter()
                        .filter_map(|(s, e)| {
                            let s = parse_time_minutes(s)?;
                            let e = parse_time_minutes(e)?;
                            (e > s).then_some((s, e))
                        })
                        .collect();
                    slots_in_ranges(&day_slots, &ranges_min)
                };
                vol.availabilities.extend(new_ids);
            }
            self.clear_schedule();
            self.update_diagnostics();
        }

        // Keep editing only while the window is open and not dismissed.
        if window_open && !apply && !cancel {
            self.vol_avail_editor = Some(ed);
        }
    }

    fn draw_volunteer_heatmap(&mut self, ui: &mut egui::Ui) {
        if self.schedule.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new("No Schedule Generated").size(16.0).color(theme::text_faint()).strong());
                ui.label("The workload heatmap requires a generated schedule to visualize assignments.");
            });
            return;
        }

        ui.label(RichText::new("VOLUNTEER WORKLOAD TIMELINE").strong().color(theme::text_muted()));
        ui.label("Solid blocks show when each volunteer is actually working; gaps (lunch / idle) are shown to scale.");
        ui.add_space(10.0);

        let sched = self.schedule.as_ref().unwrap();
        let slots = &self.config.time_slots;
        let volunteers = &self.config.volunteers;

        if volunteers.is_empty() || slots.is_empty() { return; }

        // Fixed widths so the name column, timeline bars and totals line up into
        // readable columns across every row.
        const NAME_W: f32 = 150.0;
        const TOTAL_W: f32 = 36.0;
        const RANGES_W: f32 = 230.0;

        let mut unique_days = Vec::new();
        for slot in slots {
            if !unique_days.contains(&slot.day) {
                unique_days.push(slot.day.clone());
            }
        }

        for day in unique_days {
            ui.add_space(15.0);
            ui.label(RichText::new(&day).strong().size(14.0).color(theme::accent()));
            ui.add_space(5.0);

            let day_slots: Vec<&TimeSlot> = slots.iter().filter(|s| s.day == day).collect();
            if day_slots.is_empty() { continue; }

            // Day span from the actual schedulable slots, so the bars cover only
            // the working hours of the day.
            let day_start = day_slots.iter().map(|s| slot_span(s).0).min().unwrap_or(0);
            let day_end = day_slots.iter().map(|s| slot_span(s).1).max().unwrap_or(day_start);

            // The bar fills the space left after the fixed columns.
            let bar_w = (ui.available_width() - NAME_W - TOTAL_W - RANGES_W - 24.0).clamp(220.0, 900.0);

            // Hour-tick axis aligned with the bars below.
            ui.horizontal(|ui| {
                ui.add_space(NAME_W);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, 14.0), egui::Sense::hover());
                let painter = ui.painter();
                let span = day_end.saturating_sub(day_start).max(1) as f32;
                let mut h = (day_start / 60 + 1) * 60;
                while h < day_end {
                    let x = rect.left() + rect.width() * ((h - day_start) as f32 / span);
                    painter.text(
                        egui::pos2(x, rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        fmt_minutes(h),
                        egui::FontId::proportional(9.0),
                        theme::text_muted(),
                    );
                    h += 60;
                }
            });

            for vol in volunteers {
                // Spans of the slots this volunteer is actually assigned to today.
                let mut spans: Vec<(u32, u32)> = day_slots.iter()
                    .filter(|s| sched.assignments.iter()
                        .any(|a| a.time_slot_id == s.id && a.volunteer_ids.contains(&vol.id)))
                    .map(|s| slot_span(s))
                    .collect();
                let day_total = spans.len();
                spans.sort_by_key(|(s, _)| *s);
                // Merge runs separated by no more than a short changeover gap into
                // one block, so back-to-back shifts read as a continuous period
                // while genuine idle stretches and lunch stay visible as gaps.
                let blocks = merge_work_blocks(&spans, 5);

                ui.horizontal(|ui| {
                    ui.add_sized([NAME_W, 22.0], egui::Label::new(RichText::new(&vol.name).size(11.0)).truncate(true));
                    draw_work_timeline(ui, &blocks, day_start, day_end, bar_w);

                    let color = if day_total == 0 { theme::text_faint() } else { theme::text() };
                    ui.add_sized([TOTAL_W, 22.0], egui::Label::new(RichText::new(day_total.to_string()).strong().color(color)));

                    let summary = if blocks.is_empty() {
                        "—".to_string()
                    } else {
                        blocks.iter()
                            .map(|(s, e)| format!("{}–{}", fmt_minutes(*s), fmt_minutes(*e)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    ui.add_sized([RANGES_W, 22.0], egui::Label::new(RichText::new(summary).size(10.0).color(theme::text_muted())).truncate(true));
                });
            }
        }
    }
}

/// Merges sorted assigned spans into working blocks, bridging gaps of at most
/// `bridge` minutes (a changeover between back-to-back shifts) so consecutive
/// shifts render as one block while real idle gaps remain visible.
fn merge_work_blocks(spans: &[(u32, u32)], bridge: u32) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for &(s, e) in spans {
        match out.last_mut() {
            Some(last) if s <= last.1 + bridge => last.1 = last.1.max(e),
            _ => out.push((s, e)),
        }
    }
    out
}

/// Horizontal bar of working blocks across the day span, with faint hour
/// gridlines. Idle time shows as the bar's background between blocks.
fn draw_work_timeline(ui: &mut egui::Ui, blocks: &[(u32, u32)], day_start: u32, day_end: u32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, theme::surface());

    let span = day_end.saturating_sub(day_start).max(1) as f32;

    let mut h = (day_start / 60 + 1) * 60;
    while h < day_end {
        let x = rect.left() + rect.width() * ((h - day_start) as f32 / span);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, theme::border()),
        );
        h += 60;
    }

    for (s, e) in blocks {
        let x0 = rect.left() + rect.width() * (s.saturating_sub(day_start) as f32 / span);
        let x1 = rect.left() + rect.width() * (e.saturating_sub(day_start) as f32 / span);
        let seg = egui::Rect::from_min_max(
            egui::pos2(x0, rect.top() + 2.0),
            egui::pos2(x1.max(x0 + 2.0), rect.bottom() - 2.0),
        );
        painter.rect_filled(seg, 2.0, theme::accent_strong());
    }
    painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::border()));
}

// ── Availability range helpers ────────────────────────────────────────────--

/// Start/end minutes of a slot.
fn slot_span(s: &TimeSlot) -> (u32, u32) {
    let st = parse_time_minutes(&s.start_time).unwrap_or(0);
    let en = parse_time_minutes(&s.end_time).unwrap_or(st);
    (st, en)
}

/// Format minutes-since-midnight as "HH:MM".
fn fmt_minutes(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Distinct, sorted minute boundaries (every start and end time) across a
/// day's slots — the snap points offered in the range pickers.
fn day_boundaries(slots: &[TimeSlot]) -> Vec<u32> {
    let mut set: Vec<u32> = Vec::new();
    for s in slots {
        let (st, en) = slot_span(s);
        for m in [st, en] {
            if !set.contains(&m) {
                set.push(m);
            }
        }
    }
    set.sort_unstable();
    set
}

/// True if any slot's span intersects the open interval `(g0, g1)`.
fn gap_has_slot(slots: &[TimeSlot], g0: u32, g1: u32) -> bool {
    slots.iter().any(|s| {
        let (st, en) = slot_span(s);
        st < g1 && en > g0
    })
}

/// Merge the intervals of a volunteer's available slots into contiguous covered
/// ranges, sorted by start. Used to summarise stored slot IDs as time ranges.
///
/// Adjacent clusters are bridged across gaps that contain no slot at all (the
/// short buffers between staggered slots, or a slot-free lunch break): there is
/// nothing schedulable there, so it would be noise to render it as a boundary.
/// A gap that *does* contain slots the volunteer isn't available for stays a
/// real break.
fn covered_ranges(vol: &Volunteer, slots: &[TimeSlot]) -> Vec<(u32, u32)> {
    let mut spans: Vec<(u32, u32)> = slots.iter()
        .filter(|s| vol.availabilities.contains(&s.id))
        .map(slot_span)
        .collect();
    spans.sort_by_key(|(s, _)| *s);

    let mut merged: Vec<(u32, u32)> = Vec::new();
    for (s, e) in spans {
        match merged.last_mut() {
            Some(last) if s <= last.1 || !gap_has_slot(slots, last.1, s) => {
                last.1 = last.1.max(e)
            }
            _ => merged.push((s, e)),
        }
    }
    merged
}

/// IDs of the day's slots that fall fully within any of the given ranges. A
/// volunteer must be present for the whole slot, so a slot only counts when it
/// is entirely contained.
fn slots_in_ranges(slots: &[TimeSlot], ranges: &[(u32, u32)]) -> Vec<String> {
    slots.iter()
        .filter(|s| {
            let (st, en) = slot_span(s);
            ranges.iter().any(|(rs, re)| *rs <= st && en <= *re)
        })
        .map(|s| s.id.clone())
        .collect()
}

/// Horizontal bar visualising covered ranges across the day span.
fn draw_timeline(ui: &mut egui::Ui, ranges: &[(u32, u32)], day_start: u32, day_end: u32) {
    let (rect, _) = ui.allocate_at_least(egui::vec2(110.0, 12.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, theme::surface());

    let span = day_end.saturating_sub(day_start).max(1) as f32;
    for (s, e) in ranges {
        let x0 = rect.left() + rect.width() * (s.saturating_sub(day_start) as f32 / span);
        let x1 = rect.left() + rect.width() * (e.saturating_sub(day_start) as f32 / span);
        let seg = egui::Rect::from_min_max(
            egui::pos2(x0, rect.top()),
            egui::pos2(x1, rect.bottom()),
        );
        painter.rect_filled(seg, 2.0, theme::accent());
    }
    painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, theme::border()));
}

/// Dropdown of day boundaries for picking a range start/end time. Only
/// boundaries for which `allowed` returns true are offered, so callers can
/// exclude times that would reverse the range or overlap a neighbouring one.
fn boundary_combo(
    ui: &mut egui::Ui,
    id: String,
    value: &mut String,
    boundaries: &[u32],
    allowed: impl Fn(u32) -> bool,
) {
    egui::ComboBox::from_id_source(id)
        .width(80.0)
        .selected_text(value.clone())
        .show_ui(ui, |ui| {
            for b in boundaries {
                if !allowed(*b) {
                    continue;
                }
                let label = fmt_minutes(*b);
                ui.selectable_value(value, label.clone(), label);
            }
        });
}

/// For each range (by its index in the list), the end time of the range
/// immediately before it and the start time of the one immediately after it,
/// ordered by start time. These bound the times the range's pickers may offer
/// so two ranges can never be made to overlap. Unparseable ranges contribute no
/// bound (`None`).
fn range_neighbour_bounds(ranges: &[(String, String)]) -> Vec<(Option<u32>, Option<u32>)> {
    let parsed: Vec<Option<(u32, u32)>> = ranges.iter()
        .map(|(s, e)| Some((parse_time_minutes(s)?, parse_time_minutes(e)?)))
        .collect();

    let mut order: Vec<usize> = (0..ranges.len()).collect();
    order.sort_by_key(|&i| parsed[i].map_or(u32::MAX, |(s, _)| s));

    let mut bounds = vec![(None, None); ranges.len()];
    for pos in 0..order.len() {
        let prev_end = pos.checked_sub(1).and_then(|p| parsed[order[p]].map(|(_, e)| e));
        let next_start = order.get(pos + 1).and_then(|&n| parsed[n].map(|(s, _)| s));
        bounds[order[pos]] = (prev_end, next_start);
    }
    bounds
}

/// Validate the editor's ranges. Returns a per-range "is problematic" flag (for
/// highlighting) and, if anything is wrong, a single message to show the user.
/// Empty / unparseable ranges and overlaps are rejected; an empty list is fine
/// (it just means the volunteer is unavailable that day).
fn validate_ranges(ranges: &[(String, String)]) -> (Vec<bool>, Option<String>) {
    let mut flags = vec![false; ranges.len()];
    let parsed: Vec<Option<(u32, u32)>> = ranges.iter()
        .map(|(s, e)| Some((parse_time_minutes(s)?, parse_time_minutes(e)?)))
        .collect();

    let mut error = None;

    // Each range must end strictly after it starts.
    for (i, p) in parsed.iter().enumerate() {
        if !matches!(p, Some((s, e)) if e > s) {
            flags[i] = true;
            error = Some("Each range must end after it starts.".to_string());
        }
    }

    // No two (otherwise-valid) ranges may overlap.
    let mut order: Vec<usize> = (0..ranges.len()).filter(|&i| !flags[i]).collect();
    order.sort_by_key(|&i| parsed[i].expect("filtered to valid").0);
    for pair in order.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if parsed[b].unwrap().0 < parsed[a].unwrap().1 {
            flags[a] = true;
            flags[b] = true;
            error = Some("Ranges must not overlap.".to_string());
        }
    }

    (flags, error)
}
