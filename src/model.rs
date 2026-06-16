use serde::{Deserialize, Serialize};

/// Controls how aggressively the solver balances volunteer shifts
/// relative to each volunteer's availability window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum FairnessMode {
    /// No fairness weighting — pure random selection (original behaviour).
    Off,
    /// Soft bias: prefer under-utilised volunteers via weighted-random sampling.
    /// Penalises imbalance with weight 10.0 in the cost function.
    #[default]
    Balanced,
    /// Strict bias: always pick the least-utilised qualified volunteer(s) first,
    /// with a higher penalty weight of 20.0 for any remaining imbalance.
    Strict,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum SpecialistMode {
    /// Volunteers can be assigned to any division they are qualified for.
    #[default]
    Off,
    /// Soft bias to keep volunteers in the same division.
    Balanced,
    /// Strong bias to avoid switching volunteers between divisions.
    Strict,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulingMode {
    HeadToHead,    // e.g. Soccer: Team A vs Team B
    IndividualRun, // e.g. Rescue/OnStage: Team A runs on arena/stage
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[derive(Default)]
pub enum FinalsRounds {
    #[default]
    Grand,   // Top 2 (1 match)
    Semis,   // Top 4 (3 matches)
    Quarter, // Top 8 (7 matches)
    Eighths, // Top 16 (15 matches)
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Division {
    pub id: String,
    pub name: String,
    pub mode: SchedulingMode,
    pub games_per_team: usize, // number of runs for Rescue, or total matches for Soccer (Head-to-Head)
    pub volunteers_required: usize, // e.g. 2 for standard soccer, 1 for Rescue
    pub duration_minutes: u32,      // default duration of a game/run (e.g. 20 mins)
    pub allowed_fields: Option<Vec<String>>, // None means allowed on all fields
    pub interviews_enabled: bool,
    pub interview_volunteers_required: usize,
    pub interview_duration_minutes: u32,
    #[serde(default)]
    pub finals_enabled: bool,
    #[serde(default)]
    pub finals_rounds: Option<FinalsRounds>,
    #[serde(default)]
    pub finals_duration_minutes: Option<u32>,
    #[serde(default)]
    pub finals_third_place_playoff: bool,
    pub color: Option<[u8; 3]>, // RGB color
    /// Per-division override (minutes) for the minimum recharge break between a
    /// team's consecutive matches. `None` inherits the global solver setting.
    #[serde(default)]
    pub min_match_break_minutes: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
pub enum FieldKind {
    #[default]
    Competition,
    Interview,
}


#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Team {
    pub name: String,
    pub division_id: String, // Each team belongs to exactly one division
    #[serde(default)]
    pub organization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Field {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: FieldKind,
    pub allowed_divisions: Option<Vec<String>>, // None means allowed for all divisions
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimeSlot {
    pub id: String,
    pub day: String,       // e.g. "Saturday"
    pub start_time: String, // "HH:MM" format
    pub end_time: String,   // "HH:MM" format
    #[serde(default)]
    pub kind: FieldKind,
}

impl TimeSlot {
    pub fn duration_minutes(&self) -> u32 {
        let start = parse_time_minutes(&self.start_time).unwrap_or(0);
        let end = parse_time_minutes(&self.end_time).unwrap_or(0);
        end.saturating_sub(start)
    }

    pub fn start_minutes(&self) -> u32 {
        parse_time_minutes(&self.start_time).unwrap_or(0)
    }
}

pub fn parse_time_minutes(time_str: &str) -> Option<u32> {
    let parts: Vec<&str> = time_str.split(':').collect();
    if parts.len() == 2 {
        let hours: u32 = parts[0].parse().ok()?;
        let minutes: u32 = parts[1].parse().ok()?;
        Some(hours * 60 + minutes)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum AttendanceStatus {
    #[default]
    Pending,
    CheckedIn,
    NoShow,
}

use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Volunteer {
    pub id: String,
    pub name: String,
    pub availabilities: Vec<String>, // list of time slot IDs
    pub capabilities: Option<Vec<String>>, // list of division IDs they can judge. None means can judge anything.
    pub conflict_organizations: Vec<String>, // list of organization names they cannot judge
    #[serde(default)]
    pub attendance_status: HashMap<String, AttendanceStatus>,
    /// Field/interview-table IDs this volunteer is locked to. `None` or an empty
    /// list means no restriction; otherwise the volunteer may only be rostered on
    /// an activity placed on one of these fields.
    #[serde(default)]
    pub locked_field_ids: Option<Vec<String>>,
}

impl Volunteer {
    pub fn status_for_day(&self, day: &str) -> AttendanceStatus {
        self.attendance_status.get(day).cloned().unwrap_or_default()
    }
}

/// Identifies what kind of match this is and, for round-robin matches, which
/// cycle/round it belongs to. This replaces the old practice of sniffing the
/// match `id` string (`_gf`, `_sf_`, `_c0_r1`, …) for semantics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MatchStage {
    RoundRobin { cycle: usize, round: usize },
    EighthFinal,
    QuarterFinal,
    SemiFinal,
    ThirdPlace,
    GrandFinal,
}

impl MatchStage {
    /// Numeric ordering used by the solver to enforce stage progression.
    /// RR=0, EF=1, QF=2, SF=3, 3rd-place=4, GF=5.
    pub fn stage_num(&self) -> usize {
        match self {
            MatchStage::RoundRobin { .. } => 0,
            MatchStage::EighthFinal => 1,
            MatchStage::QuarterFinal => 2,
            MatchStage::SemiFinal => 3,
            MatchStage::ThirdPlace => 4,
            MatchStage::GrandFinal => 5,
        }
    }

    pub fn label(&self) -> Option<&'static str> {
        match self {
            MatchStage::RoundRobin { .. } => None,
            MatchStage::EighthFinal => Some("Eighth Finals"),
            MatchStage::QuarterFinal => Some("Quarter Finals"),
            MatchStage::SemiFinal => Some("Semi Finals"),
            MatchStage::ThirdPlace => Some("3rd Place Playoff"),
            MatchStage::GrandFinal => Some("Grand Final"),
        }
    }

    pub fn is_final(&self) -> bool {
        !matches!(self, MatchStage::RoundRobin { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Activity {
    Match {
        id: String,
        team_a: String,
        team_b: String,
        division_id: String,
        duration_minutes: u32,
        stage: MatchStage,
    },
    Run {
        id: String,
        team: String,
        division_id: String,
        run_number: usize,
        duration_minutes: u32,
    },
    Interview {
        id: String,
        team: String,
        division_id: String,
        duration_minutes: u32,
    },
}

impl Activity {
    #[allow(dead_code)]
    pub fn id(&self) -> &str {
        match self {
            Activity::Match { id, .. } => id,
            Activity::Run { id, .. } => id,
            Activity::Interview { id, .. } => id,
        }
    }

    pub fn division_id(&self) -> &str {
        match self {
            Activity::Match { division_id, .. } => division_id,
            Activity::Run { division_id, .. } => division_id,
            Activity::Interview { division_id, .. } => division_id,
        }
    }

    pub fn duration_minutes(&self) -> u32 {
        match self {
            Activity::Match { duration_minutes, .. } => *duration_minutes,
            Activity::Run { duration_minutes, .. } => *duration_minutes,
            Activity::Interview { duration_minutes, .. } => *duration_minutes,
        }
    }

    pub fn teams(&self) -> Vec<&str> {
        match self {
            Activity::Match { team_a, team_b, .. } => vec![team_a, team_b],
            Activity::Run { team, .. } => vec![team],
            Activity::Interview { team, .. } => vec![team],
        }
    }

    pub fn is_final(&self) -> bool {
        matches!(self, Activity::Match { stage, .. } if stage.is_final())
    }

    pub fn label(&self) -> String {
        let clean_name = |name: &str, div_id: &str| -> String {
            let prefix = format!("{} ", div_id);
            if name.starts_with(&prefix) {
                name[prefix.len()..].to_string()
            } else {
                name.to_string()
            }
        };

        match self {
            Activity::Match { team_a, team_b, stage, division_id, .. } => {
                let clean_a = clean_name(team_a, division_id);
                let clean_b = clean_name(team_b, division_id);
                if stage.is_final() {
                    format!("🏆 {} vs {}", clean_a, clean_b)
                } else {
                    format!("⚽ {} vs {}", clean_a, clean_b)
                }
            }
            Activity::Run { team, run_number, division_id, .. } => {
                let clean_t = clean_name(team, division_id);
                format!("🤖 {} Run #{}", clean_t, run_number)
            }
            Activity::Interview { team, division_id, .. } => {
                let clean_t = clean_name(team, division_id);
                format!("💬 {} Interview", clean_t)
            }
        }
    }

    pub fn export_label(&self) -> String {
        let clean_name = |name: &str, div_id: &str| -> String {
            let prefix = format!("{} ", div_id);
            if name.starts_with(&prefix) {
                name[prefix.len()..].to_string()
            } else {
                name.to_string()
            }
        };

        match self {
            Activity::Match { team_a, team_b, stage, division_id, .. } => {
                let clean_a = clean_name(team_a, division_id);
                let clean_b = clean_name(team_b, division_id);
                if stage.is_final() {
                    format!("[FINAL] {} vs {}", clean_a, clean_b)
                } else {
                    format!("{} vs {}", clean_a, clean_b)
                }
            }
            Activity::Run { team, run_number, division_id, .. } => {
                let clean_t = clean_name(team, division_id);
                format!("{} Run #{}", clean_t, run_number)
            }
            Activity::Interview { team, division_id, .. } => {
                let clean_t = clean_name(team, division_id);
                format!("{} Interview", clean_t)
            }
        }
    }

    pub fn stage(&self) -> usize {
        match self {
            Activity::Match { stage, .. } => stage.stage_num(),
            _ => 0,
        }
    }

    pub fn stage_label(&self) -> Option<&'static str> {
        match self {
            Activity::Match { stage, .. } => stage.label(),
            _ => None,
        }
    }

    pub fn round_label(&self) -> String {
        match self {
            Activity::Match { stage: MatchStage::RoundRobin { .. }, .. } => {
                format!("Round {}", self.round_index() + 1)
            }
            Activity::Run { run_number, .. } => {
                format!("Run #{}", run_number)
            }
            _ => "".to_string(),
        }
    }

    pub fn round_index(&self) -> usize {
        match self {
            Activity::Match { stage: MatchStage::RoundRobin { cycle, round }, .. } => {
                cycle * 100 + round
            }
            Activity::Match { stage, .. } => {
                // Finals stages: EF=1, QF=2, SF=3, 3PL=4, GF=5.
                // Offset by a large number so they sort after any RR cycles.
                10000 + stage.stage_num() * 100
            }
            Activity::Run { run_number, .. } => *run_number,
            Activity::Interview { .. } => 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleAssignment {
    pub activity: Activity,
    pub time_slot_id: String,
    pub field_id: Option<String>,
    pub volunteer_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Schedule {
    pub assignments: Vec<ScheduleAssignment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DayGenConfig {
    pub day: String,
    pub start_time: String,
    pub end_time: String,
    pub lunch_enabled: bool,
    pub lunch_start: String,
    pub lunch_duration: u32,
    #[serde(default = "default_true")]
    pub interviews_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for DayGenConfig {
    fn default() -> Self {
        Self {
            day: "Saturday".to_string(),
            start_time: "09:00".to_string(),
            end_time: "17:00".to_string(),
            lunch_enabled: true,
            lunch_start: "12:00".to_string(),
            lunch_duration: 60,
            interviews_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverSettings {
    pub fairness_mode: FairnessMode,
    pub iterations: usize,
    pub restarts: usize,
    pub vol_consecutive_weight: f64,
    pub team_back_to_back_weight: f64,
    pub field_variety_weight: f64,
    pub field_balance_weight: f64,
    pub vol_capability_weight: f64,
    pub interview_late_weight: f64,
    pub interview_match_gap_weight: f64,
    pub team_min_break_minutes: u32,
    pub team_break_buffer_minutes: u32,
    pub team_match_min_break_minutes: u32,
    pub team_match_break_buffer_minutes: u32,
    pub vol_specialist_mode: SpecialistMode,
    pub team_wait_time_weight: f64,
    pub field_variety_strict: bool,
    pub vol_travel_weight: f64,
    pub round_order_weight: f64,
    pub vol_daily_shift_cap: usize,
    pub peak_period_weight: f64,
    pub finals_priority_multiplier: f64,
    /// When true, the solver seeds from `seed` for a fully reproducible result.
    /// When false, every generation draws a fresh random seed.
    #[serde(default)]
    pub use_seed: bool,
    /// Fixed RNG seed, used only when `use_seed` is true.
    #[serde(default)]
    pub seed: u64,
    // Time slot generator settings
    pub gen_slot_duration: u32,
    pub gen_interview_slot_duration: u32,
    pub gen_match_slot_break: u32,
    pub gen_interview_slot_break: u32,
}

impl Default for SolverSettings {
    fn default() -> Self {
        Self {
            fairness_mode: FairnessMode::Balanced,
            iterations: 50000,
            restarts: 5,
            vol_consecutive_weight: 1.0,
            team_back_to_back_weight: 1.0,
            field_variety_weight: 0.5,
            field_balance_weight: 1.5,
            vol_capability_weight: 0.5,
            interview_late_weight: 0.5,
            interview_match_gap_weight: 1.0,
            team_min_break_minutes: 10,
            team_break_buffer_minutes: 30,
            team_match_min_break_minutes: 10,
            team_match_break_buffer_minutes: 20,
            vol_specialist_mode: SpecialistMode::Off,
            team_wait_time_weight: 0.3,
            field_variety_strict: false,
            vol_travel_weight: 0.3,
            round_order_weight: 5.0,
            vol_daily_shift_cap: 0,
            // Even spread is a primary objective. 1.0 measured best on the real
            // config (dispersion CoV and overall soft cost both lowest) now that
            // the round-window banding no longer pre-clusters rounds; the old 0.1
            // barely registered against the other soft terms.
            peak_period_weight: 1.0,
            finals_priority_multiplier: 2.0,
            use_seed: false,
            seed: 0,
            gen_slot_duration: 20,
            gen_interview_slot_duration: 10,
            gen_match_slot_break: 5,
            gen_interview_slot_break: 5,
        }
    }
}

impl PartialEq for SolverSettings {
    fn eq(&self, other: &Self) -> bool {
        self.fairness_mode == other.fairness_mode
            && self.iterations == other.iterations
            && self.restarts == other.restarts
            && self.vol_consecutive_weight == other.vol_consecutive_weight
            && self.team_back_to_back_weight == other.team_back_to_back_weight
            && self.field_variety_weight == other.field_variety_weight
            && self.field_balance_weight == other.field_balance_weight
            && self.vol_capability_weight == other.vol_capability_weight
            && self.interview_late_weight == other.interview_late_weight
            && self.interview_match_gap_weight == other.interview_match_gap_weight
            && self.team_min_break_minutes == other.team_min_break_minutes
            && self.team_break_buffer_minutes == other.team_break_buffer_minutes
            && self.team_match_min_break_minutes == other.team_match_min_break_minutes
            && self.team_match_break_buffer_minutes == other.team_match_break_buffer_minutes
            && self.vol_specialist_mode == other.vol_specialist_mode
            && self.team_wait_time_weight == other.team_wait_time_weight
            && self.field_variety_strict == other.field_variety_strict
            && self.vol_travel_weight == other.vol_travel_weight
            && self.round_order_weight == other.round_order_weight
            && self.vol_daily_shift_cap == other.vol_daily_shift_cap
            && self.peak_period_weight == other.peak_period_weight
            && self.finals_priority_multiplier == other.finals_priority_multiplier
            && self.use_seed == other.use_seed
            && self.seed == other.seed
            && self.gen_slot_duration == other.gen_slot_duration
            && self.gen_interview_slot_duration == other.gen_interview_slot_duration
            && self.gen_match_slot_break == other.gen_match_slot_break
            && self.gen_interview_slot_break == other.gen_interview_slot_break
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TournamentConfig {
    pub competition_name: String,
    pub divisions: Vec<Division>,
    pub teams: Vec<Team>,
    pub fields: Vec<Field>,
    pub time_slots: Vec<TimeSlot>,
    pub volunteers: Vec<Volunteer>,
    pub strict_capabilities: bool,
    /// Legacy field — kept for backward-compatible deserialization of old configs.
    /// New configs store fairness_mode inside `solver_settings`.
    #[serde(default)]
    pub fairness_mode: FairnessMode,
    #[serde(default)]
    pub day_configs: Vec<DayGenConfig>,
    #[serde(default)]
    pub solver_settings: SolverSettings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_robin_stage_round_index() {
        let a = Activity::Match {
            id: "div1_m_3_c2_r1".into(),
            team_a: "A".into(),
            team_b: "B".into(),
            division_id: "div1".into(),
            duration_minutes: 20,
            stage: MatchStage::RoundRobin { cycle: 2, round: 1 },
        };
        assert_eq!(a.stage(), 0);
        assert!(!a.is_final());
        // round_index = cycle * 100 + round
        assert_eq!(a.round_index(), 201);
        assert_eq!(a.stage_label(), None);
    }

    #[test]
    fn finals_stage_ordering_and_labels() {
        let mk = |stage: MatchStage| Activity::Match {
            id: "x".into(), team_a: "A".into(), team_b: "B".into(),
            division_id: "d".into(), duration_minutes: 20, stage,
        };
        // Numeric stage ordering EF<QF<SF<3PL<GF, all distinct from RR(0).
        assert_eq!(mk(MatchStage::EighthFinal).stage(), 1);
        assert_eq!(mk(MatchStage::QuarterFinal).stage(), 2);
        assert_eq!(mk(MatchStage::SemiFinal).stage(), 3);
        assert_eq!(mk(MatchStage::ThirdPlace).stage(), 4);
        assert_eq!(mk(MatchStage::GrandFinal).stage(), 5);

        // Finals sort after any round-robin cycle.
        assert!(mk(MatchStage::EighthFinal).round_index() > 10_000);
        assert!(mk(MatchStage::GrandFinal).round_index() > mk(MatchStage::SemiFinal).round_index());

        assert_eq!(mk(MatchStage::GrandFinal).stage_label(), Some("Grand Final"));
        assert!(mk(MatchStage::GrandFinal).is_final());

        // A team named with a finals-looking substring must NOT be misclassified
        // (the old id-substring detection would have broken on this).
        let tricky = Activity::Match {
            id: "div_gf_team".into(), team_a: "Team _sf_ FC".into(), team_b: "B".into(),
            division_id: "d".into(), duration_minutes: 20,
            stage: MatchStage::RoundRobin { cycle: 0, round: 0 },
        };
        assert_eq!(tricky.stage(), 0);
        assert!(!tricky.is_final());
    }

    #[test]
    fn config_json_round_trip() {
        let mut config = TournamentConfig {
            competition_name: "Test Cup".into(),
            ..Default::default()
        };
        config.divisions.push(Division {
            id: "d1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead,
            games_per_team: 2, volunteers_required: 2, duration_minutes: 20,
            allowed_fields: None, interviews_enabled: true, interview_volunteers_required: 1,
            interview_duration_minutes: 10, finals_enabled: true,
            finals_rounds: Some(FinalsRounds::Semis), finals_duration_minutes: Some(25),
            finals_third_place_playoff: true, color: Some([10, 20, 30]), min_match_break_minutes: None,
        });
        config.teams.push(Team { name: "Alpha".into(), division_id: "d1".into(), organization: "Org".into() });
        config.time_slots.push(TimeSlot {
            id: "s1".into(), day: "Saturday".into(), start_time: "09:00".into(),
            end_time: "09:20".into(), kind: FieldKind::Competition,
        });
        config.volunteers.push(Volunteer {
            id: "v1".into(), name: "Vol".into(), availabilities: vec!["s1".into()],
            capabilities: Some(vec!["d1".into()]), conflict_organizations: vec![],
            attendance_status: Default::default(), locked_field_ids: None,
        });

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: TournamentConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.competition_name, config.competition_name);
        assert_eq!(restored.divisions, config.divisions);
        assert_eq!(restored.teams, config.teams);
        assert_eq!(restored.time_slots, config.time_slots);
        assert_eq!(restored.volunteers, config.volunteers);
        assert_eq!(restored.solver_settings, config.solver_settings);
    }

    #[test]
    fn solver_settings_round_trip() {
        let mut config = TournamentConfig::default();
        config.solver_settings = SolverSettings {
            fairness_mode: FairnessMode::Strict,
            iterations: 30000,
            restarts: 8,
            vol_consecutive_weight: 2.5,
            team_back_to_back_weight: 0.7,
            field_variety_weight: 1.2,
            field_balance_weight: 3.0,
            vol_capability_weight: 0.1,
            interview_late_weight: 2.0,
            interview_match_gap_weight: 0.8,
            team_min_break_minutes: 15,
            team_break_buffer_minutes: 45,
            team_match_min_break_minutes: 20,
            team_match_break_buffer_minutes: 35,
            vol_specialist_mode: SpecialistMode::Strict,
            team_wait_time_weight: 0.9,
            field_variety_strict: true,
            vol_travel_weight: 1.5,
            round_order_weight: 8.0,
            vol_daily_shift_cap: 6,
            peak_period_weight: 0.5,
            finals_priority_multiplier: 3.0,
            use_seed: true,
            seed: 123456789,
            gen_slot_duration: 15,
            gen_interview_slot_duration: 8,
            gen_match_slot_break: 3,
            gen_interview_slot_break: 2,
        };

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: TournamentConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.solver_settings, config.solver_settings);
    }

    #[test]
    fn legacy_config_without_solver_settings_loads_defaults() {
        // Simulates loading a JSON file saved before solver_settings existed
        let json = r#"{
            "competition_name": "Old Config",
            "divisions": [],
            "teams": [],
            "fields": [],
            "time_slots": [],
            "volunteers": [],
            "strict_capabilities": false,
            "fairness_mode": "Strict",
            "day_configs": []
        }"#;
        let config: TournamentConfig = serde_json::from_str(json).expect("deserialize legacy");
        assert_eq!(config.competition_name, "Old Config");
        // solver_settings should get defaults
        assert_eq!(config.solver_settings, SolverSettings::default());
        // legacy fairness_mode should still parse
        assert_eq!(config.fairness_mode, FairnessMode::Strict);
    }
}
