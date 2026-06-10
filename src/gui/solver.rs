use crate::gui::{AppState, ScheduleViewTab, SolverMessage, Tab};
use crate::model::{FairnessMode, SchedulingMode, SpecialistMode, FieldKind};
use crate::validator::DiagnosticSeverity;
use crate::gui::helpers::{draw_schedule_cell, get_competition_colors, parse_time_to_minutes};
use crate::scheduler::{RoundRow, solve_schedule, AssignmentConflict};
use eframe::egui::{self, Color32, RichText, Stroke};

impl AppState {
    pub(super) fn draw_solver(&mut self, ui: &mut egui::Ui) {
        ui.heading("Schedule Solver & Grid Viewer");
        ui.add_space(10.0);

        let is_solving = self.solver_rx.is_some();
        let config_diagnostics = crate::validator::validate_config(&self.config);
        let config_error_count = config_diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Error).count();
        
        if config_error_count > 0 {
            egui::Frame::none()
                .fill(Color32::from_rgb(127, 29, 29))
                .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                .rounding(8.0)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("❌ CONFIGURATION ERRORS:").strong().color(Color32::WHITE));
                        ui.label(RichText::new(format!("There are {} critical configuration errors that must be fixed before a schedule can be generated.", config_error_count)).color(Color32::from_rgb(254, 202, 202)));
                        if ui.button("View Errors").clicked() {
                            self.active_tab = Tab::Dashboard;
                        }
                    });
                });
            ui.add_space(15.0);
        }

        egui::Frame::none()
            .fill(Color32::from_rgb(30, 37, 50))
            .rounding(8.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label("Solver Iterations:");
                        ui.add(egui::DragValue::new(&mut self.solver_iterations).clamp_range(1000..=50000).speed(500));
                    });
                    ui.vertical(|ui| {
                        ui.label("Solver Restarts:");
                        ui.add(egui::DragValue::new(&mut self.solver_restarts).clamp_range(1..=20));
                    });

                    ui.add_space(20.0);

                    let can_solve = !is_solving && config_error_count == 0;
                    let solve_button_text = if is_solving { "⏳ Solving..." } else { "⚙ Generate Schedule" };
                    let solve_button = egui::Button::new(RichText::new(solve_button_text).strong().color(Color32::WHITE))
                        .fill(if can_solve { Color32::from_rgb(79, 70, 229) } else { Color32::from_rgb(55, 65, 81) })
                        .rounding(6.0)
                        .min_size(egui::vec2(150.0, 35.0));

                    if ui.add_enabled(can_solve, solve_button).clicked() {
                        self.solve_and_schedule();
                    }

                    if is_solving {
                        ui.add_space(8.0);
                        let stop_button = egui::Button::new(RichText::new("⏹ Stop Solving").strong().color(Color32::WHITE))
                            .fill(Color32::from_rgb(185, 28, 28)) // Red-700
                            .rounding(6.0)
                            .min_size(egui::vec2(120.0, 35.0));

                        if ui.add(stop_button).clicked()
                            && let Some(ref flag) = self.solver_cancel_flag {
                                flag.store(true, std::sync::atomic::Ordering::Relaxed);
                                self.solve_message = "Stopping solver...".to_string();
                            }
                    }
                });

                if is_solving {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.add(egui::ProgressBar::new(self.solve_progress)
                            .show_percentage()
                            .animate(true)
                            .text(RichText::new(&self.solve_message).strong().color(Color32::WHITE))
                            .desired_width(ui.available_width() - 20.0));
                    });
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                // Fairness Mode selector
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚖ Volunteer Fairness Mode:").strong().color(Color32::from_rgb(229, 231, 235)));
                    ui.add_space(8.0);

                    let modes: &[(FairnessMode, &str, &str, Color32, Color32)] = &[
                        (
                            FairnessMode::Off,
                            "Off",
                            "Pure random volunteer selection (original behaviour). No fairness adjustment.",
                            Color32::from_rgb(55, 65, 81),
                            Color32::from_rgb(107, 114, 128),
                        ),
                        (
                            FairnessMode::Balanced,
                            "⚖ Balanced",
                            "Weighted-random selection biased toward under-utilised volunteers.\nVolunteers with fewer shifts relative to their availability are preferred.\nRecommended for most tournaments.",
                            Color32::from_rgb(6, 78, 59),
                            Color32::from_rgb(52, 211, 153),
                        ),
                        (
                            FairnessMode::Strict,
                            "⚡ Strict",
                            "Always assigns the least-utilised qualified volunteers first.\nStrongest fairness guarantee — best when volunteeer workloads must be as equal as possible.",
                            Color32::from_rgb(67, 20, 7),
                            Color32::from_rgb(251, 146, 60),
                        ),
                    ];

                    for (mode, label, tooltip, bg_inactive, text_inactive) in modes {
                        let is_active = self.solver_fairness_mode == *mode;
                        let (bg, text_col) = if is_active {
                            match mode {
                                FairnessMode::Off => (Color32::from_rgb(55, 65, 81), Color32::WHITE),
                                FairnessMode::Balanced => (Color32::from_rgb(16, 185, 129), Color32::WHITE),
                                FairnessMode::Strict => (Color32::from_rgb(234, 88, 12), Color32::WHITE),
                            }
                        } else {
                            (*bg_inactive, *text_inactive)
                        };

                        let btn = egui::Button::new(RichText::new(*label).strong().color(text_col))
                            .fill(bg)
                            .rounding(6.0)
                            .min_size(egui::vec2(95.0, 28.0));

                        let resp = ui.add(btn).on_hover_text(*tooltip);
                        if resp.clicked() {
                            self.solver_fairness_mode = *mode;
                            self.config.fairness_mode = *mode;
                        }
                        ui.add_space(4.0);
                    }
                });

                ui.add_space(8.0);

                // Volunteer consecutive shift penalty
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🔁 Volunteer Rest Penalty:").strong().color(Color32::from_rgb(229, 231, 235)));
                    ui.add_space(8.0);

                    let is_on = self.solver_vol_consecutive_weight > 0.0;
                    let toggle_label = if is_on { "● On" } else { "○ Off" };
                    let toggle_color = if is_on { Color32::from_rgb(16, 185, 129) } else { Color32::from_rgb(107, 114, 128) };
                    let toggle_bg    = if is_on { Color32::from_rgb(6, 78, 59) } else { Color32::from_rgb(31, 41, 55) };

                    let toggle_btn = egui::Button::new(RichText::new(toggle_label).strong().color(toggle_color))
                        .fill(toggle_bg)
                        .rounding(6.0)
                        .min_size(egui::vec2(55.0, 26.0));

                    if ui.add(toggle_btn)
                        .on_hover_text("Penalise volunteers assigned to back-to-back time slots (no rest between duties).")
                        .clicked()
                    {
                        if is_on {
                            self.solver_vol_consecutive_weight = 0.0;
                        } else {
                            self.solver_vol_consecutive_weight = 1.0;
                        }
                    }

                    if self.solver_vol_consecutive_weight > 0.0 {
                        ui.add_space(6.0);
                        ui.label(RichText::new("Weight:").color(Color32::from_rgb(156, 163, 175)));
                        ui.add(
                            egui::DragValue::new(&mut self.solver_vol_consecutive_weight)
                                .clamp_range(0.1f64..=5.0)
                                .speed(0.1)
                                .fixed_decimals(1),
                        ).on_hover_text("How heavily to penalise consecutive volunteer shifts.\n0.5 = mild nudge, 1.0 = same as team back-to-back, 3.0+ = strongly avoid.");
                    }
                });

                ui.add_space(8.0);

                // Volunteer Specialist Mode
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🎯 Volunteer Specialist Mode:").strong().color(Color32::from_rgb(229, 231, 235)));
                    ui.add_space(8.0);

                    let modes: &[(SpecialistMode, &str, &str, Color32, Color32)] = &[
                        (
                            SpecialistMode::Off,
                            "Off",
                            "Volunteers can be assigned to any division they are qualified for.",
                            Color32::from_rgb(55, 65, 81),
                            Color32::from_rgb(107, 114, 128),
                        ),
                        (
                            SpecialistMode::Balanced,
                            "🎯 Focused",
                            "Try to keep volunteers within a single division (e.g. don't swap someone between different Soccer divisions).",
                            Color32::from_rgb(30, 58, 138), // Dark blue
                            Color32::from_rgb(96, 165, 250), // Light blue
                        ),
                        (
                            SpecialistMode::Strict,
                            "🏅 Specialist",
                            "Strongest preference to keep volunteers in the same division for the whole tournament.",
                            Color32::from_rgb(88, 28, 135), // Dark purple
                            Color32::from_rgb(192, 132, 252), // Light purple
                        ),
                    ];

                    for (mode, label, tooltip, bg_inactive, text_inactive) in modes {
                        let is_active = self.solver_vol_specialist_mode == *mode;
                        let (bg, text_col) = if is_active {
                            match mode {
                                SpecialistMode::Off => (Color32::from_rgb(55, 65, 81), Color32::WHITE),
                                SpecialistMode::Balanced => (Color32::from_rgb(37, 99, 235), Color32::WHITE),
                                SpecialistMode::Strict => (Color32::from_rgb(126, 34, 206), Color32::WHITE),
                            }
                        } else {
                            (*bg_inactive, *text_inactive)
                        };

                        let btn = egui::Button::new(RichText::new(*label).strong().color(text_col))
                            .fill(bg)
                            .rounding(6.0)
                            .min_size(egui::vec2(95.0, 28.0));

                        let resp = ui.add(btn).on_hover_text(*tooltip);
                        if resp.clicked() {
                            self.solver_vol_specialist_mode = *mode;
                        }
                        ui.add_space(4.0);
                    }
                });

                ui.add_space(8.0);

                // Volunteer Travel & Shift Cap
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📍 Volunteer Travel Penalty:").strong().color(Color32::from_rgb(229, 231, 235)));
                    ui.add(
                        egui::DragValue::new(&mut self.solver_vol_travel_weight)
                            .clamp_range(0.0f64..=3.0)
                            .speed(0.1)
                            .fixed_decimals(1),
                    ).on_hover_text("Penalises moving a volunteer between different fields/locations for back-to-back shifts.");

                    ui.add_space(20.0);

                    ui.label(RichText::new("🕒 Max Shifts/Day:").strong().color(Color32::from_rgb(229, 231, 235)));
                    ui.add(
                        egui::DragValue::new(&mut self.solver_vol_daily_shift_cap)
                            .clamp_range(0..=20)
                    ).on_hover_text("Hard limit on the number of shifts any single volunteer can be assigned in one day.\n0 = No limit.");
                });

                ui.add_space(8.0);

                // Advanced Solver Weights Collapsing Header
                egui::CollapsingHeader::new(
                    RichText::new("⚙ Advanced Solver Settings")
                        .strong()
                        .color(Color32::from_rgb(156, 163, 175))
                )
                .id_source("advanced_solver_weights")
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new("Fine-tune the optimization priorities. Higher values force the solver to avoid these conditions more strictly.")
                            .small()
                            .italics()
                            .color(Color32::from_rgb(156, 163, 175))
                    );
                    ui.add_space(6.0);

                    ui.group(|ui| {
                        egui::Grid::new("advanced_weights_grid")
                            .num_columns(2)
                            .spacing([15.0, 8.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("Team Back-to-Back Rest:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_team_back_to_back_weight, 0.0..=3.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Penalises scheduling a team for consecutive matches on the same day.\n0.0 = Ignore, 1.0 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Field/Arena Variety:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_field_variety_weight, 0.0..=3.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Encourages the solver to vary the fields/arenas teams play on.\n0.0 = Ignore, 0.5 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Field Workload Balance:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_field_balance_weight, 0.0..=5.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Penalises uneven activity load distribution across different fields.\n0.0 = Ignore, 1.5 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Volunteer Capability Match:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_vol_capability_weight, 0.0..=3.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Soft penalty for assigning a volunteer to a division outside their capability list.\n0.0 = Ignore, 0.5 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Interview Prioritisation:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_interview_late_weight, 0.0..=5.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Penalises scheduling technical interviews late in the day.\n0.0 = Ignore, 0.5 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Interview-to-Match Buffer:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_interview_match_gap_weight, 0.0..=5.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Penalises scheduling a match too close to a team's interview.\n0.0 = Ignore, 1.0 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Team Wait-Time Mode:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_team_wait_time_weight, 0.0..=3.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Penalises long gaps between a team's games on the same day.\n0.0 = Ignore, 0.3 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Arena Variety Strictness:").color(Color32::from_rgb(209, 213, 219)));
                                ui.checkbox(&mut self.solver_field_variety_strict, "")
                                    .on_hover_text("If enabled, playing on the same field twice becomes a HARD conflict.");
                                ui.end_row();

                                ui.label(RichText::new("Round Sequencing Priority:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_round_order_weight, 0.0..=10.0)
                                        .step_by(0.5)
                                        .show_value(true)
                                )
                                .on_hover_text("Ensures all Round 1 matches happen before Round 2 starts.\n0.0 = Ignore, 5.0 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Peak Period Balancing:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_peak_period_weight, 0.0..=2.0)
                                        .step_by(0.1)
                                        .show_value(true)
                                )
                                .on_hover_text("Encourages spreading games evenly across the day to avoid crowd surges.\n0.0 = Ignore, 0.1 = Default.");
                                ui.end_row();

                                ui.label(RichText::new("Finals Priority Boost:").color(Color32::from_rgb(209, 213, 219)));
                                ui.add(
                                    egui::Slider::new(&mut self.solver_finals_priority_multiplier, 1.0..=10.0)
                                        .step_by(0.5)
                                        .show_value(true)
                                )
                                .on_hover_text("Multiplier for penalties/conflicts involving final matches.\n1.0 = Same as qualifiers, 2.0 = Default.");
                                ui.end_row();
                            });
                    });
                });

                if !is_solving && !self.solve_message.is_empty() {
                    ui.add_space(8.0);
                    let cost_color = if self.solve_status.contains("No Conflicts") {
                        Color32::from_rgb(52, 211, 153)
                    } else {
                        Color32::from_rgb(248, 113, 113)
                    };
                    ui.label(RichText::new(&self.solve_message).strong().color(cost_color));

                    if self.schedule.is_some() {
                        let conflicts = &self.schedule_conflicts;
                        if !conflicts.is_empty() {
                            ui.add_space(5.0);
                            egui::CollapsingHeader::new(
                                RichText::new(format!("⚠ Detailed Conflict Diagnostics ({})", conflicts.len()))
                                    .strong()
                                    .color(Color32::from_rgb(248, 113, 113))
                            )
                            .id_source("detailed_conflict_diagnostics")
                            .default_open(true)
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_source("conflict_diagnostics_scroll")
                                    .max_height(200.0)
                                    .show(ui, |ui| {
                                        for conflict in conflicts {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new("•").color(Color32::from_rgb(248, 113, 113)));
                                                ui.label(RichText::new(conflict).color(Color32::from_rgb(229, 231, 235)));
                                            });
                                        }
                                    });
                            });
                        }
                    }
                }
            });

        ui.add_space(20.0);

        if self.schedule.is_some() {
            // ── Manual Editing Controls ─────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(RichText::new("Schedule Management:").strong().color(Color32::from_rgb(156, 163, 175)));
                ui.add_space(8.0);
                
                let lock_label = if self.schedule_locked { "🔒 Schedule Locked" } else { "🔓 Schedule Unlocked (Edit Mode)" };
                let lock_color = if self.schedule_locked { Color32::from_rgb(107, 114, 128) } else { Color32::from_rgb(251, 146, 60) };
                let lock_btn = egui::Button::new(RichText::new(lock_label).strong().color(Color32::WHITE))
                    .fill(lock_color)
                    .rounding(6.0)
                    .min_size(egui::vec2(200.0, 28.0));
                
                if ui.add(lock_btn).on_hover_text("Unlock the schedule to manually move activities between slots and fields.").clicked() {
                    self.schedule_locked = !self.schedule_locked;
                    if self.schedule_locked {
                        self.dragged_assignment = None;
                    }
                }
                
                if !self.schedule_locked {
                    ui.add_space(12.0);
                    ui.label(RichText::new("🖱 Click and drag any activity cell below to move it.").italics().color(Color32::from_rgb(251, 146, 60)));
                }
            });
            ui.add_space(10.0);

            // ── Tab bar ────────────────────────────────────────────────────────
            let has_scheduled_divs: Vec<String> = {
                let sched = self.schedule.as_ref().unwrap();
                self.config.divisions.iter()
                    .filter(|div| {
                        sched.assignments.iter().any(|a| {
                            a.activity.division_id() == div.id
                                && !matches!(a.activity, crate::model::Activity::Interview { .. })
                        })
                    })
                    .map(|d| d.id.clone())
                    .collect()
            };

            egui::Frame::none()
                .fill(Color32::from_rgb(22, 28, 40))
                .rounding(egui::Rounding { nw: 8.0, ne: 8.0, sw: 0.0, se: 0.0 })
                .inner_margin(egui::Margin::symmetric(8.0, 6.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // All Games tab
                        let is_all = self.schedule_view_tab == ScheduleViewTab::AllGames;
                        let all_btn = egui::Button::new(
                            RichText::new("📅 All Games")
                                .strong()
                                .color(if is_all { Color32::WHITE } else { Color32::from_rgb(156, 163, 175) })
                        )
                        .fill(if is_all { Color32::from_rgb(79, 70, 229) } else { Color32::TRANSPARENT })
                        .rounding(6.0)
                        .min_size(egui::vec2(110.0, 28.0));
                        if ui.add(all_btn).clicked() {
                            self.schedule_view_tab = ScheduleViewTab::AllGames;
                        }

                        ui.add_space(4.0);

                        // Per-division tabs
                        for div_id in &has_scheduled_divs {
                            let div_name = self.config.divisions.iter()
                                .find(|d| &d.id == div_id)
                                .map(|d| d.name.as_str())
                                .unwrap_or(div_id.as_str());

                            let (_, border_col) = get_competition_colors(div_id, &self.config);
                            let is_active = self.schedule_view_tab == ScheduleViewTab::Division(div_id.clone());

                            let tab_btn = egui::Button::new(
                                RichText::new(format!("🏟 {}", div_name))
                                    .strong()
                                    .color(if is_active { Color32::WHITE } else { Color32::from_rgb(156, 163, 175) })
                            )
                            .fill(if is_active {
                                Color32::from_rgba_unmultiplied(border_col.r(), border_col.g(), border_col.b(), 60)
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(if is_active {
                                Stroke::new(1.5, border_col)
                            } else {
                                Stroke::new(0.5, Color32::from_rgb(55, 65, 81))
                            })
                            .rounding(6.0)
                            .min_size(egui::vec2(110.0, 28.0));

                            if ui.add(tab_btn).clicked() {
                                self.schedule_view_tab = ScheduleViewTab::Division(div_id.clone());
                            }
                            ui.add_space(4.0);
                        }
                    });
                });

            // ── Tab content ────────────────────────────────────────────────────
            match self.schedule_view_tab.clone() {
                ScheduleViewTab::AllGames => {
                    self.draw_all_games_timeline(ui);
                }
                ScheduleViewTab::Division(div_id) => {
                    self.draw_division_view(ui, &div_id);
                }
            }

            // ── Volunteer rosters (always shown below tabs) ────────────────────
            ui.add_space(25.0);
            ui.label(RichText::new("VOLUNTEER ASSIGNMENT ROSTERS").strong().color(Color32::from_rgb(156, 163, 175)));
            ui.add_space(8.0);

            if let Some(ref sched) = self.schedule {
                for vol in &self.config.volunteers {
                    let vol_assigns: Vec<&crate::model::ScheduleAssignment> = sched
                        .assignments
                        .iter()
                        .filter(|a| a.volunteer_ids.contains(&vol.id))
                        .collect();

                    if !vol_assigns.is_empty() {
                        egui::CollapsingHeader::new(RichText::new(format!("👤 {} ({} shifts)", vol.name, vol_assigns.len())).strong().color(Color32::WHITE))
                            .id_source(format!("vol_roster_{}", vol.id))
                            .show(ui, |ui| {
                                for assign in &vol_assigns {
                                    let slot = self.config.time_slots.iter().find(|s| s.id == assign.time_slot_id);
                                    let field = assign.field_id.as_ref().and_then(|f_id| self.config.fields.iter().find(|f| f.id == *f_id));
                                    let slot_time = slot.map_or("".to_string(), |s| format!("{} {} - {}", s.day, s.start_time, s.end_time));
                                    let location = field.map_or("Open Space / Interview Table", |f| f.name.as_str());
                                    ui.label(RichText::new(format!(
                                        "  ⏰ {} | 📍 {} | {}",
                                        slot_time, location, assign.activity.label()
                                    )).color(Color32::from_rgb(209, 213, 219)));
                                }
                            });
                        ui.add_space(4.0);
                    }
                }
            }
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(RichText::new("No Schedule Generated Yet").size(16.0).color(Color32::from_rgb(107, 114, 128)).strong());
                ui.label("Review configuration warnings and click 'Generate Schedule' to create your tournament roster.");
            });
        }
    }

    /// Draws the combined timeline visualizer (All Games tab).
    fn draw_all_games_timeline(&mut self, ui: &mut egui::Ui) {
        if self.schedule.is_none() { return; }

        ui.add_space(6.0);
        ui.label(RichText::new("SCHEDULE TIMELINE VISUALIZER").strong().color(Color32::from_rgb(156, 163, 175)));
        ui.add_space(10.0);

        let fields_list: Vec<crate::model::Field> = self.config.fields.clone();
        let mut sorted_fields = fields_list.iter().collect::<Vec<_>>();
        sorted_fields.sort_by_key(|f| match f.kind {
            FieldKind::Competition => 0,
            FieldKind::Interview => 1,
        });
        
        let slots_list = self.config.time_slots.clone();

        // Group slots by day
        let mut slots_by_day: std::collections::HashMap<String, Vec<crate::model::TimeSlot>> = std::collections::HashMap::new();
        for slot in &slots_list {
            slots_by_day.entry(slot.day.clone()).or_default().push(slot.clone());
        }
        
        let mut days: Vec<String> = slots_by_day.keys().cloned().collect();
        days.sort_by_key(|day| {
            slots_list.iter().position(|s| &s.day == day).unwrap_or(0)
        });

        let mut move_request = None;

        for day in &days {
            ui.add_space(30.0);
            ui.label(RichText::new(day.to_uppercase()).strong().size(18.0).color(Color32::from_rgb(99, 102, 241)));
            ui.add_space(10.0);

            let day_slots = slots_by_day.get(day).unwrap();

            let mut day_start_min = 24 * 60;
            let mut day_end_min = 0;
            for slot in day_slots {
                let s_min = parse_time_to_minutes(&slot.start_time);
                let e_min = parse_time_to_minutes(&slot.end_time);
                if s_min < day_start_min { day_start_min = s_min; }
                if e_min > day_end_min { day_end_min = e_min; }
            }

            // Also check day configs for the intended range
            if let Some(day_cfg) = self.config.day_configs.iter().find(|cfg| &cfg.day == day) {
                let s_min = parse_time_to_minutes(&day_cfg.start_time);
                let e_min = parse_time_to_minutes(&day_cfg.end_time);
                if s_min < day_start_min { day_start_min = s_min; }
                if e_min > day_end_min { day_end_min = e_min; }
            }

            if let Some(ref sched) = self.schedule {
                for assign in &sched.assignments {
                    if let Some(slot) = slots_list.iter().find(|s| s.id == assign.time_slot_id)
                        && &slot.day == day {
                            let s_min = parse_time_to_minutes(&slot.start_time);
                            let e_min = s_min + assign.activity.duration_minutes();
                            if e_min > day_end_min { day_end_min = e_min; }
                        }
                }
            }

            if day_start_min >= day_end_min { continue; }
            
            // Ensure at least 4 hours are shown to avoid "tiny slivers"
            if day_end_min - day_start_min < 240 {
                day_end_min = day_start_min + 240;
            }

            let total_min = day_end_min - day_start_min;
            let pixels_per_minute = 3.5;
            let col_width = 170.0;
            let col_spacing = 15.0;
            let time_col_width = 55.0;
            let timeline_padding_top = 15.0;
            let timeline_padding_bottom = 15.0;
            let timeline_height = total_min as f32 * pixels_per_minute + timeline_padding_top + timeline_padding_bottom;
            let total_width = time_col_width + sorted_fields.len() as f32 * (col_width + col_spacing);

            egui::ScrollArea::horizontal()
                .id_source(format!("timeline_scroll_h_{}", day))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        // Headers
                        let (header_rect, _) = ui.allocate_exact_size(egui::vec2(total_width, 25.0), egui::Sense::hover());
                        for (f_idx, f) in sorted_fields.iter().enumerate() {
                            let x = time_col_width + f_idx as f32 * (col_width + col_spacing);
                            let label_rect = egui::Rect::from_min_size(
                                header_rect.min + egui::vec2(x, 0.0),
                                egui::vec2(col_width, 25.0)
                            );
                            let mut child_ui = ui.child_ui(label_rect, egui::Layout::centered_and_justified(egui::Direction::LeftToRight));
                            child_ui.label(RichText::new(&f.name).strong().color(Color32::from_rgb(209, 213, 219)));
                        }

                        // Grid
                        let (rect, _response) = ui.allocate_exact_size(
                            egui::vec2(total_width, timeline_height),
                            egui::Sense::hover()
                        );
                        let painter = ui.painter_at(rect);

                        for (f_idx, _) in sorted_fields.iter().enumerate() {
                            let x = time_col_width + f_idx as f32 * (col_width + col_spacing);
                            let col_rect = egui::Rect::from_min_max(
                                egui::pos2(rect.min.x + x, rect.min.y),
                                egui::pos2(rect.min.x + x + col_width, rect.max.y)
                            );
                            painter.rect_filled(col_rect, 0.0, Color32::from_rgb(22, 28, 38));
                        }

                        let start_hour = (day_start_min / 10) * 10;
                        let end_hour = day_end_min.div_ceil(10) * 10;
                        for min in (start_hour..=end_hour).step_by(10) {
                            if min >= day_start_min && min <= day_end_min {
                                let y = (min - day_start_min) as f32 * pixels_per_minute;
                                let line_y = rect.min.y + timeline_padding_top + y;
                                let is_major = min % 30 == 0;
                                let stroke_color = if is_major { Color32::from_rgb(75, 85, 99) } else { Color32::from_rgb(55, 65, 81) };
                                let stroke_width = if is_major { 0.6 } else { 0.3 };
                                painter.line_segment(
                                    [egui::pos2(rect.min.x + time_col_width, line_y), egui::pos2(rect.max.x - col_spacing, line_y)],
                                    Stroke::new(stroke_width, stroke_color)
                                );
                                let hr = min / 60;
                                let mn = min % 60;
                                let time_str = format!("{:02}:{:02}", hr, mn);
                                if is_major || pixels_per_minute >= 1.5 {
                                    let label_color = if is_major { Color32::from_rgb(156, 163, 175) } else { Color32::from_rgb(107, 114, 128) };
                                    let font_size = if is_major { 11.0 } else { 9.0 };
                                    painter.text(egui::pos2(rect.min.x + 5.0, line_y), egui::Align2::LEFT_CENTER, time_str, egui::FontId::proportional(font_size), label_color);
                                }
                            }
                        }

                        for (f_idx, _) in sorted_fields.iter().enumerate() {
                            let x = time_col_width + f_idx as f32 * (col_width + col_spacing);
                            painter.line_segment([egui::pos2(rect.min.x + x, rect.min.y), egui::pos2(rect.min.x + x, rect.max.y)], Stroke::new(0.5, Color32::from_rgb(55, 65, 81)));
                            let right_x = x + col_width;
                            painter.line_segment([egui::pos2(rect.min.x + right_x, rect.min.y), egui::pos2(rect.min.x + right_x, rect.max.y)], Stroke::new(0.5, Color32::from_rgb(55, 65, 81)));
                        }

                        // Drop highlights and move logic
                        if let Some(dragged_idx) = self.dragged_assignment {
                            let pointer_pos = ui.input(|i| i.pointer.interact_pos().unwrap_or(egui::Pos2::ZERO));
                            
                            for (f_idx, field) in sorted_fields.iter().enumerate() {
                                let x = time_col_width + f_idx as f32 * (col_width + col_spacing);
                                for slot in day_slots {
                                    if slot.kind != field.kind { continue; }
                                    
                                    let start_m = parse_time_to_minutes(&slot.start_time);
                                    let y = (start_m - day_start_min) as f32 * pixels_per_minute;
                                    let h = slot.duration_minutes() as f32 * pixels_per_minute;
                                    
                                    let cell_rect = egui::Rect::from_min_size(
                                        rect.min + egui::vec2(x, timeline_padding_top + y),
                                        egui::vec2(col_width, h)
                                    );
                                    
                                    if cell_rect.contains(pointer_pos) {
                                        painter.rect_stroke(cell_rect, 4.0, Stroke::new(2.5, Color32::from_rgb(251, 146, 60)));
                                        
                                        if ui.input(|i| i.pointer.any_released()) {
                                            move_request = Some((dragged_idx, slot.id.clone(), Some(field.id.clone())));
                                        }
                                    }
                                }
                            }
                        }

                        for kind in [FieldKind::Competition, FieldKind::Interview] {
                            let mut kind_slots: Vec<&crate::model::TimeSlot> = day_slots.iter().filter(|s| s.kind == kind).collect();
                            if kind_slots.is_empty() { continue; }
                            
                            kind_slots.sort_by_key(|s| parse_time_to_minutes(&s.start_time));
                            
                            let kind_field_indices: Vec<usize> = sorted_fields.iter().enumerate()
                                .filter(|(_, f)| f.kind == kind)
                                .map(|(i, _)| i)
                                .collect();
                                
                            if kind_field_indices.is_empty() { continue; }
                            
                            let min_f_idx = *kind_field_indices.iter().min().unwrap();
                            let max_f_idx = *kind_field_indices.iter().max().unwrap();
                            
                            let break_x_min = time_col_width + min_f_idx as f32 * (col_width + col_spacing);
                            let break_x_max = time_col_width + max_f_idx as f32 * (col_width + col_spacing) + col_width;

                            for idx in 0..kind_slots.len().saturating_sub(1) {
                                let slot = kind_slots[idx];
                                let next_slot = kind_slots[idx + 1];
                                let t1 = parse_time_to_minutes(&slot.end_time);
                                let t2 = parse_time_to_minutes(&next_slot.start_time);
                                if t2 > t1 {
                                    let gap = t2 - t1;
                                    if gap >= 45 { // Only show major breaks (Lunch)
                                        let y1 = (t1 - day_start_min) as f32 * pixels_per_minute;
                                        let y2 = (t2 - day_start_min) as f32 * pixels_per_minute;
                                        let break_rect = egui::Rect::from_min_max(
                                            egui::pos2(rect.min.x + break_x_min, rect.min.y + timeline_padding_top + y1),
                                            egui::pos2(rect.min.x + break_x_max, rect.min.y + timeline_padding_top + y2)
                                        );
                                        painter.rect(break_rect, 4.0,
                                            Color32::from_rgb(67, 20, 30),
                                            Stroke::new(0.5, Color32::from_rgb(190, 24, 74))
                                        );
                                        let break_label = format!("🍔 Lunch Break ({}m)", gap);
                                        let label_color = Color32::from_rgb(251, 113, 133);
                                        painter.text(break_rect.center(), egui::Align2::CENTER_CENTER, break_label, egui::FontId::proportional(11.0), label_color);
                                    }
                                }
                            }
                        }

                        if let Some(ref sched) = self.schedule {
                            for (idx, assign) in sched.assignments.iter().enumerate() {
                                if let Some(ref f_id) = assign.field_id
                                    && let Some(slot) = slots_list.iter().find(|s| s.id == assign.time_slot_id)
                                        && &slot.day == day
                                            && let Some(f_idx) = sorted_fields.iter().position(|f| &f.id == f_id) {
                                                let start_m = parse_time_to_minutes(&slot.start_time);
                                                let dur = assign.activity.duration_minutes();
                                                let y = (start_m - day_start_min) as f32 * pixels_per_minute;
                                                let h = dur as f32 * pixels_per_minute;
                                                let x = time_col_width + f_idx as f32 * (col_width + col_spacing) + 4.0;
                                                let w = col_width - 8.0;
                                                
                                                let mut card_rect = egui::Rect::from_min_size(rect.min + egui::vec2(x, timeline_padding_top + y), egui::vec2(w, h));
                                                
                                                let sense = if !self.schedule_locked { egui::Sense::drag() } else { egui::Sense::hover() };
                                                let response = ui.interact(card_rect, ui.id().with(idx), sense);
                                                
                                                if response.drag_started() {
                                                    self.dragged_assignment = Some(idx);
                                                    self.drag_accumulated_offset = egui::Vec2::ZERO;
                                                }
                                                
                                                if self.dragged_assignment == Some(idx) {
                                                    self.drag_accumulated_offset += response.drag_delta();
                                                    card_rect = card_rect.translate(self.drag_accumulated_offset);
                                                }
                                                
                                                let mut child_ui = ui.child_ui(card_rect, egui::Layout::top_down(egui::Align::Min));
                                                let conflicts: &[AssignmentConflict] = self.assignment_conflicts.get(&idx).map(|v| v.as_slice()).unwrap_or(&[]);
                                                draw_schedule_cell(&mut child_ui, assign, &self.config, &slot.id, w, h, conflicts);
                                            }
                            }
                        }
                    });
                });
        }

        if let Some(ref sched) = self.schedule {
            let open_space_assignments: Vec<(usize, &crate::model::ScheduleAssignment)> = sched.assignments.iter()
                .enumerate()
                .filter(|(_, a)| a.field_id.is_none())
                .collect();

            if !open_space_assignments.is_empty() {
                ui.add_space(20.0);
                ui.label(RichText::new("INTERVIEWS & UNALLOCATED EVENTS").strong().color(Color32::from_rgb(156, 163, 175)));
                ui.add_space(5.0);
                for slot in &slots_list {
                    let slot_assigns: Vec<&(usize, &crate::model::ScheduleAssignment)> = open_space_assignments
                        .iter()
                        .filter(|(_, a)| a.time_slot_id == slot.id)
                        .collect();
                    
                    let pointer_pos = ui.input(|i| i.pointer.interact_pos().unwrap_or(egui::Pos2::ZERO));
                    let is_released = ui.input(|i| i.pointer.any_released());

                    ui.horizontal(|ui| {
                        let _label_resp = ui.label(RichText::new(format!("{} ({} - {}):", slot.day, slot.start_time, slot.end_time)).strong().color(Color32::WHITE));
                        
                        // Drop target for unallocated section
                        if let Some(dragged_idx) = self.dragged_assignment {
                            // If it's an interview (no field_id) or we want to allow moving grid items to unallocated
                            if ui.max_rect().contains(pointer_pos) { // This is a bit broad, but works within the horizontal layout
                                // We'll check if the pointer is near this row
                                let row_rect = ui.max_rect();
                                if pointer_pos.y >= row_rect.min.y && pointer_pos.y <= row_rect.max.y {
                                    ui.painter().rect_stroke(row_rect.expand(2.0), 4.0, Stroke::new(1.5, Color32::from_rgb(251, 146, 60)));
                                    if is_released {
                                        move_request = Some((dragged_idx, slot.id.clone(), None));
                                    }
                                }
                            }
                        }

                        for (idx, assign) in slot_assigns {
                            let conflicts: &[AssignmentConflict] = self.assignment_conflicts.get(idx).map(|v| v.as_slice()).unwrap_or(&[]);
                            
                            let sense = if !self.schedule_locked { egui::Sense::drag() } else { egui::Sense::hover() };
                            let (rect, response) = ui.allocate_at_least(egui::vec2(145.0, 48.0), sense);
                            
                            if response.drag_started() {
                                self.dragged_assignment = Some(*idx);
                                self.drag_accumulated_offset = egui::Vec2::ZERO;
                            }

                            let mut card_rect = rect;
                            if self.dragged_assignment == Some(*idx) {
                                self.drag_accumulated_offset += response.drag_delta();
                                card_rect = card_rect.translate(self.drag_accumulated_offset);
                            }
                            let mut child_ui = ui.child_ui(card_rect, egui::Layout::top_down(egui::Align::Min));
                            draw_schedule_cell(&mut child_ui, assign, &self.config, &assign.time_slot_id, 145.0, 48.0, conflicts);
                        }
                    });
                    ui.add_space(5.0);
                }
            }
        }

        if ui.input(|i| i.pointer.any_released()) {
            self.dragged_assignment = None;
        }

        if let Some((idx, slot_id, field_id)) = move_request {
            if let Some(ref mut sched) = self.schedule {
                sched.assignments[idx].time_slot_id = slot_id;
                sched.assignments[idx].field_id = field_id;
            }
            self.re_evaluate_schedule();
        }
    }

    /// Draws the round-by-round table for a single division.
    fn draw_division_rounds_table(
        &self,
        ui: &mut egui::Ui,
        div_id: &str,
        rows: &[RoundRow],
        is_h2h: bool,
        accent: Color32,
    ) {
        if rows.is_empty() {
            ui.label(RichText::new("No matches scheduled yet.").color(Color32::from_rgb(107, 114, 128)).italics());
            return;
        }

        // Reserve a stable width before entering the scroll area to prevent layout feedback loops.
        // Do NOT call ui.available_width() inside scroll area children.
        let panel_width = ui.available_width().max(400.0);

        egui::ScrollArea::vertical()
            .id_source(format!("div_rounds_scroll_{}", div_id))
            .max_height(550.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in rows {
                    let is_finals = row.matches.iter().any(|m| m.is_final);

                    let header_bg = if is_finals { Color32::from_rgb(67, 52, 10) } else { Color32::from_rgb(30, 37, 50) };
                    let header_accent = if is_finals { Color32::from_rgb(251, 191, 36) } else { accent };

                    // Round header — use allocate_exact_size to avoid available_width feedback
                    egui::Frame::none()
                        .fill(header_bg)
                        .rounding(egui::Rounding { nw: 6.0, ne: 6.0, sw: 0.0, se: 0.0 })
                        .inner_margin(egui::Margin::symmetric(12.0, 6.0))
                        .show(ui, |ui| {
                            ui.set_min_width(panel_width - 4.0);
                            ui.horizontal(|ui| {
                                let round_icon = if is_finals { "🏆" } else { "🔄" };
                                ui.label(RichText::new(format!("{} {}", round_icon, row.round_label))
                                    .strong().size(13.0).color(header_accent));
                                if !row.bye_teams.is_empty() {
                                    ui.add_space(12.0);
                                    ui.label(RichText::new(format!("🟡 Bye: {}", row.bye_teams.join(", ")))
                                        .size(11.5).color(Color32::from_rgb(253, 224, 71)));
                                }
                            });
                        });

                    // Match rows body
                    egui::Frame::none()
                        .fill(Color32::from_rgb(17, 22, 32))
                        .rounding(egui::Rounding { nw: 0.0, ne: 0.0, sw: 6.0, se: 6.0 })
                        .stroke(Stroke::new(1.0, Color32::from_rgb(38, 46, 60)))
                        .show(ui, |ui| {
                            ui.set_min_width(panel_width - 4.0);

                            // Column headers
                            egui::Frame::none()
                            .fill(Color32::from_rgb(22, 28, 40))
                            .inner_margin(egui::Margin::symmetric(12.0, 5.0))
                            .show(ui, |ui| {
                                ui.set_min_width(panel_width - 8.0);
                                ui.horizontal(|ui| {
                                    ui.allocate_ui(egui::vec2(90.0, 16.0), |ui| {
                                        ui.label(RichText::new("Day / Time").size(10.5).color(Color32::from_rgb(107, 114, 128)).strong());
                                    });
                                    ui.allocate_ui(egui::vec2(160.0, 16.0), |ui| {

                                            ui.label(RichText::new("Field / Arena").size(10.5).color(Color32::from_rgb(107, 114, 128)).strong());
                                        });
                                        ui.label(RichText::new(if is_h2h { "Match" } else { "Team" })
                                            .size(10.5).color(Color32::from_rgb(107, 114, 128)).strong());
                                    });
                                });

                            for (m_idx, m) in row.matches.iter().enumerate() {
                                let row_bg = if m_idx % 2 == 0 { Color32::from_rgb(17, 22, 32) } else { Color32::from_rgb(20, 26, 38) };

                                // Draw thin separator above every row after the first using painter
                                if m_idx > 0 {
                                    let sep_size = egui::vec2(panel_width - 32.0, 1.0);
                                    let (sep_rect, _) = ui.allocate_exact_size(sep_size, egui::Sense::hover());
                                    ui.painter().rect_filled(sep_rect, 0.0, Color32::from_rgb(38, 46, 60));
                                }

                                egui::Frame::none()
                                    .fill(row_bg)
                                    .inner_margin(egui::Margin::symmetric(12.0, 7.0))
                                    .show(ui, |ui| {
                                        ui.set_min_width(panel_width - 8.0);
                                        ui.horizontal(|ui| {
                                            // Day / Time
                                            ui.allocate_ui(egui::vec2(90.0, 20.0), |ui| {
                                                let day_short = if m.day.len() > 3 { &m.day[..3] } else { &m.day };
                                                let display_time = if m.time.is_empty() { "—".to_string() } else { format!("{} {}", day_short, m.time) };
                                                ui.label(RichText::new(display_time)
                                                    .size(12.0).color(Color32::from_rgb(209, 213, 219)).monospace());
                                            });
                                            // Field
                                            ui.allocate_ui(egui::vec2(160.0, 20.0), |ui| {
                                                let field_color = if m.field_name == "—" { Color32::from_rgb(107, 114, 128) } else { Color32::from_rgb(156, 163, 175) };
                                                ui.label(RichText::new(&m.field_name).size(11.5).color(field_color));
                                            });
                                            // Match or team
                                            if is_h2h {
                                                let icon = if m.is_final { "🏆" } else { "⚽" };
                                                ui.label(RichText::new(icon).size(12.0));
                                                ui.label(RichText::new(&m.team_a).strong().size(12.0).color(Color32::WHITE));
                                                ui.label(RichText::new(" vs ").size(11.5).color(Color32::from_rgb(107, 114, 128)));
                                                ui.label(RichText::new(&m.team_b).strong().size(12.0).color(Color32::WHITE));
                                            } else {
                                                ui.label(RichText::new("🤖").size(12.0));
                                                ui.label(RichText::new(&m.team_a).strong().size(12.0).color(Color32::WHITE));
                                            }
                                        });
                                    });
                            }
                        });

                    ui.add_space(12.0);
                }
            });
    }
}

impl AppState {
    fn draw_division_view(&mut self, ui: &mut egui::Ui, div_id: &str) {
        let div = self.config.divisions.iter().find(|d| d.id == div_id).cloned();
        if div.is_none() { return; }
        let div = div.unwrap();
        let div_name = &div.name;
        let is_h2h = div.mode == SchedulingMode::HeadToHead;
        let (_, accent) = get_competition_colors(div_id, &self.config);

        ui.add_space(8.0);
        // Header
        ui.horizontal(|ui| {
            ui.label(RichText::new("●").size(16.0).color(accent));
            ui.label(RichText::new(div_name).strong().size(15.0).color(Color32::WHITE));
            ui.label(RichText::new(if is_h2h { " · Head-to-Head" } else { " · Individual Run" })
                .size(11.0).color(Color32::from_rgb(107, 114, 128)));
        });

        // Subtitle: explain round count
        if is_h2h {
            if let Some(rows) = self.division_rounds.get(div_id) {
                let rr_rounds: Vec<&RoundRow> = rows.iter().filter(|r| !r.matches.iter().any(|m| m.is_final)).collect();
                let finals_rounds: Vec<&RoundRow> = rows.iter().filter(|r| r.matches.iter().any(|m| m.is_final)).collect();
                let parts: Vec<String> = [
                    if rr_rounds.is_empty() { None } else { Some(format!("{} round-robin round{}", rr_rounds.len(), if rr_rounds.len() == 1 { "" } else { "s" })) },
                    if finals_rounds.is_empty() { None } else { Some(format!("{} finals stage{}", finals_rounds.len(), if finals_rounds.len() == 1 { "" } else { "s" })) },
                ].into_iter().flatten().collect();
                if !parts.is_empty() {
                    ui.label(RichText::new(parts.join(" + ")).size(11.0).color(Color32::from_rgb(107, 114, 128)).italics());
                }
            }
        }
        ui.add_space(8.0);

        // Sub-tabs
        ui.horizontal(|ui| {
            let tabs = [
                (crate::gui::DivisionSubTab::Rounds, "🔄 Rounds"),
                (crate::gui::DivisionSubTab::Teams, "👥 Teams"),
                (crate::gui::DivisionSubTab::Interviews, "💬 Interviews"),
            ];
            for (tab, label) in tabs {
                let is_active = self.active_division_sub_tab == tab;
                let text_color = if is_active { Color32::WHITE } else { Color32::from_rgb(156, 163, 175) };
                let bg_color = if is_active { Color32::from_rgb(79, 70, 229) } else { Color32::from_rgb(31, 41, 55) };
                
                let btn = egui::Button::new(RichText::new(label).strong().color(text_color))
                    .fill(bg_color)
                    .rounding(4.0)
                    .min_size(egui::vec2(100.0, 26.0));
                
                if ui.add(btn).clicked() {
                    self.active_division_sub_tab = tab;
                }
                ui.add_space(6.0);
            }
        });
        ui.add_space(10.0);

        match self.active_division_sub_tab {
            crate::gui::DivisionSubTab::Rounds => {
                if let Some(rows) = self.division_rounds.get(div_id).cloned() {
                    self.draw_division_rounds_table(ui, div_id, &rows, is_h2h, accent);
                } else {
                    ui.label(RichText::new("No rounds scheduled yet.").italics().color(Color32::from_rgb(107, 114, 128)));
                }
            }
            crate::gui::DivisionSubTab::Teams => {
                self.draw_division_teams(ui, div_id, accent);
            }
            crate::gui::DivisionSubTab::Interviews => {
                self.draw_division_interviews(ui, div_id, accent);
            }
        }
    }

    fn draw_division_teams(&self, ui: &mut egui::Ui, div_id: &str, accent: Color32) {
        let div_teams: Vec<&crate::model::Team> = self.config.teams.iter().filter(|t| t.division_id == div_id).collect();
        if div_teams.is_empty() {
            ui.label(RichText::new("No teams in this division.").italics().color(Color32::from_rgb(107, 114, 128)));
            return;
        }

        let panel_width = ui.available_width().max(400.0);

        egui::ScrollArea::vertical()
            .id_source(format!("div_teams_scroll_{}", div_id))
            .show(ui, |ui| {
                for team in div_teams {
                    egui::Frame::none()
                        .fill(Color32::from_rgb(26, 32, 44))
                        .rounding(6.0)
                        .stroke(Stroke::new(1.0, Color32::from_rgb(55, 65, 81)))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.set_min_width(panel_width - 16.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&team.name).strong().color(Color32::WHITE));
                                ui.label(RichText::new(format!("({})", team.organization)).size(10.0).color(Color32::from_rgb(156, 163, 175)));
                            });
                            ui.add_space(4.0);
                            
                            // Find matches for this team
                            if let Some(ref sched) = self.schedule {
                                let mut team_matches: Vec<&crate::model::ScheduleAssignment> = sched.assignments.iter()
                                    .filter(|a| a.activity.teams().contains(&team.name.as_str()))
                                    .collect();
                                
                                // Sort team matches chronologically
                                team_matches.sort_by_key(|a| {
                                    let slot = self.config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                                    slot.map(|s| (s.day.clone(), parse_time_to_minutes(&s.start_time)))
                                });

                                if team_matches.is_empty() {
                                    ui.label(RichText::new("No activities scheduled.").small().italics().color(Color32::from_rgb(107, 114, 128)));
                                } else {
                                    for assign in team_matches {
                                        let slot = self.config.time_slots.iter().find(|s| s.id == assign.time_slot_id);
                                        let field = assign.field_id.as_ref().and_then(|f_id| self.config.fields.iter().find(|f| f.id == *f_id));
                                        let time_str = slot.map_or("?".to_string(), |s| format!("{} {}", &s.day[..3], s.start_time));
                                        let field_name = field.map_or("Interview Table / Open Space".to_string(), |f| f.name.clone());
                                        
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(format!("⏰ {} | 📍 {}", time_str, field_name)).size(11.0).color(Color32::from_rgb(209, 213, 219)));
                                            ui.label(RichText::new(assign.activity.label()).size(11.0).color(accent).strong());
                                        });
                                    }
                                }
                            }
                        });
                    ui.add_space(8.0);
                }
            });
    }

    fn draw_division_interviews(&self, ui: &mut egui::Ui, div_id: &str, accent: Color32) {
        if let Some(ref sched) = self.schedule {
            let mut interviews: Vec<&crate::model::ScheduleAssignment> = sched.assignments.iter()
                .filter(|a| a.activity.division_id() == div_id && matches!(a.activity, crate::model::Activity::Interview { .. }))
                .collect();
            
            // Sort interviews chronologically
            interviews.sort_by_key(|a| {
                let slot = self.config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                slot.map(|s| (s.day.clone(), parse_time_to_minutes(&s.start_time)))
            });

            if interviews.is_empty() {
                ui.label(RichText::new("No interviews scheduled for this division.").italics().color(Color32::from_rgb(107, 114, 128)));
                return;
            }

            let panel_width = ui.available_width().max(400.0);

            egui::ScrollArea::vertical()
                .id_source(format!("div_interviews_scroll_{}", div_id))
                .show(ui, |ui| {
                    for assign in interviews {
                        let slot = self.config.time_slots.iter().find(|s| s.id == assign.time_slot_id);
                        let field = assign.field_id.as_ref().and_then(|f_id| self.config.fields.iter().find(|f| f.id == *f_id));
                        let time_str = slot.map_or("?".to_string(), |s| format!("{} {}", s.day, s.start_time));
                        let field_name = field.map_or("Interview Table / Open Space".to_string(), |f| f.name.clone());

                        egui::Frame::none()
                            .fill(Color32::from_rgb(26, 32, 44))
                            .rounding(6.0)
                            .stroke(Stroke::new(1.0, Color32::from_rgb(55, 65, 81)))
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                ui.set_min_width(panel_width - 16.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(assign.activity.label()).strong().color(Color32::WHITE));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(RichText::new(format!("⏰ {} | 📍 {}", time_str, field_name)).color(accent).strong());
                                    });
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
        } else {
            ui.label(RichText::new("Schedule not generated yet.").italics().color(Color32::from_rgb(107, 114, 128)));
        }
    }

    pub fn solve_and_schedule(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.solver_rx = Some(rx);
        self.solve_status = "Solving...".to_string();
        self.solve_message = "Starting solver thread...".to_string();
        self.solve_progress = 0.0;
        self.solver_max_iter_reported = 0;
        self.solver_current_restart_idx = 0;
        self.solver_restarts_progress.clear();

        let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.solver_cancel_flag = Some(cancel_flag.clone());

        let config = self.config.clone();
        let mut params = self.get_solver_params();
        params.cancel_flag = Some(cancel_flag);

        std::thread::spawn(move || {
            let tx_clone = tx.clone();
            let result = solve_schedule(&config, &params, move |restart, total_restarts, iteration, total_iterations, hard, soft| {
                let _ = tx_clone.send(SolverMessage::Progress { 
                    restart, 
                    total_restarts, 
                    iteration, 
                    total_iterations, 
                    hard, 
                    soft 
                });
            });
            let _ = tx.send(SolverMessage::Done(result));
        });
    }
}
