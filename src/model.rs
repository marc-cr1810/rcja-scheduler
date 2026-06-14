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
            attendance_status: Default::default(),
        });

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: TournamentConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.competition_name, config.competition_name);
        assert_eq!(restored.divisions, config.divisions);
        assert_eq!(restored.teams, config.teams);
        assert_eq!(restored.time_slots, config.time_slots);
        assert_eq!(restored.volunteers, config.volunteers);
    }
}
