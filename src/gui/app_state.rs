use crate::model::{
    Division, Field, FieldKind, Schedule, SchedulingMode, Team, TimeSlot, TournamentConfig,
    Volunteer,
};
use crate::validator::{validate_config, validate_schedule, DiagnosticMessage};
use crate::scheduler::AssignmentConflict;
use super::{Tab, ScheduleViewTab, SolverMessage};
use std::collections::{HashMap, HashSet};
use rand::Rng;
use eframe::egui;

pub struct CsvImportData {
    pub raw_content: String,
    pub divisions: Vec<String>,
    pub selected_divisions: HashSet<String>,
}

pub struct AppState {
    pub current_file_path: Option<std::path::PathBuf>,
    pub config: TournamentConfig,
    pub schedule: Option<Schedule>,
    pub assignment_conflicts: HashMap<usize, Vec<AssignmentConflict>>,
    pub schedule_conflicts: Vec<String>,
    pub division_rounds: HashMap<String, Vec<crate::scheduler::RoundRow>>,
    pub active_tab: Tab,
    pub diagnostics: Vec<DiagnosticMessage>,
    pub solver_rx: Option<std::sync::mpsc::Receiver<SolverMessage>>,
    
    // CSV Import state
    pub csv_import: Option<CsvImportData>,

    // Divisions temp fields
    pub new_div_name: String,
    pub new_div_mode: SchedulingMode,
    pub new_div_games: usize,
    pub new_div_duration: u32,
    pub new_div_volunteers: usize,
    pub new_div_interviews: bool,
    pub new_div_int_vols: usize,
    pub new_div_int_dur: u32,
    pub new_div_finals_enabled: bool,
    pub new_div_finals_rounds: crate::model::FinalsRounds,
    pub new_div_custom_finals_duration: bool,
    pub new_div_finals_duration: u32,
    pub new_div_finals_third_place_playoff: bool,
    pub new_div_color: [u8; 3],

    // Teams temp fields
    pub new_team_name: String,
    pub new_team_div_id: String,
    pub new_team_org: String,

    // Fields temp fields
    pub new_field_name: String,
    pub new_table_name: String,

    // Intelligent generator helper
    pub gen_slot_duration: u32,
    pub gen_interview_slot_duration: u32,
    pub gen_match_slot_break: u32,
    pub gen_interview_slot_break: u32,

    // Volunteers temp fields
    pub new_vol_name: String,
    pub new_vol_caps: Vec<String>,
    pub new_vol_conflicts_list: Vec<String>,

    // Solver state
    pub solver_iterations: usize,
    pub solver_restarts: usize,
    pub solver_fairness_mode: crate::model::FairnessMode,
    pub solver_vol_consecutive_weight: f64,
    pub solver_team_back_to_back_weight: f64,
    pub solver_field_variety_weight: f64,
    pub solver_field_balance_weight: f64,
    pub solver_vol_capability_weight: f64,
    pub solver_interview_late_weight: f64,
    pub solver_interview_match_gap_weight: f64,
    pub solver_vol_specialist_mode: crate::model::SpecialistMode,
    pub solver_team_wait_time_weight: f64,
    pub solver_field_variety_strict: bool,
    pub solver_vol_travel_weight: f64,
    pub solver_round_order_weight: f64,
    pub solver_vol_daily_shift_cap: usize,
    pub solver_peak_period_weight: f64,
    pub solver_finals_priority_multiplier: f64,
    pub solve_message: String,
    pub solve_status: String,
    pub solve_progress: f32,
    pub solver_max_iter_reported: usize,
    pub solver_current_restart_idx: usize,
    pub solver_restarts_progress: Vec<usize>,
    pub solver_cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    
    pub status_message: String,
    pub active_vol_day: String,
    pub schedule_view_tab: ScheduleViewTab,

    // Manual Editing state
    pub schedule_locked: bool,
    pub dragged_assignment: Option<usize>,
    pub drag_accumulated_offset: egui::Vec2,
}

impl Default for AppState {
    fn default() -> Self {
        let mut state = Self {
            current_file_path: None,
            config: TournamentConfig::default(),
            schedule: None,
            assignment_conflicts: HashMap::new(),
            schedule_conflicts: Vec::new(),
            division_rounds: HashMap::new(),
            active_tab: Tab::Dashboard,
            diagnostics: Vec::new(),
            solver_rx: None,
            csv_import: None,

            new_div_name: String::new(),
            new_div_mode: SchedulingMode::HeadToHead,
            new_div_games: 5,
            new_div_duration: 20,
            new_div_volunteers: 2,
            new_div_interviews: true,
            new_div_int_vols: 2,
            new_div_int_dur: 15,
            new_div_finals_enabled: false,
            new_div_finals_rounds: crate::model::FinalsRounds::Grand,
            new_div_custom_finals_duration: false,
            new_div_finals_duration: 20,
            new_div_finals_third_place_playoff: false,
            new_div_color: [
                rand::thread_rng().gen_range(50..=200),
                rand::thread_rng().gen_range(50..=200),
                rand::thread_rng().gen_range(50..=200),
            ],

            new_team_name: String::new(),
            new_team_div_id: String::new(),
            new_team_org: String::new(),

            new_field_name: String::new(),
            new_table_name: String::new(),

            gen_slot_duration: 20,
            gen_interview_slot_duration: 10,
            gen_match_slot_break: 5,
            gen_interview_slot_break: 5,

            new_vol_name: String::new(),
            new_vol_caps: Vec::new(),
            new_vol_conflicts_list: Vec::new(),

            solver_iterations: 50000,
            solver_restarts: 5,
            solver_fairness_mode: crate::model::FairnessMode::Balanced,
            solver_vol_consecutive_weight: 1.0,
            solver_team_back_to_back_weight: 1.0,
            solver_field_variety_weight: 0.5,
            solver_field_balance_weight: 1.5,
            solver_vol_capability_weight: 0.5,
            solver_interview_late_weight: 0.5,
            solver_interview_match_gap_weight: 1.0,
            solver_vol_specialist_mode: crate::model::SpecialistMode::Off,
            solver_team_wait_time_weight: 0.3,
            solver_field_variety_strict: false,
            solver_vol_travel_weight: 0.3,
            solver_round_order_weight: 5.0,
            solver_vol_daily_shift_cap: 0,
            solver_peak_period_weight: 0.1,
            solver_finals_priority_multiplier: 2.0,

            solve_message: String::new(),
            solve_status: "Unsolved".to_string(),
            solve_progress: 0.0,
            solver_max_iter_reported: 0,
            solver_current_restart_idx: 0,
            solver_restarts_progress: Vec::new(),
            solver_cancel_flag: None,
            status_message: String::new(),
            active_vol_day: String::new(),
            schedule_view_tab: ScheduleViewTab::AllGames,
            schedule_locked: true,
            dragged_assignment: None,
            drag_accumulated_offset: egui::Vec2::ZERO,
        };

        state.update_diagnostics();
        state
    }
}

impl AppState {
    pub fn clear_schedule(&mut self) {
        self.schedule = None;
        self.assignment_conflicts.clear();
        self.schedule_conflicts.clear();
        self.division_rounds.clear();
    }

    pub fn update_diagnostics(&mut self) {
        let mut diagnostics = validate_config(&self.config);
        if let Some(ref sched) = self.schedule {
            diagnostics.extend(validate_schedule(&self.config, sched));
        }
        
        // Re-sort: Error > Warning > Info
        diagnostics.sort_by_key(|d| match d.severity {
            crate::validator::DiagnosticSeverity::Error => 0,
            crate::validator::DiagnosticSeverity::Warning => 1,
            crate::validator::DiagnosticSeverity::Info => 2,
        });

        self.diagnostics = diagnostics;
    }

    pub fn re_evaluate_schedule(&mut self) {
        if let Some(ref sched) = self.schedule {
            let params = self.get_solver_params();
            let (_hard, soft) = crate::scheduler::evaluate_schedule_cost(&self.config, sched, &params);
            let conflicts = crate::scheduler::get_schedule_conflicts(&self.config, sched);
            let assignment_conflicts = crate::scheduler::get_assignment_conflicts(&self.config, sched);
            let division_rounds = crate::scheduler::get_division_rounds(&self.config, sched);
            let conflicts_count = conflicts.len();
            
            self.assignment_conflicts = assignment_conflicts;
            self.schedule_conflicts = conflicts;
            self.division_rounds = division_rounds;
            
            self.solve_status = if conflicts_count == 0 {
                "Solved (No Conflicts)".to_string()
            } else {
                format!("Solved ({} Conflicts remaining)", conflicts_count)
            };
            
            self.solve_message = format!(
                "Schedule manually adjusted. Hard Conflicts: {}, Soft Penalties: {}",
                conflicts_count, soft
            );
            
            self.update_diagnostics();
        }
    }

    pub fn load_demo_data(&mut self) {
        self.current_file_path = None;
        self.config = TournamentConfig::default();
        self.config.competition_name = "RoboCup Jr Soccer Workspace".to_string();
        self.config.day_configs = vec![
            crate::model::DayGenConfig {
                day: "Saturday".to_string(),
                start_time: "09:00".to_string(),
                end_time: "17:00".to_string(),
                lunch_enabled: true,
                lunch_start: "12:00".to_string(),
                lunch_duration: 60,
                interviews_enabled: true,
            }
        ];

        // 1. Divisions
        self.config.divisions.push(Division {
            id: "soccer_open".to_string(),
            name: "Soccer Open".to_string(),
            mode: SchedulingMode::HeadToHead,
            games_per_team: 3, // Increased from 1
            volunteers_required: 2,
            duration_minutes: 20,
            allowed_fields: None,
            interviews_enabled: true,
            interview_volunteers_required: 2,
            interview_duration_minutes: 15,
            finals_enabled: true,
            finals_rounds: Some(crate::model::FinalsRounds::Grand),
            finals_duration_minutes: Some(25),
            finals_third_place_playoff: false,
            color: Some([129, 140, 248]),
        });

        self.config.divisions.push(Division {
            id: "simple_simon".to_string(),
            name: "Simple Simon Soccer".to_string(),
            mode: SchedulingMode::HeadToHead,
            games_per_team: 3, // Increased from 1
            volunteers_required: 1,
            duration_minutes: 20,
            allowed_fields: None,
            interviews_enabled: false,
            interview_volunteers_required: 1,
            interview_duration_minutes: 10,
            finals_enabled: false,
            finals_rounds: None,
            finals_duration_minutes: None,
            finals_third_place_playoff: false,
            color: Some([52, 211, 153]),
        });

        // Set default division id for selections
        self.new_team_div_id = "soccer_open".to_string();

        // 2. Teams
        let open_teams = vec![
            ("Light Strikers", "Tech Academy"),
            ("Shadow Bots", "Tech Academy"),
            ("Cyber Kickers", "North High"),
            ("Robo Rangers", "North High"),
            ("Byte Brawlers", "South College"),
            ("Pixel United", "South College"),
        ];
        for (t, org) in open_teams {
            self.config.teams.push(Team {
                name: t.to_string(),
                division_id: "soccer_open".to_string(),
                organization: org.to_string(),
            });
        }

        let simon_teams = vec![
            ("Green Giants", "East School"),
            ("Blue Blasters", "East School"),
            ("Yellow Jackets", "West High"),
            ("Red Radicals", "West High"),
        ];
        for (t, org) in simon_teams {
            self.config.teams.push(Team {
                name: t.to_string(),
                division_id: "simple_simon".to_string(),
                organization: org.to_string(),
            });
        }

        // 3. Fields
        self.config.fields.push(Field {
            id: "soccer_field_1".to_string(),
            name: "Soccer Field 1".to_string(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        });
        self.config.fields.push(Field {
            id: "soccer_field_2".to_string(),
            name: "Soccer Field 2".to_string(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        });
        self.config.fields.push(Field {
            id: "soccer_field_3".to_string(),
            name: "Soccer Field 3".to_string(),
            kind: FieldKind::Competition,
            allowed_divisions: None,
        });
        self.config.fields.push(Field {
            id: "interview_table".to_string(),
            name: "Interview Table A".to_string(),
            kind: FieldKind::Interview,
            allowed_divisions: None,
        });

        // 4. Time Slots
        // Generate 12 slots for Saturday starting at 9:00 AM, 20 min slot duration, 5 min gaps
        let mut start_hour = 9;
        let mut start_minute = 0;
        for i in 1..=12 {
            let end_hour = start_hour + (start_minute + 20) / 60;
            let end_minute = (start_minute + 20) % 60;

            let start_str = format!("{:02}:{:02}", start_hour, start_minute);
            let end_str = format!("{:02}:{:02}", end_hour, end_minute);

            self.config.time_slots.push(TimeSlot {
                id: format!("sat_slot_{}", i),
                day: "Saturday".to_string(),
                start_time: start_str,
                end_time: end_str,
                kind: FieldKind::Competition,
            });

            start_hour = end_hour + (end_minute + 5) / 60;
            start_minute = (end_minute + 5) % 60;
        }

        // 5. Volunteers
        self.config.volunteers.push(Volunteer {
            id: "v_david".to_string(),
            name: "David (Soccer Ref)".to_string(),
            availabilities: (1..=12).map(|i| format!("sat_slot_{}", i)).collect(),
            capabilities: Some(vec!["soccer_open".to_string(), "simple_simon".to_string()]),
            conflict_organizations: vec!["Tech Academy".to_string()],
        });

        self.config.volunteers.push(Volunteer {
            id: "v_sarah".to_string(),
            name: "Sarah (Soccer Ref)".to_string(),
            availabilities: (1..=8).map(|i| format!("sat_slot_{}", i)).collect(),
            capabilities: Some(vec!["soccer_open".to_string(), "simple_simon".to_string()]),
            conflict_organizations: Vec::new(),
        });

        self.config.volunteers.push(Volunteer {
            id: "v_john".to_string(),
            name: "John (Simon Open Ref)".to_string(),
            availabilities: (1..=12).map(|i| format!("sat_slot_{}", i)).collect(),
            capabilities: Some(vec!["soccer_open".to_string()]),
            conflict_organizations: Vec::new(),
        });

        self.config.volunteers.push(Volunteer {
            id: "v_alice".to_string(),
            name: "Alice (Simon Open Ref)".to_string(),
            availabilities: (5..=12).map(|i| format!("sat_slot_{}", i)).collect(),
            capabilities: Some(vec!["simple_simon".to_string()]),
            conflict_organizations: Vec::new(),
        });

        self.config.volunteers.push(Volunteer {
            id: "v_bob".to_string(),
            name: "Bob (General Ref)".to_string(),
            availabilities: (1..=12).map(|i| format!("sat_slot_{}", i)).collect(),
            capabilities: None, // can judge anything
            conflict_organizations: Vec::new(),
        });

        self.config.volunteers.push(Volunteer {
            id: "v_charlie".to_string(),
            name: "Charlie (General Ref)".to_string(),
            availabilities: (1..=10).map(|i| format!("sat_slot_{}", i)).collect(),
            capabilities: None,
            conflict_organizations: Vec::new(),
        });

        self.config.volunteers.push(Volunteer {
            id: "v_emily".to_string(),
            name: "Emily (General Ref)".to_string(),
            availabilities: (1..=12).map(|i| format!("sat_slot_{}", i)).collect(),
            capabilities: None,
            conflict_organizations: Vec::new(),
        });

        self.clear_schedule();
        self.assignment_conflicts = HashMap::new();
        self.solve_status = "Unsolved".to_string();
        self.solve_message = String::new();
        self.status_message = "Demo data loaded successfully!".to_string();
        self.update_diagnostics();
    }
}
