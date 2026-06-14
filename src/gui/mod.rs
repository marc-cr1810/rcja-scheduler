mod helpers;
mod dashboard;
mod divisions;
mod teams;
mod fields;
mod time_slots;
mod volunteers;
mod solver;
mod app_state;
mod persistence;

use crate::model::Schedule;
use crate::scheduler::SolverParams;
use eframe::egui::{self, Color32, RichText};

pub(crate) use helpers::setup_custom_style;
pub use app_state::{AppState, VolRosterSort};

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum Tab {
    Dashboard,
    Divisions,
    Teams,
    Fields,
    TimeSlots,
    Volunteers,
    Solver,
}

#[derive(PartialEq, Clone)]
pub enum ScheduleViewTab {
    AllGames,
    Division(String),
}

#[derive(PartialEq, Clone, Copy)]
pub enum DivisionSubTab {
    Rounds,
    Teams,
    Interviews,
}

#[derive(PartialEq, Clone, Copy)]
pub enum VolunteerSubTab {
    Availability,
    WorkloadHeatmap,
}

#[derive(PartialEq, Clone, Copy)]
pub enum TeamSubTab {
    List,
    GapAnalysis,
}

pub enum SolverMessage {
    Progress { 
        restart: usize, 
        total_restarts: usize,
        iteration: usize,
        total_iterations: usize,
        hard: f64, 
        soft: f64 
    },
    Done(Option<Schedule>),
}

pub enum ExportMessage {
    Progress(f32),
    Done(String),
    Error(String),
}

impl eframe::App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_solver_messages(ctx);
        self.handle_export_messages(ctx);

        // TOP PANEL
        egui::TopBottomPanel::top("header_panel").show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading(RichText::new("RoboCup Jr Australia Coordinator Workspace").strong().color(Color32::from_rgb(129, 140, 248)));
                        let subtitle = if let Some(path) = &self.current_file_path {
                            format!("Per-Competition Workspace & Schedule Solver - {}", path.display())
                        } else {
                            "Per-Competition Workspace & Schedule Solver - Unsaved".to_string()
                        };
                        ui.label(RichText::new(subtitle).size(11.0).color(Color32::from_rgb(156, 163, 175)));
                    });
                    
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚡ Load Demo Data").clicked() {
                            self.load_demo_data();
                        }
                        if ui.button("📥 Load Config").clicked() {
                            self.load_config();
                        }
                        if ui.button("💾 Save As...").clicked() {
                            self.save_config_as();
                        }
                        if ui.button("💾 Save").clicked() {
                            self.save_config();
                        }
                        if self.schedule.is_some() {
                            ui.separator();
                            let export_btn = egui::Button::new("📤 Full Export (CSV & PDF)");
                            if ui.add_enabled(!self.is_exporting, export_btn).clicked() {
                                self.export_full_tournament();
                            }
                            if ui.button("📊 Export Master CSV").clicked() {
                                self.export_to_csv();
                            }
                        }
                    });
                });

                if self.is_exporting {
                    ui.add_space(6.0);
                    egui::Frame::none()
                        .fill(Color32::from_rgb(30, 41, 59))
                        .rounding(4.0)
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("📄 Generating tournament documents...").strong().color(Color32::from_rgb(129, 140, 248)));
                                ui.add(egui::ProgressBar::new(self.export_progress)
                                    .show_percentage()
                                    .animate(true)
                                    .desired_width(ui.available_width() - 20.0));
                            });
                        });
                }
                ui.add_space(8.0);
            });
        });

        // BOTTOM PANEL
        egui::TopBottomPanel::bottom("status_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Status: ").color(Color32::from_rgb(107, 114, 128)));
                ui.label(RichText::new(&self.status_message).strong().color(Color32::from_rgb(229, 231, 235)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("Solver Status: {}", self.solve_status)).strong().color(Color32::from_rgb(129, 140, 248)));
                });
            });
        });

        // SIDE PANEL
        egui::SidePanel::left("navigation_panel").width_range(210.0..=240.0).show(ctx, |ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("WORKSPACE PANELS").size(10.0).color(Color32::from_rgb(107, 114, 128)).strong());
            ui.add_space(5.0);

            let tab_buttons = vec![
                (Tab::Dashboard, "📊 Dashboard & Alerts"),
                (Tab::Divisions, "🏆 Divisions"),
                (Tab::Teams, "👥 Teams"),
                (Tab::Fields, "🏟 Fields & Arenas"),
                (Tab::TimeSlots, "📅 Time Slots"),
                (Tab::Volunteers, "👤 Volunteers"),
                (Tab::Solver, "⚙ Schedule Solver"),
            ];

            for (tab, label) in tab_buttons {
                let is_active = self.active_tab == tab;
                let text_color = if is_active { Color32::WHITE } else { Color32::from_rgb(156, 163, 175) };

                let button = egui::Button::new(RichText::new(label).color(text_color).strong())
                    .fill(if is_active { Color32::from_rgb(79, 70, 229) } else { Color32::TRANSPARENT })
                    .rounding(egui::Rounding::same(6.0))
                    .min_size(egui::vec2(180.0, 32.0));

                ui.horizontal(|ui| {
                    if ui.add(button).clicked() {
                        self.active_tab = tab;
                        self.status_message = format!("Opened {} tab", label.split(' ').nth(1).unwrap_or(""));
                        self.update_diagnostics();
                    }

                    if tab == Tab::Dashboard {
                        let error_count = self.diagnostics.iter().filter(|d| matches!(d.severity, crate::validator::DiagnosticSeverity::Error)).count();
                        let warn_count = self.diagnostics.iter().filter(|d| matches!(d.severity, crate::validator::DiagnosticSeverity::Warning)).count();
                        
                        if error_count > 0 || warn_count > 0 {
                            let (color, text_color, count) = if error_count > 0 {
                                (Color32::from_rgb(239, 68, 68), Color32::WHITE, error_count)
                            } else {
                                (Color32::from_rgb(251, 191, 36), Color32::BLACK, warn_count)
                            };

                            let badge_size = 18.0;
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(badge_size, badge_size), egui::Sense::hover());
                            ui.painter().circle_filled(rect.center(), badge_size / 2.0, color);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                count.to_string(),
                                egui::FontId::proportional(11.0),
                                text_color,
                            );

                            if error_count > 0 {
                                ui.interact(rect, ui.id(), egui::Sense::hover()).on_hover_text(format!("{} critical errors", error_count));
                            } else {
                                ui.interact(rect, ui.id(), egui::Sense::hover()).on_hover_text(format!("{} warnings", warn_count));
                            }
                        }                    }
                });
                ui.add_space(4.0);
            }
        });

        // CENTRAL PANEL
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                match self.active_tab {
                    Tab::Dashboard => self.draw_dashboard(ui),
                    Tab::Divisions => self.draw_divisions(ui),
                    Tab::Teams => self.draw_teams(ui),
                    Tab::Fields => self.draw_fields(ui),
                    Tab::TimeSlots => self.draw_time_slots(ui),
                    Tab::Volunteers => self.draw_volunteers(ui),
                    Tab::Solver => self.draw_solver(ui),
                }
            });
        });
    }
}

impl AppState {
    fn handle_export_messages(&mut self, ctx: &egui::Context) {
        if let Some(ref rx) = self.export_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    ExportMessage::Progress(p) => {
                        self.export_progress = p;
                    }
                    ExportMessage::Done(msg) => {
                        self.status_message = msg;
                        self.is_exporting = false;
                        self.export_rx = None;
                        break;
                    }
                    ExportMessage::Error(err) => {
                        self.status_message = format!("Export failed: {}", err);
                        self.is_exporting = false;
                        self.export_rx = None;
                        break;
                    }
                }
            }
            ctx.request_repaint();
        }
    }

    fn handle_solver_messages(&mut self, ctx: &egui::Context) {
        if let Some(ref rx) = self.solver_rx {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    SolverMessage::Progress { restart: restart_idx, total_restarts, iteration, total_iterations, hard, soft } => {
                        // Ensure we have enough slots in the progress vector
                        if self.solver_restarts_progress.len() != total_restarts {
                            self.solver_restarts_progress = vec![0; total_restarts];
                        }
                        
                        // Update this restart's progress
                        if iteration > self.solver_restarts_progress[restart_idx] {
                            self.solver_restarts_progress[restart_idx] = iteration;
                        }

                        // Calculate total aggregated progress across all parallel threads
                        let total_work = total_restarts * total_iterations;
                        let completed_work: usize = self.solver_restarts_progress.iter().sum();
                        self.solve_progress = (completed_work as f32 / total_work as f32).clamp(0.0, 1.0);
                        
                        // Use high-water mark for the text display so it doesn't jump back and forth between threads
                        // We show the stats for the restart that is furthest along.
                        if restart_idx >= self.solver_current_restart_idx || iteration > self.solver_max_iter_reported {
                            self.solver_current_restart_idx = restart_idx;
                            self.solver_max_iter_reported = iteration;
                            
                            self.solve_message = format!(
                                "Solving... Attempt {}/{} | Iteration {}/{} | Best so far — Hard Conflicts: {}, Soft: {:.1}",
                                restart_idx + 1, total_restarts, iteration, total_iterations, hard, soft
                            );
                        }
                    }
                    SolverMessage::Done(result) => {
                        let was_cancelled = self.solver_cancel_flag.as_ref()
                            .map(|f| f.load(std::sync::atomic::Ordering::Relaxed))
                            .unwrap_or(false);

                        self.solve_progress = 1.0;
                        self.solver_cancel_flag = None;

                        if let Some(sched) = result {
                            self.schedule = Some(sched);
                            self.re_evaluate_schedule();
                            
                            // Override the manual message with the solver success message.
                            // The hard-conflict count and soft penalty come from the
                            // same engine the solver optimised and the live readout
                            // reported, so the numbers are consistent end to end.
                            if let Some(ref sched) = self.schedule {
                                let params = self.get_solver_params();
                                let (_hard, soft) = crate::scheduler::evaluate_schedule_cost(&self.config, sched, &params);
                                let conflicts_count = self.schedule_conflicts.len();
                                self.solve_message = format!(
                                    "Schedule optimized successfully! Hard Conflicts: {}, Soft Penalties: {:.1}",
                                    conflicts_count, soft
                                );
                                self.status_message = "Schedule solved successfully!".to_string();
                            }
                        } else {
                            self.clear_schedule();
                            if was_cancelled {
                                self.solve_status = "Cancelled".to_string();
                                self.solve_message = "Solver was stopped by user.".to_string();
                                self.status_message = "Solver stopped.".to_string();
                            } else {
                                self.solve_status = "Failed".to_string();
                                self.solve_message = "Could not find a valid schedule.".to_string();
                                self.status_message = "Solver error occurred.".to_string();
                            }
                        }
                        self.solver_rx = None;
                        self.update_diagnostics();
                        break;
                    }
                }
            }
            ctx.request_repaint();
        }
    }

    pub fn get_solver_params(&self) -> SolverParams {
        SolverParams {
            max_iterations: self.solver_iterations,
            num_restarts: self.solver_restarts,
            fairness_mode: self.solver_fairness_mode,
            vol_consecutive_weight: self.solver_vol_consecutive_weight,
            team_back_to_back_weight: self.solver_team_back_to_back_weight,
            field_variety_weight: self.solver_field_variety_weight,
            field_balance_weight: self.solver_field_balance_weight,
            vol_capability_weight: self.solver_vol_capability_weight,
            interview_late_weight: self.solver_interview_late_weight,
            interview_match_gap_weight: self.solver_interview_match_gap_weight,
            vol_specialist_mode: self.solver_vol_specialist_mode,
            team_wait_time_weight: self.solver_team_wait_time_weight,
            field_variety_strict: self.solver_field_variety_strict,
            vol_travel_weight: self.solver_vol_travel_weight,
            round_order_weight: self.solver_round_order_weight,
            vol_daily_shift_cap: self.solver_vol_daily_shift_cap,
            peak_period_weight: self.solver_peak_period_weight,
            finals_priority_multiplier: self.solver_finals_priority_multiplier,
            cancel_flag: None,
        }
    }
}
