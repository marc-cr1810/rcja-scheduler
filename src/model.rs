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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Volunteer {
    pub id: String,
    pub name: String,
    pub availabilities: Vec<String>, // list of time slot IDs
    pub capabilities: Option<Vec<String>>, // list of division IDs they can judge. None means can judge anything.
    pub conflict_organizations: Vec<String>, // list of organization names they cannot judge
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Activity {
    Match {
        id: String,
        team_a: String,
        team_b: String,
        division_id: String,
        duration_minutes: u32,
        is_final: bool,
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
            Activity::Match { team_a, team_b, is_final, division_id, .. } => {
                let clean_a = clean_name(team_a, division_id);
                let clean_b = clean_name(team_b, division_id);
                if *is_final {
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
            Activity::Match { team_a, team_b, is_final, division_id, .. } => {
                let clean_a = clean_name(team_a, division_id);
                let clean_b = clean_name(team_b, division_id);
                if *is_final {
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
            Activity::Match { id, .. } => {
                if id.contains("_ef_") {
                    1
                } else if id.contains("_qf_") {
                    2
                } else if id.contains("_sf_") {
                    3
                } else if id.contains("_3pl") {
                    4
                } else if id.contains("_gf")  {
                    5
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    pub fn stage_label(&self) -> Option<&'static str> {
        match self {
            Activity::Match { id, .. } => {
                if id.contains("_ef_") { return Some("Eighth Finals"); }
                if id.contains("_qf_") { return Some("Quarter Finals"); }
                if id.contains("_sf_") { return Some("Semi Finals"); }
                if id.contains("_3pl") { return Some("3rd Place Playoff"); }
                if id.contains("_gf")  { return Some("Grand Final"); }
                None
            }
            _ => None,
        }
    }

    pub fn round_label(&self) -> String {
        match self {
            Activity::Match { is_final, .. } if !*is_final => {
                // RR match
                format!("Round {}", self.round_index() + 1)
            }
            Activity::Run { run_number, .. } => {
                format!("Run #{}", run_number)
            }
            _ => "".to_string(),
        }
    }

    pub fn round_index(&self) -> usize {
        let stage = self.stage();
        if stage > 0 {
            // Finals stages: EF=1, QF=2, SF=3, 3PL=4, GF=5
            // Offset by a large number to ensure they come after any RR cycles
            return 10000 + stage * 100;
        }

        match self {
            Activity::Match { id, .. } => {
                // Expected pattern: something_c{cycle}_r{round}
                if let Some(c_pos) = id.rfind("_c") {
                    let after_c = &id[c_pos + 2..];
                    if let Some(r_pos) = after_c.find("_r") {
                        let round_str = &after_c[r_pos + 2..];
                        if let Ok(round) = round_str.parse::<usize>() {
                            if let Ok(cycle) = after_c[..r_pos].parse::<usize>() {
                                return cycle * 100 + round;
                            }
                            return round;
                        }
                    }
                }
                0
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TournamentConfig {
    pub competition_name: String,
    pub divisions: Vec<Division>,
    pub teams: Vec<Team>,
    pub fields: Vec<Field>,
    pub time_slots: Vec<TimeSlot>,
    pub volunteers: Vec<Volunteer>,
    pub strict_capabilities: bool,
    #[serde(default)]
    pub fairness_mode: FairnessMode,
    #[serde(default)]
    pub day_configs: Vec<DayGenConfig>,
}
