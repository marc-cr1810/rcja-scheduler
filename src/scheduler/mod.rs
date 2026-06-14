pub mod activity;
mod evaluator;
mod solver;
pub mod utils;
mod internal;
mod fast_evaluator;
mod conflicts;
#[cfg(test)]
mod bench;

use crate::model::{FairnessMode, SpecialistMode, TournamentConfig};
use std::collections::{HashMap, HashSet};

#[allow(unused_imports)]
pub use solver::solve_schedule;
#[allow(unused_imports)]
pub use evaluator::{evaluate_schedule_cost, get_schedule_conflicts, get_occupied_slots, get_assignment_conflicts, AssignmentConflict, ConflictSeverity};
pub use activity::generate_activities;
pub use utils::{sanitize_name, is_field_suitable_for_activity};

/// Solver parameters
#[derive(Debug, Clone)]
pub struct SolverParams {
    pub max_iterations: usize,
    pub num_restarts: usize,
    pub fairness_mode: FairnessMode,
    /// Soft penalty weight for volunteers assigned to consecutive time slots.
    /// Set to 0.0 to disable. Default 1.0 (same weight as team back-to-back).
    pub vol_consecutive_weight: f64,
    pub team_back_to_back_weight: f64,
    pub field_variety_weight: f64,
    pub field_balance_weight: f64,
    pub vol_capability_weight: f64,
    /// Penalises scheduling interviews late in the day.
    pub interview_late_weight: f64,
    /// Soft penalty weight applied as a team's interview→match gap shrinks below
    /// `team_break_buffer_minutes`. Scaled by how far under the target the gap is.
    pub interview_match_gap_weight: f64,
    /// Hard minimum break (in minutes) required between a team's interview and a
    /// match. Any closer pairing is a hard conflict. 0 = disabled.
    pub team_min_break_minutes: u32,
    /// Soft "comfortable" target gap (in minutes) between a team's interview and a
    /// match. Gaps below this are softly penalised via `interview_match_gap_weight`.
    pub team_break_buffer_minutes: u32,
    /// Global hard minimum break (in minutes) between a team's consecutive
    /// matches (robot recharge). A division may override this. 0 = disabled.
    pub team_match_min_break_minutes: u32,
    /// Soft "comfortable" target gap (in minutes) between a team's consecutive
    /// matches. Gaps below this are softly penalised via `team_back_to_back_weight`.
    pub team_match_break_buffer_minutes: u32,
    /// Soft penalty for assigning a volunteer to multiple different divisions.
    pub vol_specialist_mode: SpecialistMode,
    /// Penalises long gaps between a team's matches on the same day.
    pub team_wait_time_weight: f64,
    /// If true, playing on the same field twice is a hard conflict.
    pub field_variety_strict: bool,
    /// Penalises moving a volunteer between different fields/locations.
    pub vol_travel_weight: f64,
    /// Penalises scheduling a higher round before a lower round.
    pub round_order_weight: f64,
    /// Hard limit on shifts per day for a single volunteer. 0 = no limit.
    pub vol_daily_shift_cap: usize,
    /// Encourages even distribution of activities across the day.
    pub peak_period_weight: f64,
    /// Multiplier for penalties/conflicts involving final matches.
    pub finals_priority_multiplier: f64,
    /// Optional flag to signal cancellation to the solver thread.
    pub cancel_flag: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Optional RNG seed. When `Some`, the solve is fully reproducible: each
    /// restart is seeded deterministically from this value and the best-result
    /// selection breaks ties by restart index, so the same config + params
    /// always yields the same schedule. When `None`, the solver seeds from
    /// system entropy (different result each run).
    pub seed: Option<u64>,
}

impl Default for SolverParams {
    fn default() -> Self {
        Self {
            max_iterations: 50000,
            num_restarts: 5,
            fairness_mode: FairnessMode::Balanced,
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
            peak_period_weight: 10.0,
            finals_priority_multiplier: 2.0,
            cancel_flag: None,
            seed: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoundMatch {
    pub team_a: String,
    pub team_b: String,
    /// Scheduled start time, e.g. "09:20". Empty if not yet scheduled.
    pub time: String,
    /// Scheduled day, e.g. "Saturday".
    pub day: String,
    /// Human-readable field name, e.g. "Soccer Field 1".
    pub field_name: String,
    pub is_final: bool,
}

/// One logical round (or finals stage) for a division.
#[derive(Debug, Clone)]
pub struct RoundRow {
    /// Display label: "Round 1", "Semi Final", "Grand Final", etc.
    pub round_label: String,
    /// Matches scheduled in this round (sorted by time).
    pub matches: Vec<RoundMatch>,
    /// Teams from this division that have no match this round (byes).
    pub bye_teams: Vec<String>,
}

/// Builds a round-by-round view for every division given the solved schedule.
pub fn get_division_rounds(
    config: &TournamentConfig,
    schedule: &crate::model::Schedule,
) -> HashMap<String, Vec<RoundRow>> {
    use crate::model::{Activity, SchedulingMode, parse_time_minutes};

    let mut result: HashMap<String, Vec<RoundRow>> = HashMap::new();

    let mut day_order = HashMap::new();
    for (i, day_config) in config.day_configs.iter().enumerate() {
        day_order.insert(day_config.day.to_lowercase(), i);
    }

    // Fallback for days not in day_configs: use order of appearance in time_slots
    if day_order.is_empty() {
        for slot in &config.time_slots {
            let day = slot.day.to_lowercase();
            if !day_order.contains_key(&day) {
                let idx = day_order.len();
                day_order.insert(day, idx);
            }
        }
    }

    for div in &config.divisions {
        let div_teams: Vec<String> = config.teams
            .iter()
            .filter(|t| t.division_id == div.id)
            .map(|t| t.name.clone())
            .collect();

        // Collect all scheduled assignments for this division (excluding interviews)
        let div_assignments: Vec<&crate::model::ScheduleAssignment> = schedule.assignments
            .iter()
            .filter(|a| {
                a.activity.division_id() == div.id
                    && !matches!(a.activity, Activity::Interview { .. })
            })
            .collect();

        match div.mode {
            SchedulingMode::HeadToHead => {
                // Separate round-robin matches from finals
                let mut rr_rounds: HashMap<usize, Vec<&crate::model::ScheduleAssignment>> = HashMap::new();
                let mut finals_stages: std::collections::BTreeMap<u8, Vec<&crate::model::ScheduleAssignment>> = std::collections::BTreeMap::new();

                for assign in &div_assignments {
                    let stage = assign.activity.stage();
                    if stage > 0 {
                        // Finals match
                        finals_stages.entry(stage as u8).or_default().push(assign);
                    } else if let Activity::Match { stage: crate::model::MatchStage::RoundRobin { cycle, round }, .. } = &assign.activity {
                        // n_teams - 1 rounds per cycle (using padded team count for odd divisions)
                        let n_padded = if div_teams.len().is_multiple_of(2) { div_teams.len() } else { div_teams.len() + 1 };
                        let rounds_per_cycle = n_padded.saturating_sub(1).max(1);
                        let global_round = cycle * rounds_per_cycle + round;
                        rr_rounds.entry(global_round).or_default().push(assign);
                    }
                }

                let mut rows = Vec::new();

                // Process Round Robin rounds
                let mut sorted_rr: Vec<_> = rr_rounds.keys().collect();
                sorted_rr.sort_unstable();
                for &round_idx in sorted_rr {
                    let assignments = rr_rounds.get(&round_idx).unwrap();
                    let mut matches = Vec::new();
                    let mut round_teams = HashSet::new();

                    for a in assignments {
                        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                        let field = a.field_id.as_ref().and_then(|fid| config.fields.iter().find(|f| f.id == *fid));
                        
                        if let Activity::Match { team_a, team_b, .. } = &a.activity {
                            matches.push(RoundMatch {
                                team_a: team_a.clone(),
                                team_b: team_b.clone(),
                                time: slot.map_or("".to_string(), |s| s.start_time.clone()),
                                day: slot.map_or("".to_string(), |s| s.day.clone()),
                                field_name: field.map_or("—".to_string(), |f| f.name.clone()),
                                is_final: false,
                            });
                            round_teams.insert(team_a.clone());
                            round_teams.insert(team_b.clone());
                        }
                    }

                    // Sort matches by day then time
                    matches.sort_by_key(|m| {
                        let d_idx = day_order.get(&m.day.to_lowercase()).copied().unwrap_or(99);
                        let t_min = parse_time_minutes(&m.time).unwrap_or(0);
                        (d_idx, t_min)
                    });

                    let bye_teams: Vec<String> = div_teams.iter()
                        .filter(|t| !round_teams.contains(*t))
                        .cloned()
                        .collect();

                    rows.push(RoundRow {
                        round_label: format!("Round {}", round_idx + 1),
                        matches,
                        bye_teams,
                    });
                }

                // Process Finals
                for (_, assignments) in finals_stages {
                    let mut stage_matches = Vec::new();
                    let mut label = "Finals";

                    let clean_name = |name: &str, div_id: &str| -> String {
                        let prefix = format!("{} ", div_id);
                        if name.starts_with(&prefix) {
                            name[prefix.len()..].to_string()
                        } else {
                            name.to_string()
                        }
                    };

                    for a in assignments {
                        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                        let field = a.field_id.as_ref().and_then(|fid| config.fields.iter().find(|f| f.id == *fid));
                        
                        if let Activity::Match { team_a, team_b, division_id, .. } = &a.activity {
                            if let Some(l) = a.activity.stage_label() {
                                label = l;
                            }
                            stage_matches.push(RoundMatch {
                                team_a: clean_name(team_a, division_id),
                                team_b: clean_name(team_b, division_id),
                                time: slot.map_or("".to_string(), |s| s.start_time.clone()),
                                day: slot.map_or("".to_string(), |s| s.day.clone()),
                                field_name: field.map_or("—".to_string(), |f| f.name.clone()),
                                is_final: true,
                            });
                        }
                    }

                    stage_matches.sort_by_key(|m| {
                        let d_idx = day_order.get(&m.day.to_lowercase()).copied().unwrap_or(99);
                        let t_min = parse_time_minutes(&m.time).unwrap_or(0);
                        (d_idx, t_min)
                    });

                    rows.push(RoundRow {
                        round_label: label.to_string(),
                        matches: stage_matches,
                        bye_teams: Vec::new(),
                    });
                }

                result.insert(div.id.clone(), rows);
            }
            SchedulingMode::IndividualRun => {
                let mut run_groups: HashMap<usize, Vec<&crate::model::ScheduleAssignment>> = HashMap::new();

                for assign in &div_assignments {
                    if let Activity::Run { run_number, .. } = &assign.activity {
                        run_groups.entry(*run_number).or_default().push(assign);
                    }
                }

                let mut rows = Vec::new();
                let mut sorted_runs: Vec<_> = run_groups.keys().collect();
                sorted_runs.sort_unstable();

                for &run_num in sorted_runs {
                    let assignments = run_groups.get(&run_num).unwrap();
                    let mut matches = Vec::new();
                    let mut round_teams = HashSet::new();

                    for a in assignments {
                        let slot = config.time_slots.iter().find(|s| s.id == a.time_slot_id);
                        let field = a.field_id.as_ref().and_then(|fid| config.fields.iter().find(|f| f.id == *fid));
                        
                        if let Activity::Run { team, .. } = &a.activity {
                            matches.push(RoundMatch {
                                team_a: team.clone(),
                                team_b: "".to_string(),
                                time: slot.map_or("".to_string(), |s| s.start_time.clone()),
                                day: slot.map_or("".to_string(), |s| s.day.clone()),
                                field_name: field.map_or("—".to_string(), |f| f.name.clone()),
                                is_final: false,
                            });
                            round_teams.insert(team.clone());
                        }
                    }

                    matches.sort_by_key(|m| {
                        let d_idx = day_order.get(&m.day.to_lowercase()).copied().unwrap_or(99);
                        let t_min = parse_time_minutes(&m.time).unwrap_or(0);
                        (d_idx, t_min)
                    });

                    let bye_teams: Vec<String> = div_teams.iter()
                        .filter(|t| !round_teams.contains(*t))
                        .cloned()
                        .collect();

                    rows.push(RoundRow {
                        round_label: format!("Run #{}", run_num),
                        matches,
                        bye_teams,
                    });
                }

                result.insert(div.id.clone(), rows);
            }
        }
    }

    result
}
