use crate::model::{Activity, Schedule, TournamentConfig};
use std::collections::HashMap;
use super::SolverParams;
use super::conflicts::{distinct_hard_conflicts, Conflict, ConflictKind};
use super::fast_evaluator::evaluate_schedule_conflicts;
use super::internal::InternalTournamentConfig;


pub fn get_occupied_slots(config: &TournamentConfig, start_slot_id: &str, duration_minutes: u32) -> Vec<String> {
    let mut occupied = Vec::new();
    if let Some(start_slot) = config.time_slots.iter().find(|s| s.id == start_slot_id) {
        let start_min = start_slot.start_minutes();
        let end_min = start_min + duration_minutes;
        
        for slot in &config.time_slots {
            if slot.day.to_lowercase() == start_slot.day.to_lowercase() {
                let slot_start = slot.start_minutes();
                let slot_end = slot_start + slot.duration_minutes();
                
                if start_min < slot_end && slot_start < end_min {
                    occupied.push(slot.id.clone());
                }
            }
        }
    } else {
        occupied.push(start_slot_id.to_string());
    }
    occupied
}

/// Scores a schedule. Delegates to the solver's evaluator
/// ([`super::fast_evaluator::evaluate_schedule_cost`]) so cost display and
/// optimization share one implementation.
pub fn evaluate_schedule_cost(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> (f64, f64) {
    super::fast_evaluator::evaluate_schedule_cost(config, schedule, params)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ConflictSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AssignmentConflict {
    pub severity: ConflictSeverity,
    pub message: String,
}

/// Strips the leading emoji icon (⚽ 🏆 🤖 💬) from an activity label for text logs.
fn clean_activity_label(activity: &Activity) -> String {
    let label = activity.label();
    if label.starts_with('⚽') || label.starts_with('🏆') || label.starts_with('🤖') || label.starts_with('💬') {
        label.chars().skip(2).collect::<String>()
    } else {
        label
    }
}

/// Formats one hard conflict into a UI severity + message. The `who` indices are
/// positions in `schedule.assignments`; the entity indices carried by the
/// [`ConflictKind`] are internal indices resolved against `internal`. Returns
/// `None` for soft penalties, which are not surfaced as conflicts.
fn format_hard_conflict(
    internal: &InternalTournamentConfig,
    config: &TournamentConfig,
    schedule: &Schedule,
    conflict: &Conflict,
) -> Option<(ConflictSeverity, String)> {
    let primary = *conflict.who.first()?;
    let assign = schedule.assignments.get(primary)?;
    let act = clean_activity_label(&assign.activity);

    let field_name = |fi: usize| {
        let id = &internal.fields[fi].id;
        config.fields.iter().find(|f| &f.id == id).map(|f| f.name.clone()).unwrap_or_else(|| id.clone())
    };
    let vol_name = |vi: usize| {
        let id = &internal.volunteers[vi].id;
        config.volunteers.iter().find(|v| &v.id == id).map(|v| v.name.clone()).unwrap_or_else(|| id.clone())
    };
    let slot_disp = |si: usize| {
        let id = &internal.slots[si].id;
        config.time_slots.iter().find(|s| &s.id == id).map(|s| format!("{} {}-{}", s.day, s.start_time, s.end_time)).unwrap_or_else(|| id.clone())
    };
    let team_name = |ti: usize| internal.teams[ti].name.clone();
    let div_name = || {
        let did = assign.activity.division_id();
        config.divisions.iter().find(|d| d.id == did).map(|d| d.name.clone()).unwrap_or_else(|| did.to_string())
    };

    let result = match conflict.kind {
        ConflictKind::SlotKindMismatch => {
            if matches!(assign.activity, Activity::Interview { .. }) {
                (ConflictSeverity::Error, format!("Slot Type Error: Interview '{}' assigned to a Competition time slot.", act))
            } else {
                (ConflictSeverity::Error, format!("Slot Type Error: Match/Run '{}' assigned to an Interview time slot.", act))
            }
        }
        ConflictKind::FieldUnsuitable { field_idx } => (ConflictSeverity::Error, format!("Field Suitability: Field '{}' is not suitable for '{}'.", field_name(field_idx), act)),
        ConflictKind::FieldMissing => (ConflictSeverity::Error, format!("Field Missing: No field/arena assigned for '{}'.", act)),
        ConflictKind::VolUnavailable { vol_idx, slot_idx } => (ConflictSeverity::Error, format!("Volunteer Availability: '{}' is not available during slot '{}'.", vol_name(vol_idx), slot_disp(slot_idx))),
        ConflictKind::VolUnqualified { vol_idx } => (ConflictSeverity::Error, format!("Volunteer Capability: '{}' lacks the required qualifications for '{}'.", vol_name(vol_idx), act)),
        ConflictKind::ConflictOfInterest { vol_idx, team_idx } => (ConflictSeverity::Error, format!("Conflict of Interest: '{}' has a conflict of interest with team '{}'.", vol_name(vol_idx), team_name(team_idx))),
        ConflictKind::UnderRostered { required, assigned } => (ConflictSeverity::Warning, format!("Under-Rostered: '{}' requires at least {} volunteer(s), but only {} assigned.", act, required, assigned)),
        ConflictKind::InterviewsDisabled => (ConflictSeverity::Error, format!("Interviews are disabled on the day '{}' is scheduled.", act)),
        ConflictKind::DurationExceedsDay => (ConflictSeverity::Error, format!("Duration Error: Activity '{}' exceeds the end of the day.", act)),
        ConflictKind::DailyShiftCapExceeded { vol_idx } => (ConflictSeverity::Error, format!("Volunteer Shift Cap: '{}' exceeds the daily shift cap.", vol_name(vol_idx))),
        ConflictKind::TeamDoubleBooked { team_idx } => (ConflictSeverity::Error, format!("Team Double-Booking: Team '{}' is scheduled for overlapping activities.", team_name(team_idx))),
        ConflictKind::FieldDoubleBooked { field_idx } => (ConflictSeverity::Error, format!("Field Double-Booking: Field/Arena '{}' is double-booked.", field_name(field_idx))),
        ConflictKind::VolDoubleBooked { vol_idx } => (ConflictSeverity::Warning, format!("Volunteer Double-Booking: '{}' is double-booked.", vol_name(vol_idx))),
        ConflictKind::StageOrder => (ConflictSeverity::Error, format!("Stage Order: In division '{}', a later-stage match is scheduled before an earlier-stage match.", div_name())),
        ConflictKind::StageOverlap => (ConflictSeverity::Error, format!("Stage Overlap: In division '{}', an earlier-stage match overlaps a later stage.", div_name())),
        ConflictKind::FieldVarietyStrict { team_idx, field_idx } => (ConflictSeverity::Error, format!("Field Variety: Team '{}' is assigned field '{}' more than once.", team_name(team_idx), field_name(field_idx))),
        // Soft penalties are not surfaced as conflicts.
        _ => return None,
    };
    Some(result)
}

/// Per-assignment hard conflicts for the schedule, keyed by assignment index.
/// Derived from the same engine as [`evaluate_schedule_cost`], so the conflicts
/// shown always match the cost the solver optimised.
pub fn get_assignment_conflicts(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> HashMap<usize, Vec<AssignmentConflict>> {
    let (internal, records, dropped) = evaluate_schedule_conflicts(config, schedule, params);
    let mut result: HashMap<usize, Vec<AssignmentConflict>> = HashMap::new();

    for conflict in distinct_hard_conflicts(&records) {
        if let Some((severity, message)) = format_hard_conflict(&internal, config, schedule, &conflict) {
            for &idx in &conflict.who {
                result.entry(idx).or_default().push(AssignmentConflict { severity, message: message.clone() });
            }
        }
    }

    for idx in dropped {
        if let Some(a) = schedule.assignments.get(idx) {
            result.entry(idx).or_default().push(AssignmentConflict {
                severity: ConflictSeverity::Error,
                message: format!("Internal Error: Division '{}' not found.", a.activity.division_id()),
            });
        }
    }

    result
}

/// Flat, sorted list of the schedule's distinct hard conflicts as human-readable
/// strings. Its length is the canonical hard-conflict count shown to the user,
/// and matches the hard cost the solver reports (both are zero together).
pub fn get_schedule_conflicts(
    config: &TournamentConfig,
    schedule: &Schedule,
    params: &SolverParams,
) -> Vec<String> {
    let (internal, records, dropped) = evaluate_schedule_conflicts(config, schedule, params);
    let mut conflicts: Vec<String> = Vec::new();

    for conflict in distinct_hard_conflicts(&records) {
        if let Some((_severity, message)) = format_hard_conflict(&internal, config, schedule, &conflict) {
            conflicts.push(message);
        }
    }

    for idx in dropped {
        if let Some(a) = schedule.assignments.get(idx) {
            conflicts.push(format!("Internal Error: Division '{}' not found.", a.activity.division_id()));
        }
    }

    conflicts.sort();
    conflicts
}


#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::scheduler::SolverParams;

    #[test]
    fn test_field_balance_overlapping_pools() {
        let mut config = TournamentConfig::default();
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div1".into()]) },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div2".into()]) },
            Field { id: "f3".into(), name: "Field 3".into(), kind: FieldKind::Competition, allowed_divisions: None }, // Unrestricted
        ];
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
            Division { 
                id: "div2".into(), name: "Div 2".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];

        // Isolate the field-balance dimension: the peak-period penalty also reacts
        // to how these toy schedules cluster in time, which would confound the check.
        let params = SolverParams { peak_period_weight: 0.0, ..SolverParams::default() };

        // Schedule 1: Field 3 is overloaded with matches
        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
            ]
        };

        // Schedule 2: Matches are balanced across fields
        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        assert!(cost1.1 > cost2.1);
    }

    #[test]
    fn test_field_balance_with_interviews() {
        let mut config = TournamentConfig::default();
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div1".into()]) },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: Some(vec!["div2".into()]) },
            Field { id: "f3".into(), name: "Field 3".into(), kind: FieldKind::Interview, allowed_divisions: None },
        ];
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
            Division { 
                id: "div2".into(), name: "Div 2".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];

        // Isolate the field-balance dimension: the peak-period penalty also reacts
        // to how these toy schedules cluster in time, which would confound the check.
        let params = SolverParams { peak_period_weight: 0.0, ..SolverParams::default() };

        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f3".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m1".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "m2".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div2".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        assert!(cost1.1 > cost2.1);
    }

    #[test]
    fn test_round_order_penalty() {
        let mut config = TournamentConfig::default();
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 1, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: false, finals_rounds: None, finals_duration_minutes: None,
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];

        let mut params = SolverParams::default();
        params.round_order_weight = 10.0;

        // Schedule 1: Round 1 at 09:00, Round 2 at 09:30 (Correct Order)
        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_2_c0_r1".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 1 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
            ]
        };

        // Schedule 2: Round 2 at 09:00, Round 1 at 09:30 (Inverted Order)
        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_2_c0_r1".into(), team_a: "c".into(), team_b: "d".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 1 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s2".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        assert_eq!(cost1.1, 0.0);
        assert!(cost2.1 >= 10.0);
    }

    #[test]
    fn test_rr_finals_overlap_hard_conflict() {
        let mut config = TournamentConfig::default();
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 2, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: true, finals_rounds: Some(FinalsRounds::Grand), finals_duration_minutes: Some(20),
                finals_third_place_playoff: false,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:20".into(), end_time: "09:40".into(), kind: FieldKind::Competition },
        ];
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.day_configs = vec![
            DayGenConfig { day: "Sat".into(), ..Default::default() },
        ];

        let mut params = SolverParams::default();
        params.round_order_weight = 1000.0;

        // Schedule 1: RR and Finals overlap at the same time (s1)
        let schedule1 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_gf".into(), team_a: "1st".into(), team_b: "2nd".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::GrandFinal }, 
                    time_slot_id: "s1".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        // Schedule 2: RR and Finals are sequential (RR at s1, Finals at s2)
        let schedule2 = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_m_1_c0_r0".into(), team_a: "a".into(), team_b: "b".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::RoundRobin { cycle: 0, round: 0 } }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_gf".into(), team_a: "1st".into(), team_b: "2nd".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::GrandFinal }, 
                    time_slot_id: "s2".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost1 = evaluate_schedule_cost(&config, &schedule1, &params);
        let cost2 = evaluate_schedule_cost(&config, &schedule2, &params);

        // Schedule 1 should have at least 1 hard conflict
        assert!(cost1.0 >= 1.0, "Overlap should be a hard conflict (got {})", cost1.0);
        // Schedule 2 should have 0 hard conflicts
        assert_eq!(cost2.0, 0.0, "Sequential RR and Finals should have no hard conflicts");
        
        // Also check conflicts report
        let conflicts1 = get_schedule_conflicts(&config, &schedule1, &params);
        assert!(conflicts1.iter().any(|c| c.contains("Stage Overlap")));
    }

    #[test]
    fn test_sf_3pl_order_hard_conflict() {
        let mut config = TournamentConfig::default();
        config.divisions = vec![
            Division { 
                id: "div1".into(), name: "Div 1".into(), mode: SchedulingMode::HeadToHead, 
                games_per_team: 2, volunteers_required: 0, duration_minutes: 20, 
                allowed_fields: None, interviews_enabled: false, 
                interview_volunteers_required: 0, interview_duration_minutes: 0,
                finals_enabled: true, finals_rounds: Some(FinalsRounds::Semis), finals_duration_minutes: Some(20),
                finals_third_place_playoff: true,
                color: None,
            },
        ];
        config.time_slots = vec![
            TimeSlot { id: "s1".into(), day: "Sat".into(), start_time: "09:00".into(), end_time: "09:20".into(), kind: FieldKind::Competition },
            TimeSlot { id: "s2".into(), day: "Sat".into(), start_time: "09:30".into(), end_time: "09:50".into(), kind: FieldKind::Competition },
        ];
        config.fields = vec![
            Field { id: "f1".into(), name: "Field 1".into(), kind: FieldKind::Competition, allowed_divisions: None },
            Field { id: "f2".into(), name: "Field 2".into(), kind: FieldKind::Competition, allowed_divisions: None },
        ];
        config.day_configs = vec![
            DayGenConfig { day: "Sat".into(), ..Default::default() },
        ];

        let mut params = SolverParams::default();
        params.round_order_weight = 1000.0;

        // Schedule: 3rd Place Playoff at s1 (09:00), Semi Final at s2 (09:30)
        // This is INVERTED order.
        let schedule = Schedule {
            assignments: vec![
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_3pl".into(), team_a: "L1".into(), team_b: "L2".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::ThirdPlace }, 
                    time_slot_id: "s1".into(), field_id: Some("f1".into()), volunteer_ids: vec![] 
                },
                ScheduleAssignment { 
                    activity: Activity::Match { id: "div1_sf_1".into(), team_a: "1st".into(), team_b: "4th".into(), division_id: "div1".into(), duration_minutes: 20, stage: crate::model::MatchStage::SemiFinal }, 
                    time_slot_id: "s2".into(), field_id: Some("f2".into()), volunteer_ids: vec![] 
                },
            ]
        };

        let cost = evaluate_schedule_cost(&config, &schedule, &params);
        let conflicts = get_schedule_conflicts(&config, &schedule, &params);

        assert!(cost.0 >= 10.0, "3PL before SF should be a hard conflict (got {})", cost.0);
        assert!(conflicts.iter().any(|c| c.contains("Stage Order")), "Conflict message should contain 'Stage Order'");
    }
}
